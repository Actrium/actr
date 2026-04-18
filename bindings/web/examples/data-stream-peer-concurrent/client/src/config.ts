import type { ActorConfig, SwRuntimeConfig } from '@actr/web';

export const runtimeConfig: SwRuntimeConfig = {
    ais_endpoint: 'http://localhost:8081/ais',
    signaling_url: 'ws://localhost:8081/signaling/ws',
    realm_id: 2368266035,
    client_actr_type: 'acme:DataStreamPeerConcurrentClient:0.1.0',
    target_actr_type: 'acme:DataStreamPeerConcurrentServer:0.1.0',
    service_fingerprint: '',
    acl_allow_types: ['acme:DataStreamPeerConcurrentServer:0.1.0'],
};

export const actrConfig: ActorConfig = {
    signalingUrl: runtimeConfig.signaling_url,
    realm: String(runtimeConfig.realm_id),
    serviceWorkerPath: '/actor.sw.js',
    runtimeConfig,
    iceServers: [{ urls: 'stun:localhost:3478' }],
    debug: true,
};
