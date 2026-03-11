/**
 * Type definitions for Actor-RTC Web
 */

/**
 * Connection state
 */
export enum ConnectionState {
  Disconnected = 'disconnected',
  Connecting = 'connecting',
  Connected = 'connected',
  Reconnecting = 'reconnecting',
  Failed = 'failed',
}

/**
 * Service client interface
 */
export interface ServiceClient {
  /** Service name */
  readonly serviceName: string;
}

/**
 * Message metadata
 */
export interface MessageMetadata {
  /** Message ID */
  id: string;

  /** Timestamp */
  timestamp: number;

  /** Sender */
  from?: string;
}

/**
 * RPC options
 */
export interface RpcOptions {
  /** Request timeout in milliseconds */
  timeout?: number;

  /** Retry configuration */
  retry?: {
    maxAttempts?: number;
    initialDelay?: number;
  };

  /** Request metadata */
  metadata?: Record<string, string>;
}

/**
 * Subscription callback
 *
 * ，metadata  WASM ，UI 
 */
export type SubscriptionCallback<T> = (data: T) => void;

/**
 * Unsubscribe function
 */
export type UnsubscribeFn = () => void;

/**
 * Stream handle
 */
export interface StreamHandle {
  /** Stream ID */
  id: string;

  /** Close the stream */
  close(): Promise<void>;
}
