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

type DataStream = {
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

export type SendDataStreamCall = {
  target: Dest;
  chunk: DataStream;
  payloadType: PayloadType;
};

export const hostCalls = {
  registerStream: [] as string[],
  unregisterStream: [] as string[],
  sendDataStream: [] as SendDataStreamCall[],
};

export function resetHostCalls(): void {
  hostCalls.registerStream.length = 0;
  hostCalls.unregisterStream.length = 0;
  hostCalls.sendDataStream.length = 0;
}

export function registerStream(streamId: string): void {
  hostCalls.registerStream.push(streamId);
}

export function unregisterStream(streamId: string): void {
  hostCalls.unregisterStream.push(streamId);
}

export function sendDataStream(
  target: Dest,
  chunk: DataStream,
  payloadType: PayloadType,
): void {
  hostCalls.sendDataStream.push({ target, chunk, payloadType });
}
