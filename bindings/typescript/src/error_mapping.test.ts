/**
 * Error-mapping tests for the TypeScript binding.
 *
 * Every protocol-level failure crosses the napi boundary as a plain `Error`
 * whose `.message` is a JSON payload (see `src/error.rs::build_payload`):
 *
 *   { "kind": "Client", "code": "UnknownRoute", "message": "…" }
 *
 * `mapNativeError` re-wraps that payload into a strongly-typed `ActrError`
 * carrying `kind` / `code` / `service_name` / `retry_after_ms`, with helper
 * predicates (`isRetryable`, `isConnectionNotReady`, `requiresDlq`).
 *
 * These tests assert that contract directly against the wire payloads the
 * Rust side emits, rather than spawning a live mock-actrix + echo service.
 * They mirror the original integration test's three protocol scenarios
 * (UnknownRoute / PermissionDenied / TimedOut) and extend coverage to the
 * fault-domain classification and recovery metadata that the current API
 * exposes.
 *
 * The wire shapes below are kept in lock-step with the Rust unit tests in
 * `src/error.rs` (`mod tests`) and the classification table in
 * `core/protocol/src/error.rs::Classify for ActrError`.
 */

import { describe, it, expect } from 'vitest';
import { ActrError, mapNativeError, callNative } from '../typescript';

/**
 * Build the exact JSON-in-message `Error` the napi layer throws, so the
 * tests exercise the real parse path instead of constructing `ActrError`
 * by hand.
 */
function nativeError(payload: Record<string, unknown>): Error {
  return new Error(JSON.stringify(payload));
}

// ── Protocol scenarios ported from the original integration test ─────────────

describe('Error mapping — protocol scenarios', () => {
  it('unknown_route_maps_to_client_unknown_route', () => {
    // Server has no handler for the route key → UnknownRoute (Client domain).
    const wrapped = mapNativeError(
      nativeError({
        kind: 'Client',
        code: 'UnknownRoute',
        message: 'no handler for route fixture.Fixture.NoSuchMethod',
      }),
    );

    expect(wrapped).toBeInstanceOf(ActrError);
    const err = wrapped as ActrError;
    expect(err.kind).toBe('Client');
    expect(err.code).toBe('UnknownRoute');
    // The offending route key is preserved in the surfaced message.
    expect(err.message).toContain('NoSuchMethod');
    expect(err.isRetryable()).toBe(false);
  });

  it('timeout_maps_to_transient_timed_out', () => {
    // Caller deadline fires before the handler returns → TimedOut (Transient).
    const wrapped = mapNativeError(
      nativeError({
        kind: 'Transient',
        code: 'TimedOut',
        message: 'operation timed out',
      }),
    );

    expect(wrapped).toBeInstanceOf(ActrError);
    const err = wrapped as ActrError;
    expect(err.kind).toBe('Transient');
    expect(err.code).toBe('TimedOut');
    expect(err.isRetryable()).toBe(true);
  });

  it('acl_deny_maps_to_client_permission_denied', () => {
    // Deny-all ACL blocks the caller → PermissionDenied (Client domain).
    const wrapped = mapNativeError(
      nativeError({
        kind: 'Client',
        code: 'PermissionDenied',
        message: 'permission denied: caller not in ACL',
      }),
    );

    expect(wrapped).toBeInstanceOf(ActrError);
    const err = wrapped as ActrError;
    expect(err.kind).toBe('Client');
    expect(err.code).toBe('PermissionDenied');
    expect(err.isRetryable()).toBe(false);
  });
});

// ── Fault-domain classification (Classify for ActrError) ─────────────────────

describe('Error mapping — fault-domain classification', () => {
  it('transient_variants_are_retryable', () => {
    for (const code of ['Unavailable', 'ConnectionNotReady', 'TimedOut']) {
      const err = mapNativeError(
        nativeError({ kind: 'Transient', code, message: code }),
      ) as ActrError;
      expect(err.isRetryable()).toBe(true);
      expect(err.requiresDlq()).toBe(false);
    }
  });

  it('client_variants_are_not_retryable', () => {
    for (const code of [
      'NotFound',
      'PermissionDenied',
      'InvalidArgument',
      'UnknownRoute',
      'DependencyNotFound',
    ]) {
      const err = mapNativeError(
        nativeError({ kind: 'Client', code, message: code }),
      ) as ActrError;
      expect(err.isRetryable()).toBe(false);
      expect(err.requiresDlq()).toBe(false);
    }
  });

  it('corrupt_decode_failure_requires_dlq', () => {
    const err = mapNativeError(
      nativeError({
        kind: 'Corrupt',
        code: 'DecodeFailure',
        message: 'malformed payload',
      }),
    ) as ActrError;
    expect(err.kind).toBe('Corrupt');
    expect(err.code).toBe('DecodeFailure');
    expect(err.requiresDlq()).toBe(true);
    expect(err.isRetryable()).toBe(false);
  });

  it('internal_variants_are_neither_retryable_nor_dlq', () => {
    for (const code of ['NotImplemented', 'Internal']) {
      const err = mapNativeError(
        nativeError({ kind: 'Internal', code, message: code }),
      ) as ActrError;
      expect(err.kind).toBe('Internal');
      expect(err.isRetryable()).toBe(false);
      expect(err.requiresDlq()).toBe(false);
    }
  });
});

// ── Recovery metadata carried by specific variants ───────────────────────────

describe('Error mapping — recovery metadata', () => {
  it('connection_not_ready_carries_retry_hint', () => {
    // Mirrors src/error.rs: ConnectionNotReadyInfo::new(120, 6000) → 5880.
    const err = mapNativeError(
      nativeError({
        kind: 'Transient',
        code: 'ConnectionNotReady',
        message: 'connection not ready: retry_after_ms=Some(5880)',
        retry_after_ms: 5880,
      }),
    ) as ActrError;

    expect(err.code).toBe('ConnectionNotReady');
    expect(err.isConnectionNotReady()).toBe(true);
    expect(err.isRetryable()).toBe(true);
    expect(err.retry_after_ms).toBe(5880);
  });

  it('connection_not_ready_retry_hint_may_be_null', () => {
    // ConnectionNotReadyInfo::without_retry_hint() → null on the wire.
    const err = mapNativeError(
      nativeError({
        kind: 'Transient',
        code: 'ConnectionNotReady',
        message: 'connection not ready: retry_after_ms=None',
        retry_after_ms: null,
      }),
    ) as ActrError;

    expect(err.isConnectionNotReady()).toBe(true);
    expect(err.retry_after_ms).toBeNull();
  });

  it('dependency_not_found_carries_service_name', () => {
    const err = mapNativeError(
      nativeError({
        kind: 'Client',
        code: 'DependencyNotFound',
        message: "dependency 'echo' not found: missing",
        service_name: 'echo',
      }),
    ) as ActrError;

    expect(err.code).toBe('DependencyNotFound');
    expect(err.service_name).toBe('echo');
  });

  it('non_dependency_errors_omit_service_name', () => {
    const err = mapNativeError(
      nativeError({
        kind: 'Transient',
        code: 'TimedOut',
        message: 'timed out',
      }),
    ) as ActrError;
    expect(err.service_name).toBeUndefined();
  });
});

// ── Passthrough: non-ACTR errors keep their identity ─────────────────────────

describe('Error mapping — passthrough', () => {
  it('plain_error_without_json_payload_is_unchanged', () => {
    const original = new Error('boom');
    const result = mapNativeError(original);
    expect(result).toBe(original);
    expect(result).not.toBeInstanceOf(ActrError);
  });

  it('non_structured_json_message_is_unchanged', () => {
    // Valid JSON, but not the structured payload shape → left alone.
    const original = new Error('{"hello":"world"}');
    const result = mapNativeError(original);
    expect(result).toBe(original);
    expect(result).not.toBeInstanceOf(ActrError);
  });

  it('non_error_values_pass_through', () => {
    expect(mapNativeError('a string')).toBe('a string');
    expect(mapNativeError(42)).toBe(42);
    expect(mapNativeError(null)).toBeNull();
  });

  it('already_wrapped_actr_error_is_returned_as_is', () => {
    const err = new ActrError({
      kind: 'Client',
      code: 'UnknownRoute',
      message: 'already typed',
    });
    expect(mapNativeError(err)).toBe(err);
  });

  it('original_stack_is_preserved_on_wrap', () => {
    const original = nativeError({
      kind: 'Transient',
      code: 'TimedOut',
      message: 'timed out',
    });
    const stack = original.stack;
    const wrapped = mapNativeError(original) as ActrError;
    expect(wrapped.stack).toBe(stack);
  });
});

// ── callNative: async wrapper re-throws as ActrError ─────────────────────────

describe('Error mapping — callNative wrapper', () => {
  it('rethrows_native_failure_as_actr_error', async () => {
    let caught: unknown = null;
    try {
      await callNative(async () => {
        throw nativeError({
          kind: 'Client',
          code: 'PermissionDenied',
          message: 'permission denied',
        });
      });
    } catch (e) {
      caught = e;
    }

    expect(caught).toBeInstanceOf(ActrError);
    expect((caught as ActrError).code).toBe('PermissionDenied');
  });

  it('passes_through_a_successful_result', async () => {
    const value = await callNative(async () => 'ok');
    expect(value).toBe('ok');
  });
});
