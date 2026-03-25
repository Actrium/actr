# Error Handling

Actor-RTC Web should treat errors as structured runtime signals rather than ad hoc console noise.

## Principles

- Preserve actionable context close to the failure site.
- Distinguish transport failures, protocol failures, and application failures.
- Surface browser constraints clearly, especially around workers, WebRTC, and IndexedDB.
- Avoid retry loops that hide root causes or diverge from the rest of the repository.

## Main Error Categories

### Configuration Errors

Examples:

- Invalid signaling URL
- Missing realm or actor identifiers
- Unsupported browser feature flags

These should fail fast during initialization whenever possible.

### Connectivity Errors

Examples:

- WebSocket signaling failures
- ICE negotiation failures
- WebRTC channel closure or setup timeouts

These should include enough transport context to explain whether the failure happened during signaling, candidate exchange, or data-path setup.

### Persistence Errors

Examples:

- IndexedDB open failures
- Store creation failures
- Quota or transaction failures

These should not be silently swallowed because they affect durability and ordering guarantees.

### Protocol Errors

Examples:

- Malformed payloads
- Route-key mismatches
- Decode and encode failures

These should be treated as correctness bugs or wire-compatibility problems, not transient transport issues.

## Reporting Guidance

- Use structured error types in Rust where possible.
- Use clear English log messages for browser-visible errors.
- Include enough identifiers to correlate the failing actor, route, or peer.
- Prefer one high-signal error report over cascades of duplicate warnings.

## Recovery Guidance

- Retry only when the owning layer has a clear policy for doing so.
- Reconnect flows should be explicit and observable.
- Persistent storage corruption or schema mismatch should trigger an intentional reset path rather than undefined behavior.

## Documentation Expectations

Every externally visible failure mode should document:

- what failed
- where it failed
- whether it is retryable
- what the operator or developer should do next
