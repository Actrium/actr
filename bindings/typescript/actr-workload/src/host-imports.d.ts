declare module 'actr:workload/host@0.1.0' {
  type Realm = {
    realmId: number;
  };

  type ActrType = {
    manufacturer: string;
    name: string;
    version: string;
  };

  type ActrId = {
    realm: Realm;
    serialNumber: bigint;
    type: ActrType;
  };

  type MetadataEntry = {
    key: string;
    value: string;
  };

  type Dest =
    | { tag: 'shell' }
    | { tag: 'local' }
    | { tag: 'actor'; val: ActrId };

  type DataStream = {
    streamId: string;
    sequence: bigint;
    payload: Uint8Array;
    metadata: MetadataEntry[];
    timestampMs?: bigint;
  };

  type PayloadType =
    | { tag: 'rpc-reliable' }
    | { tag: 'rpc-signal' }
    | { tag: 'stream-reliable' }
    | { tag: 'stream-latency-first' }
    | { tag: 'media-rtp' };

  export function registerStream(streamId: string): void;
  export function unregisterStream(streamId: string): void;
  export function sendDataStream(
    target: Dest,
    chunk: DataStream,
    payloadType: PayloadType,
  ): void;
}
