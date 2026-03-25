/**
 * Fast Path Forwarder - 
 *
 *  WebRTC DataChannel  Service Worker WASM
 */

import { ServiceWorkerBridge } from './sw-bridge';

export interface FastPathData {
  streamId: string;
  data: ArrayBuffer;
  timestamp: number;
}

/**
 * Fast Path 
 */
export class FastPathForwarder {
  private swBridge: ServiceWorkerBridge;
  private batchQueue: FastPathData[] = [];
  private batchTimer: number | null = null;
  private batchSize = 10; // 
  private batchTimeoutMs = 5; // （）

  constructor(swBridge: ServiceWorkerBridge) {
    this.swBridge = swBridge;
  }

  /**
   *  Fast Path  Service Worker
   *
   *  Transferable ArrayBuffer 
   */
  forward(streamId: string, data: ArrayBuffer): void {
    const fastPathData: FastPathData = {
      streamId,
      data,
      timestamp: Date.now(),
    };

    // （）
    this.forwardImmediate(fastPathData);
  }

  /**
   *  Fast Path 
   *
   * ， PostMessage 
   */
  forwardBatch(streamId: string, data: ArrayBuffer): void {
    this.batchQueue.push({
      streamId,
      data,
      timestamp: Date.now(),
    });

    // ，
    if (this.batchQueue.length >= this.batchSize) {
      this.flushBatch();
    } else if (this.batchTimer === null) {
      // 
      this.batchTimer = window.setTimeout(() => {
        this.flushBatch();
      }, this.batchTimeoutMs);
    }
  }

  /**
   * 
   */
  private forwardImmediate(fastPathData: FastPathData): void {
    const view = new Uint8Array(fastPathData.data);
    this.swBridge.sendToSW(
      {
        type: 'fast_path_data',
        payload: {
          streamId: fastPathData.streamId,
          data: view,
          timestamp: fastPathData.timestamp,
        },
      },
      [view.buffer as ArrayBuffer] // Transferable - 
    );
  }

  /**
   * 
   */
  private flushBatch(): void {
    if (this.batchQueue.length === 0) {
      return;
    }

    // 
    if (this.batchTimer !== null) {
      window.clearTimeout(this.batchTimer);
      this.batchTimer = null;
    }

    //  transferables
    const batchPayload = this.batchQueue.map((item) => ({
      streamId: item.streamId,
      data: new Uint8Array(item.data),
      timestamp: item.timestamp,
    }));
    const transferables: ArrayBuffer[] = batchPayload.map(
      (item) => item.data.buffer as ArrayBuffer
    );

    // 
    this.swBridge.sendToSW(
      {
        type: 'fast_path_data',
        payload: {
          batch: batchPayload,
        },
      },
      transferables
    );

    // 
    this.batchQueue = [];
  }

  /**
   * 
   */
  setBatchParams(size: number, timeoutMs: number): void {
    this.batchSize = size;
    this.batchTimeoutMs = timeoutMs;
  }

  /**
   * 
   */
  getMetrics(): {
    queueLength: number;
    hasPendingBatch: boolean;
  } {
    return {
      queueLength: this.batchQueue.length,
      hasPendingBatch: this.batchTimer !== null,
    };
  }

  /**
   * 
   */
  dispose(): void {
    if (this.batchTimer !== null) {
      window.clearTimeout(this.batchTimer);
      this.batchTimer = null;
    }
    this.batchQueue = [];
  }
}
