/**
 * 自动生成的 Actr 配置
 * 来源: actr.toml
 *
 * ⚠️  请勿手动编辑此文件
 */

import type { ActorConfig } from '@actr/web';

/**
 * Actor 配置
 */
export const actrConfig: ActorConfig = {
  signalingUrl: 'wss://10.30.3.206:8081/signaling/ws',
  realm: '2368266035',
  iceServers: [
    { urls: 'stun:10.30.3.206:3478' },
  ],
  serviceWorkerPath: '/actor.sw.js',
  autoReconnect: true,
  debug: false,
};

/**
 * 包名称
 */
export const packageName = 'echo-real-client-app';

/**
 * ActrType
 */
export const actrType = {
  manufacturer: 'acme',
  name: 'echo-client-app',
};
