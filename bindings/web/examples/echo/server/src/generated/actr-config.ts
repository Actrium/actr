/**
 * 自动生成的 Actr 配置
 * 来源: Actr.toml
 *
 * ⚠️  请勿手动编辑此文件
 * 此文件由 actr gen 命令根据 Actr.toml 生成
 */

import type { ActorClientConfig, SwRuntimeConfig } from '@actr/web';

// ── Actr.toml 完整信息 ──

/** Actr.toml edition */
export const edition = 1;

/** 导出的 proto 文件 */
export const exports = ['proto/echo.proto'];

/** 包信息 */
export const packageInfo = {
    name: 'echo-server',
    description: 'Echo Server - Browser-side Actor Service',
    authors: ['Actr Dev Team'],
    license: 'Apache-2.0',
    tags: ['dev', 'server'],
} as const;

/** ActrType */
export const actorType = {
    manufacturer: 'acme',
    name: 'EchoService',
    fullType: 'acme:EchoService',
} as const;

/** 依赖 (无) */
export const dependencies = {} as const;

/** Web 平台配置 */
export const platform = {
    web: {
        generate_types: true,
        types_output: './src/generated',
        worker_type: 'service-worker' as const,
        service_worker_path: '/actor.sw.js',
        generate_sw_entry: true,
    },
} as const;

/** 系统配置 */
export const system = {
    signaling: {
        url: 'wss://10.30.3.206:8081/signaling/ws',
    },
    deployment: {
        realm_id: 2368266035,
    },
    discovery: {
        visible: true,
    },
    observability: {
        filter_level: 'info',
        tracing_enabled: false,
    },
    webrtc: {
        stun_urls: ['stun:10.30.3.206:3478'],
    },
} as const;

/** ACL 配置 */
export const acl = {
    rules: [
        {
            permission: 'allow' as const,
            types: ['acme:echo-client-app'],
        },
    ],
} as const;

// ── runtimeConfig (传给 Service Worker WASM 注册) ──

/**
 * Service Worker 运行时配置
 * 由主线程通过 DOM_PORT_INIT 传递给 Service Worker
 */
export const runtimeConfig: SwRuntimeConfig = {
  signaling_url: system.signaling.url,
  realm_id: system.deployment.realm_id,
  client_actr_type: 'acme:EchoService',
  target_actr_type: 'acme:echo-client-app',
  service_fingerprint: '',
  acl_allow_types: ['acme:echo-client-app'],
  is_server: true,
};

// ── ActorClientConfig (传给 createActor) ──

/**
 * Actr 配置
 * 从 Actr.toml 的 system 配置中提取
 */
export const actrConfig: ActorClientConfig = {
    signalingUrl: system.signaling.url,
    realm: String(system.deployment.realm_id),
    iceServers: [
        ...system.webrtc.stun_urls.map((url) => ({ urls: url })),
    ],
    runtimeConfig,
};
