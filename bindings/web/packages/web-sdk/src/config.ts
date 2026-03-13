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

  /** This actor's type (manufacturer:name) */
  client_actr_type: string;

  /** Target actor type for peer discovery (manufacturer:name) */
  target_actr_type: string;

  /** Service fingerprint for exact matching (optional) */
  service_fingerprint: string;

  /** ACL allow-list of actor types */
  acl_allow_types: string[];

  /** Whether this actor is a server (registers and waits) or client (discovers) */
  is_server: boolean;
}

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
