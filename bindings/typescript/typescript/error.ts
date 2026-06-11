/**
 * Structured ActrError bridge.
 *
 * The native napi-rs binding signals every protocol-level failure by
 * throwing a plain `Error` whose `.message` is a JSON payload:
 *
 *   { "kind": "Client", "code": "DependencyNotFound",
 *     "message": "…", "service_name": "echo" }
 *
 * Consumers typically just want to branch on fault domain (retry vs. DLQ
 * vs. fail fast). Parsing JSON off `.message` at every call site is a
 * paper cut, so we wrap each native call in `mapNativeError` and surface a
 * proper `ActrError` subclass of `Error` that carries strongly-typed
 * classification fields.
 */

export type ActrErrorKind = 'Transient' | 'Client' | 'Internal' | 'Corrupt';

export type ActrErrorCode =
  | 'Unavailable'
  | 'Recovering'
  | 'TimedOut'
  | 'NotFound'
  | 'PermissionDenied'
  | 'InvalidArgument'
  | 'UnknownRoute'
  | 'DependencyNotFound'
  | 'DecodeFailure'
  | 'NotImplemented'
  | 'Internal'
  | 'Config'
  | 'HyperBootstrap';

export type ActrRecoveryCode =
  | 'PeerDisconnected'
  | 'PeerFailed'
  | 'IceNetworkStarted'
  | 'RecoveryTimeout'
  | 'TransportClosing';

export type ActrDeliveryState = 'NotSent' | 'DeliveryUncertain';

export interface ActrErrorPeerId {
  realm_id: number;
  serial_number: number;
  type: {
    manufacturer: string;
    name: string;
    version: string;
  };
}

interface StructuredPayload {
  kind: ActrErrorKind;
  code: ActrErrorCode;
  message: string;
  service_name?: string;
  recovery_code?: ActrRecoveryCode;
  peer?: ActrErrorPeerId;
  session_id?: number | null;
  reason?: string;
  elapsed_ms?: number;
  timeout_ms?: number;
  retry_after_ms?: number | null;
  delivery?: ActrDeliveryState;
}

/**
 * Typed error thrown from every ACTR native call.
 *
 * `kind` is the fault-domain bucket (drive retry / DLQ policy off this),
 * `code` is the exact protocol variant, and `service_name` is populated
 * only when `code === 'DependencyNotFound'`.
 *
 * When `code === 'Recovering'`, the recovery fields describe the peer
 * connection recovery window. `delivery === 'NotSent'` means the operation was
 * stopped by preflight before bytes were written to the transport.
 */
export class ActrError extends Error {
  readonly kind: ActrErrorKind;
  readonly code: ActrErrorCode;
  readonly service_name?: string;
  readonly recovery_code?: ActrRecoveryCode;
  readonly peer?: ActrErrorPeerId;
  readonly session_id?: number | null;
  readonly reason?: string;
  readonly elapsed_ms?: number;
  readonly timeout_ms?: number;
  readonly retry_after_ms?: number | null;
  readonly delivery?: ActrDeliveryState;

  constructor(payload: StructuredPayload) {
    super(payload.message);
    this.name = 'ActrError';
    this.kind = payload.kind;
    this.code = payload.code;
    if (payload.service_name !== undefined) {
      this.service_name = payload.service_name;
    }
    if (payload.recovery_code !== undefined) {
      this.recovery_code = payload.recovery_code;
    }
    if (payload.peer !== undefined) {
      this.peer = payload.peer;
    }
    if (payload.session_id !== undefined) {
      this.session_id = payload.session_id;
    }
    if (payload.reason !== undefined) {
      this.reason = payload.reason;
    }
    if (payload.elapsed_ms !== undefined) {
      this.elapsed_ms = payload.elapsed_ms;
    }
    if (payload.timeout_ms !== undefined) {
      this.timeout_ms = payload.timeout_ms;
    }
    if (payload.retry_after_ms !== undefined) {
      this.retry_after_ms = payload.retry_after_ms;
    }
    if (payload.delivery !== undefined) {
      this.delivery = payload.delivery;
    }
    // Preserve V8 stack-trace ergonomics in Node.
    if (
      typeof (Error as { captureStackTrace?: unknown }).captureStackTrace ===
      'function'
    ) {
      (
        Error as unknown as {
          captureStackTrace: (t: unknown, c: unknown) => void;
        }
      ).captureStackTrace(this, ActrError);
    }
  }

  /** `true` iff the error is in the Transient fault domain. */
  isRetryable(): boolean {
    return this.kind === 'Transient';
  }

  /** `true` iff this is a WebRTC recovery-window preflight error. */
  isRecovering(): boolean {
    return this.code === 'Recovering';
  }

  /** `true` iff the error should be routed to a Dead Letter Queue. */
  requiresDlq(): boolean {
    return this.kind === 'Corrupt';
  }
}

function isStructuredPayload(value: unknown): value is StructuredPayload {
  if (typeof value !== 'object' || value === null) return false;
  const p = value as Record<string, unknown>;
  return (
    typeof p.kind === 'string' &&
    typeof p.code === 'string' &&
    typeof p.message === 'string'
  );
}

/**
 * If `err` carries a JSON payload produced by the Rust binding, re-wrap
 * it as an `ActrError`; otherwise return it unchanged so non-ACTR errors
 * keep their original identity.
 */
export function mapNativeError(err: unknown): unknown {
  if (err instanceof ActrError) return err;
  if (!(err instanceof Error)) return err;
  const raw = err.message;
  if (typeof raw !== 'string' || raw.length === 0 || raw[0] !== '{') {
    return err;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    // Not a structured payload — leave the error alone so consumers can
    // still see the original message.
    return err;
  }
  if (!isStructuredPayload(parsed)) return err;
  const wrapped = new ActrError(parsed);
  if (err.stack) wrapped.stack = err.stack;
  return wrapped;
}

/**
 * Invoke an async native call and re-throw ACTR failures as `ActrError`.
 *
 * Used by the thin TS wrappers around the napi-rs-generated classes.
 */
export async function callNative<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (err) {
    throw mapNativeError(err);
  }
}
