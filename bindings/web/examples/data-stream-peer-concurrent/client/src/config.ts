import type { ActorConfig } from '@actr/web';

export const runtimeConfig = {
    signaling_url: 'wss://localhost:8081/signaling/ws',
    realm_id: 2368266035,
    client_actr_type: 'acme:DataStreamPeerConcurrentClient',
    target_actr_type: 'acme:DataStreamPeerConcurrentServer',
    service_fingerprint: '',
    acl_allow_types: ['acme:DataStreamPeerConcurrentServer'],
    is_server: false,
};

export const actrConfig: ActorConfig = {
    signalingUrl: runtimeConfig.signaling_url,
    realm: String(runtimeConfig.realm_id),
    serviceWorkerPath: '/actor.sw.js',
    runtimeConfig,
    iceServers: [{ urls: 'stun:stun.l.google.com:19302' }],
    debug: true,
};
