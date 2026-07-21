import {
  call as hostCall,
  callRaw as hostCallRaw,
  discover as hostDiscover,
  logMessage as hostLogMessage,
  registerStream as hostRegisterStream,
  sendDataChunk as hostSendDataChunk,
  tell as hostTell,
  unregisterStream as hostUnregisterStream,
} from 'actr:workload/host@0.2.0';

export type PayloadBytes = Uint8Array | ArrayBuffer | ArrayLike<number>;

// V2 `rpc-envelope` is `{request-id, route-key, payload}`. `method` is kept as
// a friendly alias for `route-key` so existing generated dispatchers that route
// on `envelope.method` keep working; `route-key` and `request-id` expose the
// raw V2 fields. The V1 `contentType` / `correlationId` / `deadlineMs` fields do
// not exist in V2 and are intentionally dropped (per the V2 WIT types interface).
export interface RpcEnvelope {
  method: string;
  routeKey: string;
  requestId: string;
  payload?: Uint8Array;
}

export interface Realm {
  realmId: number;
}

export interface ActrType {
  manufacturer: string;
  name: string;
  version: string;
}

export interface ActrId {
  realm: Realm;
  serialNumber: bigint | number;
  type: ActrType;
}

export interface MetadataEntry {
  key: string;
  value: string;
}

export interface DataChunk {
  streamId: string;
  sequence: bigint | number;
  payload: PayloadBytes;
  metadata?: MetadataEntry[];
  timestampMs?: bigint | number;
}

export type Dest = 'host' | 'workload' | { peer: ActrId };

// Per-invocation context threaded through every V2 export. Carries the
// identity triple that the 0.1.0 world exposed via getter imports, plus the
// `ctx-token` that routes outbound host imports to the host's per-invocation
// table. Under run_concurrent several dispatches may interleave on one
// instance; each carries its own `invocation-ctx`, so identity never
// cross-reads a shared slot.
export interface InvocationCtx {
  ctxToken: bigint;
  selfId: ActrId;
  callerId?: ActrId;
  requestId: string;
}

export interface ErrorEvent {
  source: ActrError;
  category:
    | 'handler-panic'
    | 'handler-error'
    | 'signaling-failure'
    | 'transport-failure'
    | 'data-chunk-delivery-uncertain';
  context: string;
  timestamp: { seconds: bigint; nanoseconds: number };
}

export interface PeerEvent {
  peer: ActrId;
  relayed?: boolean;
  status?: 'idle' | 'connecting' | 'connected' | 'recovering';
}

export interface CredentialEvent {
  newExpiry: { seconds: bigint; nanoseconds: number };
}

export interface BackpressureEvent {
  queueLen: bigint | number;
  threshold: bigint | number;
}

export type ActrError =
  | { tag: 'unavailable'; val: string }
  | { tag: 'connection-not-ready'; val: { retryAfterMs?: bigint } }
  | { tag: 'timed-out' }
  | { tag: 'not-found'; val: string }
  | { tag: 'permission-denied'; val: string }
  | { tag: 'invalid-argument'; val: string }
  | { tag: 'unknown-route'; val: string }
  | {
      tag: 'dependency-not-found';
      val: { serviceName: string; message: string };
    }
  | { tag: 'decode-failure'; val: string }
  | { tag: 'not-implemented'; val: string }
  | { tag: 'internal'; val: string };

type WitActrId = Omit<ActrId, 'serialNumber'> & {
  serialNumber: bigint;
};

type WitDest =
  | { tag: 'host' }
  | { tag: 'workload' }
  | { tag: 'peer'; val: WitActrId };

type WitPayloadType = { tag: PayloadType };

type WitDataChunk = Omit<
  DataChunk,
  'sequence' | 'payload' | 'metadata' | 'timestampMs'
> & {
  sequence: bigint;
  payload: Uint8Array;
  metadata: MetadataEntry[];
  timestampMs?: bigint;
};

export const PayloadType = {
  RpcReliable: 'rpc-reliable',
  RpcSignal: 'rpc-signal',
  StreamReliable: 'stream-reliable',
  StreamLatencyFirst: 'stream-latency-first',
  MediaRtp: 'media-rtp',
} as const;

export type PayloadType = (typeof PayloadType)[keyof typeof PayloadType];

export type StreamCallback = (
  chunk: DataChunk,
  sender: ActrId,
) => void | Promise<void>;

// `ctx` is optional on every hook: the host always threads an `invocation-ctx`
// into the export, but callers may ignore it. Host-import wrappers below read
// the `ctx-token` from an explicitly-passed `ctx` (the concurrency-safe path,
// since StarlingMonkey has no `AsyncLocalStorage`), with an optional
// `AsyncLocalStorage` store as a Node-side convenience. A host import without
// either source fails closed instead of guessing a token.
export interface Workload {
  dispatch(
    envelope: RpcEnvelope,
    ctx?: InvocationCtx,
  ): Uint8Array | ArrayBuffer | Promise<Uint8Array | ArrayBuffer>;
  onStart?(ctx?: InvocationCtx): void | Promise<void>;
  onReady?(ctx?: InvocationCtx): void | Promise<void>;
  onStop?(ctx?: InvocationCtx): void | Promise<void>;
  onError?(event: ErrorEvent, ctx?: InvocationCtx): void | Promise<void>;
  onDataChunk?(
    chunk: DataChunk,
    sender: ActrId,
    ctx?: InvocationCtx,
  ): void | Promise<void>;
}

export function defineWorkload(workload: Workload): Workload {
  return workload;
}

const streamCallbacks = new Map<string, StreamCallback>();

// Optional `AsyncLocalStorage` for the Node (vitest / non-component) path. The
// StarlingMonkey wasm guest has no `node:async_hooks`, so in a real component
// the dynamic import rejects and `invocationStorage` stays `undefined`; callers
// then thread `ctx` explicitly. Top-level `await` keeps this lazy and avoids a
// static `node:` import that would break componentization.
let invocationStorage:
  | import('node:async_hooks').AsyncLocalStorage<InvocationCtx>
  | undefined;
try {
  const { AsyncLocalStorage } = await import('node:async_hooks');
  invocationStorage = new AsyncLocalStorage<InvocationCtx>();
} catch {
  // StarlingMonkey wasm guest: no node:async_hooks. ctx is threaded explicitly.
}

/**
 * Enter an invocation context for the duration of `fn`. Used by the
 * componentize shim at each export entry so host-import wrappers can read the
 * current `ctx-token` without an explicit argument on the Node side. In the
 * wasm guest this is a pass-through (no `AsyncLocalStorage`); callers pass
 * `ctx` to the wrappers directly.
 */
export function withInvocationCtx<T>(
  ctx: InvocationCtx,
  fn: () => T | Promise<T>,
): T | Promise<T> {
  if (invocationStorage) {
    return invocationStorage.run(ctx, fn);
  }
  return fn();
}

/**
 * Read the currently-active invocation context, if any. Available on the Node
 * side (where `AsyncLocalStorage` is initialized); always `undefined` inside
 * the wasm guest.
 */
export function getCurrentInvocationCtx(): InvocationCtx | undefined {
  return invocationStorage?.getStore();
}

function resolveCtxToken(ctx?: InvocationCtx): bigint {
  if (ctx?.ctxToken !== undefined) {
    return ctx.ctxToken;
  }
  const stored = invocationStorage?.getStore();
  if (stored?.ctxToken !== undefined) {
    return stored.ctxToken;
  }
  throw new Error(
    'ACTR host import requires an InvocationCtx; pass the ctx received by the workload export',
  );
}

export function toUint8Array(value: PayloadBytes): Uint8Array {
  if (value instanceof Uint8Array) {
    return value;
  }
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  return Uint8Array.from(value);
}

function toWitActrId(id: ActrId): WitActrId {
  return {
    realm: id.realm,
    serialNumber: BigInt(id.serialNumber),
    type: id.type,
  };
}

function toWitDest(dest: Dest): WitDest {
  if (dest === 'host') {
    return { tag: 'host' };
  }
  if (dest === 'workload') {
    return { tag: 'workload' };
  }
  return {
    tag: 'peer',
    val: toWitActrId(dest.peer),
  };
}

function toWitDataChunk(chunk: DataChunk): WitDataChunk {
  return {
    streamId: chunk.streamId,
    sequence: BigInt(chunk.sequence),
    payload: toUint8Array(chunk.payload),
    metadata: chunk.metadata ?? [],
    timestampMs:
      chunk.timestampMs === undefined ? undefined : BigInt(chunk.timestampMs),
  };
}

function fromWitActrId(id: WitActrId): ActrId {
  return {
    realm: id.realm,
    serialNumber: id.serialNumber,
    type: id.type,
  };
}

// ── Host-import wrappers ──────────────────────────────────────────────────
//
// Each wrapper resolves the current `ctx-token` (explicit `ctx` arg first,
// then the `AsyncLocalStorage` store) and forwards to the V2 host import. It
// fails closed when neither source exists so a missing context can never route
// through another invocation's token. V2 imports surface WIT `result<T, E>`
// as a resolved value on success and a thrown JS `Error` on the `err` arm, so
// the wrappers simply await and let errors propagate.

export async function call(
  target: Dest,
  routeKey: string,
  payload: PayloadBytes,
  ctx?: InvocationCtx,
): Promise<Uint8Array> {
  return hostCall(
    resolveCtxToken(ctx),
    toWitDest(target),
    routeKey,
    toUint8Array(payload),
  );
}

export async function tell(
  target: Dest,
  routeKey: string,
  payload: PayloadBytes,
  ctx?: InvocationCtx,
): Promise<void> {
  await hostTell(
    resolveCtxToken(ctx),
    toWitDest(target),
    routeKey,
    toUint8Array(payload),
  );
}

export async function callRaw(
  target: ActrId,
  routeKey: string,
  payload: PayloadBytes,
  ctx?: InvocationCtx,
): Promise<Uint8Array> {
  return hostCallRaw(
    resolveCtxToken(ctx),
    toWitActrId(target),
    routeKey,
    toUint8Array(payload),
  );
}

export async function discover(
  targetType: ActrType,
  ctx?: InvocationCtx,
): Promise<ActrId> {
  const id = await hostDiscover(resolveCtxToken(ctx), targetType);
  return fromWitActrId(id);
}

export async function registerStream(
  streamId: string,
  callback: StreamCallback,
  ctx?: InvocationCtx,
): Promise<void> {
  // Register host-side first so the runner queues any DataChunk delivery behind
  // this invocation; installing the guest callback immediately after the
  // awaited import cannot miss a delivery, and avoids retaining a guest
  // callback when host registration fails.
  await hostRegisterStream(resolveCtxToken(ctx), streamId);
  streamCallbacks.set(streamId, callback);
}

export async function unregisterStream(
  streamId: string,
  ctx?: InvocationCtx,
): Promise<void> {
  await hostUnregisterStream(resolveCtxToken(ctx), streamId);
  streamCallbacks.delete(streamId);
}

export async function sendDataChunk(
  target: Dest,
  chunk: DataChunk,
  payloadType: PayloadType,
  ctx?: InvocationCtx,
): Promise<void> {
  await hostSendDataChunk(
    resolveCtxToken(ctx),
    toWitDest(target),
    toWitDataChunk(chunk),
    { tag: payloadType } satisfies WitPayloadType,
  );
}

export async function logMessage(
  level: string,
  message: string,
  ctx?: InvocationCtx,
): Promise<void> {
  await hostLogMessage(resolveCtxToken(ctx), level, message);
}

export async function __dispatchDataChunk(
  chunk: DataChunk,
  sender: ActrId,
): Promise<void> {
  const callback = streamCallbacks.get(chunk.streamId);
  if (!callback) {
    throw new Error(`No stream callback registered for ${chunk.streamId}`);
  }
  await callback(chunk, sender);
}
