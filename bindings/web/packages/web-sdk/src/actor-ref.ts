/**
 * ActorRef - Actor 引用
 *
 * 基于新架构的实现：通过 @actr/dom 与 Service Worker WASM 交互
 */

import {
  ActrDomRuntime,
  RpcResponsePayload,
  SubscriptionDataPayload,
  WebRtcEventPayload,
} from '@actr/dom';
import type { SubscriptionCallback, UnsubscribeFn } from './types';

/**
 * RPC 请求
 */
export interface RpcRequest<T = unknown> {
  service: string;
  method: string;
  params: T;
  timeout?: number;
}

/**
 * Actor 引用
 *
 * 提供三个核心 API：
 * - call(): 请求-响应（State Path，30-40ms）
 * - subscribe(): 订阅数据流（Fast Path，6-13ms）
 * - on(): 系统事件监听（<5ms）
 */
export class ActorRef {
  private domRuntime: ActrDomRuntime;
  private requestId = 0;

  /** 待处理的 RPC 请求 */
  private pendingRequests = new Map<
    string,
    {
      resolve: (value: Uint8Array) => void;
      reject: (error: Error) => void;
      timeout: number;
    }
  >();

  /** 订阅管理 */
  private subscriptions = new Map<string, Set<SubscriptionCallback<any>>>();

  /** 事件监听 */
  private eventListeners = new Map<string, Set<(...args: any[]) => void>>();

  constructor(domRuntime: ActrDomRuntime) {
    this.domRuntime = domRuntime;
    this.setupMessageHandlers();
  }

  /**
   * 设置来自 Service Worker 的消息处理器
   */
  private setupMessageHandlers(): void {
    const bridge = this.domRuntime.getSWBridge();

    bridge.onMessage((message) => {
      switch (message.type) {
        case 'control_response':
          this.handleRpcResponse(message.payload);
          break;

        case 'subscription_data':
          this.handleSubscriptionData(message.payload);
          break;

        case 'webrtc_event':
          this.handleWebRtcEvent(message.payload);
          break;

        case 'webrtc_command':
          // Handled by WebRTC Coordinator, ignore here
          break;

        default:
          console.warn(`[ActorRef] Unknown message type: ${message.type}`);
      }
    });
  }

  /**
   * 处理 RPC 响应
   */
  private handleRpcResponse(payload: RpcResponsePayload): void {
    const { request_id, data, error } = payload;

    const pending = this.pendingRequests.get(request_id);
    if (!pending) {
      console.warn(`[ActorRef] No pending request for ${request_id}`);
      return;
    }

    clearTimeout(pending.timeout);
    this.pendingRequests.delete(request_id);

    if (error) {
      pending.reject(new Error(`RPC error [${error.code}]: ${error.message}`));
    } else {
      pending.resolve(data ?? new Uint8Array());
    }
  }

  /**
   * 处理订阅数据
   */
  private handleSubscriptionData(payload: SubscriptionDataPayload): void {
    const { topic, data } = payload;

    const callbacks = this.subscriptions.get(topic);
    if (!callbacks || callbacks.size === 0) {
      return;
    }

    for (const callback of callbacks) {
      try {
        callback(data);
      } catch (error) {
        console.error(`[ActorRef] Subscription callback error:`, error);
      }
    }
  }

  /**
   * 处理 WebRTC 事件
   */
  private handleWebRtcEvent(payload: WebRtcEventPayload): void {
    const { eventType, data } = payload;

    switch (eventType) {
      case 'connection_state_changed':
        this.emit('stateChange', data.state);
        this.emit('connectionStateChanged', {
          state: data.state,
          peerId: data.peerId,
        });
        // Fail-fast: reject all pending RPCs when connection is irrecoverable
        if (data.state === 'failed' || data.state === 'closed') {
          this.rejectAllPending(
            `WebRTC connection ${data.state} (peer: ${data.peerId})`
          );
        }
        break;

      case 'datachannel_open':
        this.emit('peerConnected', { peerId: data.peerId });
        break;

      case 'datachannel_close':
        this.emit('peerDisconnected', { peerId: data.peerId });
        break;

      default:
        console.log(`[ActorRef] WebRTC event: ${eventType}`, data);
    }
  }

  /**
   * call() - 请求-响应（State Path）
   *
   * @param service - 目标服务名称
   * @param method - 方法名称
   * @param params - 请求参数
   * @param timeout - 超时时间（毫秒），默认 30000ms
   */
  async call<TReq = unknown, TRes = unknown>(
    service: string,
    method: string,
    params: TReq,
    timeout: number = 30000
  ): Promise<TRes> {
    const requestId = `req_${++this.requestId}_${Date.now()}`;

    const promise = new Promise<TRes>((resolve, reject) => {
      const timeoutHandle = window.setTimeout(() => {
        this.pendingRequests.delete(requestId);
        reject(new Error(`RPC timeout after ${timeout}ms`));
      }, timeout);

      this.pendingRequests.set(requestId, {
        resolve: (data: Uint8Array) => resolve(data as unknown as TRes),
        reject,
        timeout: timeoutHandle,
      });
    });

    const request: RpcRequest<TReq> = {
      service,
      method,
      params,
      timeout,
    };

    this.domRuntime.getSWBridge().sendToSW({
      type: 'control',
      payload: {
        action: 'rpc_call',
        request_id: requestId,
        request,
      },
    });

    return promise;
  }

  /**
   * callRaw() - 发送已编码的 RPC payload（Raw bytes）。
   * TODO: Prefer typed call() once Phase 1 RPC path is fully wired.
   */
  async callRaw(
    routeKey: string,
    payload: Uint8Array,
    timeout: number = 30000
  ): Promise<Uint8Array> {
    // TODO: Prefer typed call() once Phase 1 RPC path is fully wired.
    const requestId = `req_${++this.requestId}_${Date.now()}`;

    const promise = new Promise<Uint8Array>((resolve, reject) => {
      const timeoutHandle = window.setTimeout(() => {
        this.pendingRequests.delete(requestId);
        reject(new Error(`RPC timeout after ${timeout}ms`));
      }, timeout);

      this.pendingRequests.set(requestId, {
        resolve,
        reject,
        timeout: timeoutHandle,
      });
    });

    this.domRuntime.getSWBridge().sendToSW({
      type: 'control',
      payload: {
        action: 'rpc_call',
        request_id: requestId,
        request: {
          route_key: routeKey,
          payload,
          timeout,
        },
      },
    });

    return promise;
  }

  /**
   * subscribe() - 订阅数据流（Fast Path）
   *
   * @param topic - 订阅主题
   * @param callback - 数据回调函数
   */
  async subscribe<T = unknown>(
    topic: string,
    callback: SubscriptionCallback<T>
  ): Promise<UnsubscribeFn> {
    let callbacks = this.subscriptions.get(topic);
    if (!callbacks) {
      callbacks = new Set();
      this.subscriptions.set(topic, callbacks);

      this.domRuntime.getSWBridge().sendToSW({
        type: 'control',
        payload: {
          action: 'subscribe',
          topic,
        },
      });
    }

    callbacks.add(callback);

    return () => {
      const currentCallbacks = this.subscriptions.get(topic);
      if (!currentCallbacks) return;

      currentCallbacks.delete(callback);

      if (currentCallbacks.size === 0) {
        this.subscriptions.delete(topic);

        this.domRuntime.getSWBridge().sendToSW({
          type: 'control',
          payload: {
            action: 'unsubscribe',
            topic,
          },
        });
      }
    };
  }

  /**
   * on() - 系统事件监听
   *
   * @param event - 事件类型
   * @param callback - 事件回调函数
   */
  on(event: string, callback: (...args: unknown[]) => void): UnsubscribeFn {
    let callbacks = this.eventListeners.get(event);
    if (!callbacks) {
      callbacks = new Set();
      this.eventListeners.set(event, callbacks);
    }

    callbacks.add(callback);

    return () => {
      const currentCallbacks = this.eventListeners.get(event);
      if (!currentCallbacks) return;

      currentCallbacks.delete(callback);

      if (currentCallbacks.size === 0) {
        this.eventListeners.delete(event);
      }
    };
  }

  /**
   * emit() - 触发事件（内部使用）
   */
  private emit(event: string, data: unknown): void {
    const callbacks = this.eventListeners.get(event);
    if (!callbacks || callbacks.size === 0) {
      return;
    }

    for (const callback of callbacks) {
      try {
        callback(data);
      } catch (error) {
        console.error(`[ActorRef] Event listener error:`, error);
      }
    }
  }

  /**
   * Reject all pending RPC requests immediately.
   *
   * Called when the WebRTC connection enters an irrecoverable state
   * ("failed" or "closed") so callers don't wait for the 30s timeout.
   */
  private rejectAllPending(reason: string): void {
    if (this.pendingRequests.size === 0) return;
    console.warn(
      `[ActorRef] Rejecting ${this.pendingRequests.size} pending RPCs: ${reason}`
    );
    for (const [requestId, pending] of this.pendingRequests) {
      clearTimeout(pending.timeout);
      pending.reject(new Error(reason));
    }
    this.pendingRequests.clear();
  }

  /**
   * 清理所有资源
   */
  dispose(): void {
    for (const [_requestId, pending] of this.pendingRequests) {
      clearTimeout(pending.timeout);
      pending.reject(new Error('ActorRef disposed'));
    }
    this.pendingRequests.clear();

    for (const topic of this.subscriptions.keys()) {
      this.domRuntime.getSWBridge().sendToSW({
        type: 'control',
        payload: {
          action: 'unsubscribe',
          topic,
        },
      });
    }
    this.subscriptions.clear();

    this.eventListeners.clear();
  }
}
