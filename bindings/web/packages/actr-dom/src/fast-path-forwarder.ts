/**
 * Fast Path Forwarder - 快车道数据转发
 *
 * 负责将 WebRTC DataChannel 接收的数据零拷贝转发到 Service Worker WASM
 */

import { ServiceWorkerBridge } from './sw-bridge';

export interface FastPathData {
  streamId: string;
  data: ArrayBuffer;
  timestamp: number;
}

/**
 * Fast Path 数据转发器
 */
export class FastPathForwarder {
  private swBridge: ServiceWorkerBridge;
  private batchQueue: FastPathData[] = [];
  private batchTimer: number | null = null;
  private batchSize = 10; // 批量转发阈值
  private batchTimeoutMs = 5; // 批量超时（毫秒）

  constructor(swBridge: ServiceWorkerBridge) {
    this.swBridge = swBridge;
  }

  /**
   * 转发 Fast Path 数据到 Service Worker
   *
   * 使用 Transferable ArrayBuffer 实现零拷贝
   */
  forward(streamId: string, data: ArrayBuffer): void {
    const fastPathData: FastPathData = {
      streamId,
      data,
      timestamp: Date.now(),
    };

    // 立即转发（单条数据）
    this.forwardImmediate(fastPathData);
  }

  /**
   * 批量转发 Fast Path 数据
   *
   * 用于高吞吐场景，减少 PostMessage 次数
   */
  forwardBatch(streamId: string, data: ArrayBuffer): void {
    this.batchQueue.push({
      streamId,
      data,
      timestamp: Date.now(),
    });

    // 如果达到批量阈值，立即发送
    if (this.batchQueue.length >= this.batchSize) {
      this.flushBatch();
    } else if (this.batchTimer === null) {
      // 设置超时定时器
      this.batchTimer = window.setTimeout(() => {
        this.flushBatch();
      }, this.batchTimeoutMs);
    }
  }

  /**
   * 立即转发单条数据
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
      [view.buffer as ArrayBuffer] // Transferable - 零拷贝
    );
  }

  /**
   * 刷新批量队列
   */
  private flushBatch(): void {
    if (this.batchQueue.length === 0) {
      return;
    }

    // 清除定时器
    if (this.batchTimer !== null) {
      window.clearTimeout(this.batchTimer);
      this.batchTimer = null;
    }

    // 准备 transferables
    const batchPayload = this.batchQueue.map((item) => ({
      streamId: item.streamId,
      data: new Uint8Array(item.data),
      timestamp: item.timestamp,
    }));
    const transferables: ArrayBuffer[] = batchPayload.map(
      (item) => item.data.buffer as ArrayBuffer
    );

    // 发送批量数据
    this.swBridge.sendToSW(
      {
        type: 'fast_path_data',
        payload: {
          batch: batchPayload,
        },
      },
      transferables
    );

    // 清空队列
    this.batchQueue = [];
  }

  /**
   * 设置批量参数
   */
  setBatchParams(size: number, timeoutMs: number): void {
    this.batchSize = size;
    this.batchTimeoutMs = timeoutMs;
  }

  /**
   * 获取性能指标
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
   * 清理资源
   */
  dispose(): void {
    if (this.batchTimer !== null) {
      window.clearTimeout(this.batchTimer);
      this.batchTimer = null;
    }
    this.batchQueue = [];
  }
}
