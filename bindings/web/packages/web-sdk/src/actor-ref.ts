/**
 * ActorRef - Actor 
 *
 * ： @actr/dom  Service Worker WASM 
 */

import {
  ActrDomRuntime,
  RpcResponsePayload,
  SubscriptionDataPayload,
  WebRtcEventPayload,
} from '@actr/dom';
import type { SubscriptionCallback, UnsubscribeFn } from './types';

/**
 * RPC 
 */
export interface RpcRequest<T = unknown> {
  service: string;
  method: string;
  params: T;
  timeout?: number;
}

/**
 * Actor 
 *
 *  API：
 * - call(): -（State Path，30-40ms）
 * - subscribe(): （Fast Path，6-13ms）
 * - on(): （<5ms）
 */
export class ActorRef {
  private domRuntime: ActrDomRuntime;
  private requestId = 0;

  /**  RPC  */
  private pendingRequests = new Map<
    string,
    {
      resolve: (value: Uint8Array) => void;
      reject: (error: Error) => void;
      timeout: number;
    }
  >();

  /**  */
  private subscriptions = new Map<string, Set<SubscriptionCallback<any>>>();

  /**  */
  private eventListeners = new Map<string, Set<(...args: any[]) => void>>();

  constructor(domRuntime: ActrDomRuntime) {
    this.domRuntime = domRuntime;
    this.setupMessageHandlers();
  }

  /**
   *  Service Worker 
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
          console.warn(`[ActorRef] Unknown message type: ${(message as any).type}`);
      }
    });
  }

  /**
   *  RPC 
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
   * 
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
   *  WebRTC 
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
   * call() - -（State Path）
   *
   * @param service - 
   * @param method - 
   * @param params - 
   * @param timeout - （）， 30000ms
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
   * callRaw() -  RPC payload（Raw bytes）。
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
   * subscribe() - （Fast Path）
   *
   * @param topic - 
   * @param callback - 
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
   * on() - 
   *
   * @param event - 
   * @param callback - 
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
   * emit() - （）
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
   * 
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
