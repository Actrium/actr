declare module 'actr:workload/host@0.2.0' {
  // V2 host imports are real WIT `async func`. Every import takes a
  // `ctx-token: u64` first parameter (surfaced to JS as a `bigint`) that keys
  // the host's per-invocation table so it can recover the correct bridge even
  // when multiple dispatches interleave on one instance.
  //
  // WIT `result<T, E>` is surfaced by the jco/StarlingMonkey guest runtime as a
  // thrown JS `Error` for the `err` arm and a resolved value for the `ok` arm
  // (matching the generated `jco guest-types` definitions, which type these as
  // `Promise<T>` rather than a tagged union). The thrown error carries the WIT
  // `actr-error` variant; callers may catch and inspect `error.actrError`.

  export function call(
    ctxToken: bigint,
    target: Dest,
    routeKey: string,
    payload: Uint8Array,
  ): Promise<Uint8Array>;

  export function tell(
    ctxToken: bigint,
    target: Dest,
    routeKey: string,
    payload: Uint8Array,
  ): Promise<void>;

  export function callRaw(
    ctxToken: bigint,
    target: ActrId,
    routeKey: string,
    payload: Uint8Array,
  ): Promise<Uint8Array>;

  export function discover(
    ctxToken: bigint,
    targetType: ActrType,
  ): Promise<ActrId>;

  export function registerStream(
    ctxToken: bigint,
    streamId: string,
  ): Promise<void>;

  export function unregisterStream(
    ctxToken: bigint,
    streamId: string,
  ): Promise<void>;

  export function sendDataChunk(
    ctxToken: bigint,
    target: Dest,
    chunk: DataChunk,
    payloadType: PayloadType,
  ): Promise<void>;

  export function logMessage(
    ctxToken: bigint,
    level: string,
    message: string,
  ): Promise<void>;

  export type Realm = {
    realmId: number;
  };

  export type ActrType = {
    manufacturer: string;
    name: string;
    version: string;
  };

  export type ActrId = {
    realm: Realm;
    serialNumber: bigint;
    type: ActrType;
  };

  export type MetadataEntry = {
    key: string;
    value: string;
  };

  export type Dest =
    { tag: 'host' } | { tag: 'workload' } | { tag: 'peer'; val: ActrId };

  export type DataChunk = {
    streamId: string;
    sequence: bigint;
    payload: Uint8Array;
    metadata: MetadataEntry[];
    timestampMs?: bigint;
  };

  export type PayloadType =
    | { tag: 'rpc-reliable' }
    | { tag: 'rpc-signal' }
    | { tag: 'stream-reliable' }
    | { tag: 'stream-latency-first' }
    | { tag: 'media-rtp' };

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
}
