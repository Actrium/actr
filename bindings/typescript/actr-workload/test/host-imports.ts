type ActrId = {
  realm: { realmId: number };
  serialNumber: bigint;
  type: {
    manufacturer: string;
    name: string;
    version: string;
  };
};

type ActrType = ActrId['type'];

type Dest =
  { tag: 'host' } | { tag: 'workload' } | { tag: 'peer'; val: ActrId };

type DataChunk = {
  streamId: string;
  sequence: bigint;
  payload: Uint8Array;
  metadata: Array<{ key: string; value: string }>;
  timestampMs?: bigint;
};

type PayloadType = {
  tag:
    | 'rpc-reliable'
    | 'rpc-signal'
    | 'stream-reliable'
    | 'stream-latency-first'
    | 'media-rtp';
};

export type SendDataChunkCall = {
  target: Dest;
  chunk: DataChunk;
  payloadType: PayloadType;
};

export const hostCalls = {
  ctxTokens: [] as bigint[],
  operations: [] as string[],
  registerStream: [] as string[],
  unregisterStream: [] as string[],
  sendDataChunk: [] as SendDataChunkCall[],
};

export function resetHostCalls(): void {
  hostCalls.ctxTokens.length = 0;
  hostCalls.operations.length = 0;
  hostCalls.registerStream.length = 0;
  hostCalls.unregisterStream.length = 0;
  hostCalls.sendDataChunk.length = 0;
}

// V2 host imports take a `ctx-token: bigint` first parameter and are async.
// Record the token separately from the user-facing payload so tests can assert
// explicit V2 context propagation without changing the existing call fixtures.

function record(ctxToken: bigint, operation: string): void {
  hostCalls.ctxTokens.push(ctxToken);
  hostCalls.operations.push(operation);
}

export async function call(
  ctxToken: bigint,
  _target: Dest,
  _routeKey: string,
  payload: Uint8Array,
): Promise<Uint8Array> {
  record(ctxToken, 'call');
  return payload;
}

export async function tell(
  ctxToken: bigint,
  _target: Dest,
  _routeKey: string,
  _payload: Uint8Array,
): Promise<void> {
  record(ctxToken, 'tell');
}

export async function callRaw(
  ctxToken: bigint,
  _target: ActrId,
  _routeKey: string,
  payload: Uint8Array,
): Promise<Uint8Array> {
  record(ctxToken, 'callRaw');
  return payload;
}

export async function discover(
  ctxToken: bigint,
  targetType: ActrType,
): Promise<ActrId> {
  record(ctxToken, 'discover');
  return {
    realm: { realmId: 1 },
    serialNumber: 1n,
    type: targetType,
  };
}

export async function registerStream(
  ctxToken: bigint,
  streamId: string,
): Promise<void> {
  record(ctxToken, 'registerStream');
  hostCalls.registerStream.push(streamId);
}

export async function unregisterStream(
  ctxToken: bigint,
  streamId: string,
): Promise<void> {
  record(ctxToken, 'unregisterStream');
  hostCalls.unregisterStream.push(streamId);
}

export async function sendDataChunk(
  ctxToken: bigint,
  target: Dest,
  chunk: DataChunk,
  payloadType: PayloadType,
): Promise<void> {
  record(ctxToken, 'sendDataChunk');
  hostCalls.sendDataChunk.push({ target, chunk, payloadType });
}

export async function logMessage(
  ctxToken: bigint,
  _level: string,
  _message: string,
): Promise<void> {
  record(ctxToken, 'logMessage');
}
