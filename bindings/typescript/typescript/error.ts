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

interface StructuredPayload {
  kind: ActrErrorKind;
  code: ActrErrorCode;
  message: string;
  service_name?: string;
}

/**
 * Typed error thrown from every ACTR native call.
 *
 * `kind` is the fault-domain bucket (drive retry / DLQ policy off this),
 * `code` is the exact protocol variant, and `service_name` is populated
 * only when `code === 'DependencyNotFound'`.
 */
export class ActrError extends Error {
  readonly kind: ActrErrorKind;
  readonly code: ActrErrorCode;
  readonly service_name?: string;

  constructor(payload: StructuredPayload) {
    super(payload.message);
    this.name = 'ActrError';
    this.kind = payload.kind;
    this.code = payload.code;
    if (payload.service_name !== undefined) {
      this.service_name = payload.service_name;
    }
    // Preserve V8 stack-trace ergonomics in Node.
    if (typeof (Error as { captureStackTrace?: unknown }).captureStackTrace === 'function') {
      (Error as unknown as { captureStackTrace: (t: unknown, c: unknown) => void })
        .captureStackTrace(this, ActrError);
    }
  }

  /** `true` iff the error is in the Transient fault domain. */
  isRetryable(): boolean {
    return this.kind === 'Transient';
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
