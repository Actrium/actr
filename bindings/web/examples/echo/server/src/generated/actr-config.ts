/**
 * 自动生成的 Actr 配置
 *
 * ⚠️  请勿手动编辑此文件
 * 此文件由 actr gen 命令根据 Actr.toml 生成
 */

import type { ActorClientConfig } from '@actr/web';

/**
 * Actr 配置
 */
export const actrConfig: ActorClientConfig = {
    signalingUrl: 'wss://10.30.3.206:8081/signaling/ws',
    realm: '2368266035',
    iceServers: [
        { urls: 'stun:10.30.3.206:3478' },
    ],
};

/**
 * Actor 类型信息
 */
export const actorType = {
    manufacturer: 'acme',
    name: 'EchoService',
    fullType: 'acme+EchoService',
};
