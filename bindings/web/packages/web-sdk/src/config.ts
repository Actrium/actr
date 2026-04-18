/**
 * Configuration types for Actor-RTC Web
 */

/**
 * Worker type
 */
export type WorkerType = 'service-worker' | 'web-worker';

/**
 * Service Worker runtime configuration
 *
 * Passed from main thread to Service Worker via DOM_PORT_INIT,
 * then forwarded to WASM register_client().
 */
export interface SwRuntimeConfig {
  /** AIS (Actor Identity Service) endpoint URL (e.g. http://host:port/ais) */
  ais_endpoint: string;

  /** Signaling WebSocket URL (e.g. wss://host:port/signaling/ws) */
  signaling_url: string;

  /** Deployment realm ID */
  realm_id: number;

  /** This actor's type (manufacturer:name:version) */
  client_actr_type: string;

  /** Target actor type for peer discovery (manufacturer:name:version) */
  target_actr_type: string;

  /** Service fingerprint for exact matching (optional) */
  service_fingerprint: string;

  /** ACL allow-list of actor types */
  acl_allow_types: string[];

  // ── Package loading (Web load_package_executor) ──

  /** URL of the .actr package to load (e.g. "/packages/echo-server.actr").
   *  The .actr package is a signed ZIP containing WASM binary, JS glue, and actor.sw.js.
   *  This is the Web equivalent of Rust Hyper's load_package_executor. */
  package_url?: string;

  /** Name of the wasm_bindgen register function to call after init (e.g. "register_echo_service") */
  register_fn?: string;

  // ── Legacy: direct file loading (development fallback) ──

  /** Filename or URL of the wasm-bindgen JS glue file (e.g. "echo_server.js").
   *  Used when package_url is not set. */
  package_js?: string;

  /** Filename or URL of the WASM binary (e.g. "echo_server_bg.wasm").
   *  Used when package_url is not set. */
  package_wasm?: string;

  // ── Guest Bridge: split runtime + guest loading ──

  /** URL of the runtime WASM + JS glue (e.g. "/packages/echo_server_bg.wasm").
   *  When set together with package_url, the guest bridge mode is activated:
   *  - Runtime WASM is loaded from this URL (+ derived JS glue URL)
   *  - The .actr package contains only the standard guest WASM (entry! FFI)
   *  - This enables sharing guest WASMs between web and native platforms. */
  runtime_wasm_url?: string;

  // ── Package verification (Web verify_package) ──

  /** Trust anchors for verifying the .actr package signature. Array form
   *  mirrors the server-side `[[trust]]` config in `actr.toml`.
   *
   *  The browser Service Worker currently honours only `kind = "static"`
   *  anchors (using `pubkey_b64`); `kind = "registry"` anchors cause the SW
   *  to skip verification with a warning, pending an async AIS lookup
   *  implementation.
   *
   *  When the array is empty or missing, verification is skipped. */
  trust?: TrustAnchor[];
}

/** Trust anchor config, matching `actr_config::TrustAnchor` on the server. */
export type TrustAnchor =
  | {
      /** Pre-shared Ed25519 public key; accepts any manufacturer. */
      kind: 'static';
      /** Base64 (standard) of the 32-byte Ed25519 public key. */
      pubkey_b64?: string;
      /** Path to a JSON file with a `public_key` field (resolved by the host
       *  before the config reaches the SW). */
      pubkey_file?: string;
    }
  | {
      /** Look up MFR public keys via AIS HTTP registry. Not yet implemented
       *  in the browser SW. */
      kind: 'registry';
      endpoint: string;
    };

/**
 * Actor System configuration
 */
export interface ActorSystemConfig {
  /** Signaling server URL */
  signalingUrl: string;

  /** Realm name */
  realm: string;

  /** Actor identity (optional) */
  identity?: string;

  /** Auto-reconnect on disconnect */
  autoReconnect?: boolean;

  /** Connection timeout in milliseconds */
  connectionTimeout?: number;

  /** Worker type to use */
  workerType?: WorkerType;

  /** Service Worker file path (default: '/actor.sw.js') */
  serviceWorkerPath?: string;

  /** STUN/TURN server configuration */
  iceServers?: RTCIceServer[];

  /** ICE transport policy ('all' or 'relay' for force_relay mode) */
  iceTransportPolicy?: RTCIceTransportPolicy;

  /** Runtime config for the Service Worker WASM layer */
  runtimeConfig?: SwRuntimeConfig;
}

/**
 * Actor Client configuration
 */
export interface ActorClientConfig extends ActorSystemConfig {
  /** Enable debug logging */
  debug?: boolean;

  /** Retry configuration */
  retry?: RetryConfig;
}

/**
 * Retry configuration
 */
export interface RetryConfig {
  /** Maximum retry attempts */
  maxAttempts?: number;

  /** Initial retry delay in milliseconds */
  initialDelay?: number;

  /** Maximum retry delay in milliseconds */
  maxDelay?: number;

  /** Backoff multiplier */
  backoffMultiplier?: number;
}

/**
 * Default configuration values
 */
export const DEFAULT_CONFIG: Partial<ActorClientConfig> = {
  autoReconnect: true,
  connectionTimeout: 30000,
  workerType: 'service-worker',
  debug: false,
  retry: {
    maxAttempts: 3,
    initialDelay: 1000,
    maxDelay: 10000,
    backoffMultiplier: 2,
  },
};
