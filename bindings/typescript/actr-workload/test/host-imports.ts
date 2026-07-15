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
  registerStream: [] as string[],
  unregisterStream: [] as string[],
  sendDataChunk: [] as SendDataChunkCall[],
};

export function resetHostCalls(): void {
  hostCalls.registerStream.length = 0;
  hostCalls.unregisterStream.length = 0;
  hostCalls.sendDataChunk.length = 0;
}

// V2 host imports take a `ctx-token: bigint` first parameter and are async.
// The test runtime resolves the token to `0n` when no invocation context is
// active, so the mocks ignore it and record only the user-facing payload —
// keeping the existing assertions stable.

export async function registerStream(_ctxToken: bigint, streamId: string): Promise<void> {
  hostCalls.registerStream.push(streamId);
}

export async function unregisterStream(
  _ctxToken: bigint,
  streamId: string,
): Promise<void> {
  hostCalls.unregisterStream.push(streamId);
}

export async function sendDataChunk(
  _ctxToken: bigint,
  target: Dest,
  chunk: DataChunk,
  payloadType: PayloadType,
): Promise<void> {
  hostCalls.sendDataChunk.push({ target, chunk, payloadType });
}
