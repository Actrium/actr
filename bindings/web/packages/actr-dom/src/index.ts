/**
 * @actr/dom - Actor-RTC DOM-side Fixed Forwarding Layer
 *
 *  JS （Hardware Abstraction Layer），。
 *
 * ：
 * 1.  WebRTC （DOM  WebRTC API）
 * 2.  WebRTC  Service Worker WASM
 * 3.  Service Worker  PostMessage 
 *
 * ：docs/architecture/wasm-dom-integration.md
 */

import { ServiceWorkerBridge } from './sw-bridge';
import { FastPathForwarder } from './fast-path-forwarder';
import { WebRtcCoordinator } from './webrtc-coordinator';

export { ServiceWorkerBridge } from './sw-bridge';
export type {
  MessageToSW,
  MessageFromSW,
  MessageHandler,
  RpcResponsePayload,
  SubscriptionDataPayload,
  WebRtcEventPayload,
} from './sw-bridge';

export { FastPathForwarder } from './fast-path-forwarder';
export type { FastPathData } from './fast-path-forwarder';

export { WebRtcCoordinator } from './webrtc-coordinator';
export type { WebRtcConfig, PeerConnectionInfo } from './webrtc-coordinator';

/**
 * Actor-RTC DOM 
 */
export interface ActrDomConfig {
  serviceWorkerUrl: string;
  webrtcConfig?: {
    iceServers?: RTCIceServer[];
    iceTransportPolicy?: RTCIceTransportPolicy;
  };
  /** Runtime config forwarded to Service Worker WASM layer */
  runtimeConfig?: Record<string, unknown>;
}

/**
 * Actor-RTC DOM 
 *
 * 
 */
export class ActrDomRuntime {
  private swBridge: ServiceWorkerBridge;
  private forwarder: FastPathForwarder;
  private coordinator: WebRtcCoordinator;

  constructor(
    swBridge: ServiceWorkerBridge,
    forwarder: FastPathForwarder,
    coordinator: WebRtcCoordinator
  ) {
    this.swBridge = swBridge;
    this.forwarder = forwarder;
    this.coordinator = coordinator;
  }

  /**
   *  Service Worker 
   */
  getSWBridge(): ServiceWorkerBridge {
    return this.swBridge;
  }

  /**
   *  Fast Path 
   */
  getForwarder(): FastPathForwarder {
    return this.forwarder;
  }

  /**
   *  WebRTC 
   */
  getCoordinator(): WebRtcCoordinator {
    return this.coordinator;
  }

  /**
   * 
   */
  dispose(): void {
    this.coordinator.dispose();
    this.forwarder.dispose();
    this.swBridge.close();
  }
}

/**
 *  Actor-RTC DOM 
 *
 * @example
 * ```typescript
 * //  HTML 
 * import { initActrDom } from '@actr/dom';
 *
 * const runtime = await initActrDom({
 *   serviceWorkerUrl: '/my-actor.sw.js',
 *   webrtcConfig: {
 *     iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
 *   },
 * });
 *
 * console.log('Actor-RTC DOM runtime initialized');
 * ```
 */
export async function initActrDom(config: ActrDomConfig): Promise<ActrDomRuntime> {
  console.log('[actr-dom] Initializing...');

  // 1.  Service Worker 
  const swBridge = new ServiceWorkerBridge();
  await swBridge.initialize(config.serviceWorkerUrl, config.runtimeConfig);

  // 2.  Fast Path 
  const forwarder = new FastPathForwarder(swBridge);

  // 3.  WebRTC 
  const coordinator = new WebRtcCoordinator(swBridge, forwarder, config.webrtcConfig || {});

  console.log('[actr-dom] Initialized successfully');

  return new ActrDomRuntime(swBridge, forwarder, coordinator);
}

/**
 * 
 */
export default {
  initActrDom,
  ServiceWorkerBridge,
  FastPathForwarder,
  WebRtcCoordinator,
  ActrDomRuntime,
};
