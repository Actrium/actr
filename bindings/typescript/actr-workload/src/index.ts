export type PayloadBytes = Uint8Array | ArrayBuffer | ArrayLike<number>;

export interface RpcEnvelope {
  method: string;
  payload?: Uint8Array;
  contentType?: string;
  correlationId?: string;
  deadlineMs?: bigint;
}

export interface Workload {
  dispatch(
    envelope: RpcEnvelope,
  ): Uint8Array | ArrayBuffer | Promise<Uint8Array | ArrayBuffer>;
  onStart?(): void | Promise<void>;
  onReady?(): void | Promise<void>;
  onStop?(): void | Promise<void>;
  onError?(message: string): void | Promise<void>;
}

export function defineWorkload(workload: Workload): Workload {
  return workload;
}
