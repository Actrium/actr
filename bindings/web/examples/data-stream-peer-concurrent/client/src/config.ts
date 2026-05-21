import type { ActorConfig, SwRuntimeConfig } from '@actr/web';

const actrixHttpUrl = (
    import.meta.env.VITE_ACTRIX_HTTP_URL || 'http://localhost:8081'
).replace(/\/+$/, '');
const iceServers = import.meta.env.VITE_ACTRIX_STUN_URL
    ? [{ urls: import.meta.env.VITE_ACTRIX_STUN_URL }]
    : [];

export const runtimeConfig: SwRuntimeConfig = {
    ais_endpoint: `${actrixHttpUrl}/ais`,
    signaling_url:
        import.meta.env.VITE_ACTRIX_SIGNALING_URL ||
        `${actrixHttpUrl.replace(/^http/, 'ws')}/signaling/ws`,
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
    iceServers,
    debug: true,
};
