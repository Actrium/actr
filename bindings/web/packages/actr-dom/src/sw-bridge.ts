/**
 * Service Worker Bridge - PostMessage 
 *
 *  DOM  Service Worker 
 */

export type MessageToSW =
  | {
    type: 'fast_path_data';
    payload:
    | { streamId: string; data: Uint8Array; timestamp: number }
    | { batch: { streamId: string; data: Uint8Array; timestamp: number }[] };
  }
  | { type: 'webrtc_event'; payload: WebRtcEventPayload }
  | { type: 'control'; payload: ControlCommandPayload }
  | { type: 'register_datachannel_port'; payload: { peerId: string; port: MessagePort } };

/**
 * Control command payload
 * Corresponds to ControlCommand in Rust
 */
export type ControlCommandPayload =
  | {
    action: 'rpc_call';
    request_id: string;
    request: Uint8Array | { route_key: string; payload: Uint8Array; timeout?: number } | any;
  }
  | { action: 'subscribe'; topic: string }
  | { action: 'unsubscribe'; topic: string };

/**
 * WebRTC command payload
 * Corresponds to WebRtcCommandPayload in Rust
 */
export type WebRtcCommandPayload =
  | { action: 'create_peer'; peerId: string; payload?: never }
  | {
    action: 'set_remote_description';
    peerId: string;
    payload: { sdp: RTCSessionDescriptionInit };
  }
  | { action: 'set_local_description'; peerId: string; payload: { sdp: RTCSessionDescriptionInit } }
  | { action: 'add_ice_candidate'; peerId: string; payload: { candidate: RTCIceCandidateInit } }
  | { action: 'create_offer'; peerId: string; payload?: never }
  | { action: 'create_ice_restart_offer'; peerId: string; payload?: never }
  | { action: 'create_answer'; peerId: string; payload?: never }
  | { action: 'close_peer'; peerId: string; payload?: never }
  | { action: 'send_data'; peerId: string; payload: { channelId: number; data: Uint8Array } };

/**
 * RPC response payload
 * Corresponds to RpcResponsePayload in Rust
 */
export interface RpcResponsePayload {
  request_id: string;
  data: Uint8Array | null;
  error?: {
    code: number;
    message: string;
  } | null;
}

/**
 * Subscription data payload
 */
export interface SubscriptionDataPayload {
  topic: string;
  data: Uint8Array;
}

/**
 * WebRTC event payload
 * Corresponds to DomWebRtcEvent in Rust
 */
export type WebRtcEventPayload =
  | {
    eventType: 'connection_state_changed';
    data: { peerId: string; state: RTCPeerConnectionState };
  }
  | { eventType: 'datachannel_open'; data: { peerId: string; channelId: number; label: string } }
  | { eventType: 'datachannel_close'; data: { peerId: string; channelId: number } }
  | { eventType: 'local_description'; data: { peerId: string; sdp: RTCSessionDescriptionInit } }
  | {
    eventType: 'ice_restart_local_description';
    data: { peerId: string; sdp: RTCSessionDescriptionInit };
  }
  | { eventType: 'ice_candidate'; data: { peerId: string; candidate: RTCIceCandidateInit } }
  | { eventType: 'command_error'; data: { peerId: string; action: string; error: string } }
  | { eventType: 'sw_log'; data: unknown };

export type MessageFromSW =
  | { type: 'webrtc_command'; payload: WebRtcCommandPayload }
  | { type: 'control_response'; payload: RpcResponsePayload }
  | { type: 'subscription_data'; payload: SubscriptionDataPayload }
  | { type: 'webrtc_event'; payload: WebRtcEventPayload }
  | { type: 'update_turn_credential'; payload: { username: string; password: string } };

export type MessageHandler = (message: MessageFromSW) => void;

/**
 * Service Worker 
 */
export class ServiceWorkerBridge {
  private swPort: MessagePort | null = null;
  private messageHandlers: Set<MessageHandler> = new Set();
  private readyPromise: Promise<void>;
  private resolveReady!: () => void;
  private clientId: string;

  constructor() {
    this.readyPromise = new Promise((resolve) => {
      this.resolveReady = resolve;
    });
    // Generate a unique client ID for this browser tab
    this.clientId = `client_${Date.now()}_${Math.random().toString(36).substring(2, 8)}`;
  }

  /**
   *  Service Worker 
   */
  async initialize(serviceWorkerUrl: string, runtimeConfig?: Record<string, unknown>): Promise<void> {
    //  Service Worker
    if ('serviceWorker' in navigator) {
      const registration = await navigator.serviceWorker.register(serviceWorkerUrl, {
        updateViaCache: 'none',
      });
      await registration.update();

      //  Service Worker 
      await navigator.serviceWorker.ready;

      // Wait for the controller to be set (may not be immediate after fresh registration).
      // The SW calls clients.claim() on activate, which triggers 'controllerchange'.
      if (!navigator.serviceWorker.controller) {
        await new Promise<void>((resolve) => {
          const onControllerChange = () => {
            navigator.serviceWorker.removeEventListener('controllerchange', onControllerChange);
            resolve();
          };
          navigator.serviceWorker.addEventListener('controllerchange', onControllerChange);
          // Safety timeout: if controller never arrives, proceed with registration.active
          setTimeout(onControllerChange, 3000);
        });
      }

      const target = navigator.serviceWorker.controller ?? registration.active;
      if (!target) {
        throw new Error('Service Worker active target not available');
      }

      navigator.serviceWorker.addEventListener('message', (event) => {
        console.log('[SW Bridge] <- SW (client)', event.data); // [DEBUG] Keep for now
      });

      //  MessageChannel
      const channel = new MessageChannel();
      this.swPort = channel.port1;

      //  SW 
      this.swPort.onmessage = (event) => {
        this.handleMessageFromSW(event.data);
      };

      //  Service Worker
      target.postMessage(
        {
          type: 'DOM_PORT_INIT',
          port: channel.port2,
          clientId: this.clientId,
          runtimeConfig: runtimeConfig ?? undefined,
        },
        [channel.port2]
      );
      target.postMessage({ type: 'PING' });

      this.resolveReady();
      console.log('[SW Bridge] Initialized');
    } else {
      throw new Error('Service Worker not supported');
    }
  }

  /**
   * 
   */
  async waitReady(): Promise<void> {
    return this.readyPromise;
  }

  /**
   *  Service Worker
   */
  sendToSW(message: MessageToSW, transferables?: Transferable[]): void {
    if (!this.swPort) {
      console.warn('[SW Bridge] Cannot send: bridge not initialized or already closed');
      return;
    }

    console.log('[SW Bridge] -> SW', message); // [DEBUG] Keep for now
    if (transferables && transferables.length > 0) {
      //  Transferable 
      this.swPort.postMessage(message, transferables);
    } else {
      this.swPort.postMessage(message);
    }
  }

  /**
   *  DataChannel MessagePort  Service Worker
   *
   * DOM  DataChannel ：
   * 1.  MessageChannel  DataChannel ↔ SW
   * 2. port1  DOM （ DataChannel ）
   * 3. port2  Transferable  SW
   *
   * SW  WirePool，
   *  DataLane::PostMessage(port2) 。
   */
  sendDataChannelPort(peerId: string, port: MessagePort): void {
    this.sendToSW(
      { type: 'register_datachannel_port', payload: { peerId, port } },
      [port]
    );
  }

  /**
   * 
   */
  onMessage(handler: MessageHandler): () => void {
    this.messageHandlers.add(handler);

    // 
    return () => {
      this.messageHandlers.delete(handler);
    };
  }

  /**
   *  SW 
   */
  private handleMessageFromSW(message: MessageFromSW): void {
    console.log('[SW Bridge] <- SW', message); // [DEBUG] Keep for now
    for (const handler of this.messageHandlers) {
      try {
        handler(message);
      } catch (error) {
        console.error('[SW Bridge] Handler error:', error);
      }
    }
  }

  /**
   * 
   */
  getClientId(): string {
    return this.clientId;
  }

  /**
   * 
   */
  close(): void {
    if (this.swPort) {
      this.swPort.close();
      this.swPort = null;
    }
    this.messageHandlers.clear();
  }
}
