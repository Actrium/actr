/**
 * @actr/dom - Actor-RTC DOM-side Fixed Forwarding Layer
 *
 * 这是框架提供的固定 JS 层（Hardware Abstraction Layer），用户无需修改。
 *
 * 职责：
 * 1. 管理 WebRTC 连接（DOM 侧才能访问 WebRTC API）
 * 2. 接收 WebRTC 数据并零拷贝转发到 Service Worker WASM
 * 3. 提供与 Service Worker 的 PostMessage 通信桥梁
 *
 * 架构决策：docs/architecture/wasm-dom-integration.md
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
 * Actor-RTC DOM 运行时配置
 */
export interface ActrDomConfig {
  serviceWorkerUrl: string;
  webrtcConfig?: {
    iceServers?: RTCIceServer[];
    iceTransportPolicy?: RTCIceTransportPolicy;
  };
}

/**
 * Actor-RTC DOM 运行时
 *
 * 这是用户唯一需要初始化的对象
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
   * 获取 Service Worker 桥接
   */
  getSWBridge(): ServiceWorkerBridge {
    return this.swBridge;
  }

  /**
   * 获取 Fast Path 转发器
   */
  getForwarder(): FastPathForwarder {
    return this.forwarder;
  }

  /**
   * 获取 WebRTC 协调器
   */
  getCoordinator(): WebRtcCoordinator {
    return this.coordinator;
  }

  /**
   * 清理所有资源
   */
  dispose(): void {
    this.coordinator.dispose();
    this.forwarder.dispose();
    this.swBridge.close();
  }
}

/**
 * 初始化 Actor-RTC DOM 运行时
 *
 * @example
 * ```typescript
 * // 在用户 HTML 页面中引入
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

  // 1. 创建 Service Worker 桥接
  const swBridge = new ServiceWorkerBridge();
  await swBridge.initialize(config.serviceWorkerUrl);

  // 2. 创建 Fast Path 转发器
  const forwarder = new FastPathForwarder(swBridge);

  // 3. 创建 WebRTC 协调器
  const coordinator = new WebRtcCoordinator(swBridge, forwarder, config.webrtcConfig || {});

  console.log('[actr-dom] Initialized successfully');

  return new ActrDomRuntime(swBridge, forwarder, coordinator);
}

/**
 * 默认导出
 */
export default {
  initActrDom,
  ServiceWorkerBridge,
  FastPathForwarder,
  WebRtcCoordinator,
  ActrDomRuntime,
};
