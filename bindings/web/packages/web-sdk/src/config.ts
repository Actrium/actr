/**
 * Configuration types for Actor-RTC Web
 */

/**
 * Worker type
 */
export type WorkerType = 'service-worker' | 'web-worker';

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
