/**
 * Actr Runtime Configuration Loader
 *
 * Fetches runtime config from /actr-runtime-config.json served by `actr run --web`.
 * For Vite dev mode, place a static actr-runtime-config.json in public/.
 *
 * This file replaces the old auto-generated actr-config.ts.
 * Config now comes from the runtime config endpoint instead of build-time generation.
 */

import type { ActorClientConfig, SwRuntimeConfig, TrustAnchor } from '@actr/web';

// ── Runtime Config JSON shape ──

export interface RuntimeConfigJson {
    signaling_url: string;
    ais_endpoint: string;
    realm_id: number;
    visible: boolean;
    force_relay: boolean;
    stun_urls: string[];
    turn_urls: string[];
    package: {
        name: string;
        manufacturer: string;
        actr_name: string;
        version: string;
        full_type: string;
    };
    acl_allow_types: string[];
    package_url: string;
    runtime_wasm_url: string;
    trust: TrustAnchor[];
}

// ── Internal state ──

let _config: RuntimeConfigJson | null = null;

/**
 * Load runtime configuration from the /actr-runtime-config.json endpoint.
 * Must be called once before using any config exports.
 */
export async function initConfig(): Promise<RuntimeConfigJson> {
    const resp = await fetch('/actr-runtime-config.json');
    if (!resp.ok) {
        throw new Error(
            `Failed to load runtime config: ${resp.status} ${resp.statusText}`
        );
    }
    _config = (await resp.json()) as RuntimeConfigJson;
    return _config;
}

function requireConfig(): RuntimeConfigJson {
    if (!_config) {
        throw new Error('Config not loaded. Call initConfig() first.');
    }
    return _config;
}

// ── Backward-compatible exports ──

/** ActrType info (available after initConfig) */
export const actrType = {
    get manufacturer() {
        return requireConfig().package.manufacturer;
    },
    get name() {
        return requireConfig().package.actr_name;
    },
    get version() {
        return requireConfig().package.version;
    },
    get fullType() {
        return requireConfig().package.full_type;
    },
};

/** System config (available after initConfig) */
export const system = {
    signaling: {
        get url() {
            return requireConfig().signaling_url;
        },
    },
    deployment: {
        get realm_id() {
            return requireConfig().realm_id;
        },
        get ais_endpoint() {
            return requireConfig().ais_endpoint;
        },
    },
    discovery: {
        get visible() {
            return requireConfig().visible;
        },
    },
    webrtc: {
        get force_relay() {
            return requireConfig().force_relay;
        },
        get stun_urls() {
            return requireConfig().stun_urls;
        },
        get turn_urls() {
            return requireConfig().turn_urls;
        },
    },
};

/** ACL config */
export const acl = {
    get rules() {
        return requireConfig().acl_allow_types.map((t) => ({
            permission: 'allow' as const,
            types: [t],
        }));
    },
};

/**
 * Build Service Worker runtime config from the loaded JSON.
 */
export function buildRuntimeConfig(): SwRuntimeConfig {
    const c = requireConfig();
    return {
        ais_endpoint: c.ais_endpoint,
        signaling_url: c.signaling_url,
        realm_id: c.realm_id,
        client_actr_type: c.package.full_type,
        target_actr_type: c.acl_allow_types[0] || '',
        service_fingerprint: '',
        acl_allow_types: c.acl_allow_types,
        package_url: c.package_url,
        runtime_wasm_url: c.runtime_wasm_url,
        trust: c.trust,
    };
}

/** runtimeConfig (available after initConfig) */
export const runtimeConfig: SwRuntimeConfig = new Proxy({} as SwRuntimeConfig, {
    get(_target, prop) {
        return (buildRuntimeConfig() as Record<string | symbol, unknown>)[prop];
    },
});

/**
 * Build Actor client config from the loaded JSON.
 */
export function buildActrConfig(): ActorClientConfig {
    const c = requireConfig();
    const rc = buildRuntimeConfig();
    return {
        signalingUrl: c.signaling_url,
        realm: String(c.realm_id),
        iceServers: [
            ...c.stun_urls.map((url: string) => ({ urls: url })),
            ...c.turn_urls.map((url: string) => ({ urls: url })),
        ],
        iceTransportPolicy: 'all',
        runtimeConfig: rc,
    };
}

/** actrConfig (available after initConfig) */
export const actrConfig: ActorClientConfig = new Proxy(
    {} as ActorClientConfig,
    {
        get(_target, prop) {
            return (buildActrConfig() as Record<string | symbol, unknown>)[prop];
        },
    }
);
