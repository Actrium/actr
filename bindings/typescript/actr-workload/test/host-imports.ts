type ActrId = {
  realm: { realmId: number };
  serialNumber: bigint;
  type: {
    manufacturer: string;
    name: string;
    version: string;
  };
};

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
  registerStream: [] as string[],
  unregisterStream: [] as string[],
  sendDataChunk: [] as SendDataChunkCall[],
};

export function resetHostCalls(): void {
  hostCalls.ctxTokens.length = 0;
  hostCalls.registerStream.length = 0;
  hostCalls.unregisterStream.length = 0;
  hostCalls.sendDataChunk.length = 0;
}

// V2 host imports take a `ctx-token: bigint` first parameter and are async.
// Record the token separately from the user-facing payload so tests can assert
// explicit V2 context propagation without changing the existing call fixtures.

export async function registerStream(
  ctxToken: bigint,
  streamId: string,
): Promise<void> {
  hostCalls.ctxTokens.push(ctxToken);
  hostCalls.registerStream.push(streamId);
}

export async function unregisterStream(
  ctxToken: bigint,
  streamId: string,
): Promise<void> {
  hostCalls.ctxTokens.push(ctxToken);
  hostCalls.unregisterStream.push(streamId);
}

export async function sendDataChunk(
  ctxToken: bigint,
  target: Dest,
  chunk: DataChunk,
  payloadType: PayloadType,
): Promise<void> {
  hostCalls.ctxTokens.push(ctxToken);
  hostCalls.sendDataChunk.push({ target, chunk, payloadType });
}
