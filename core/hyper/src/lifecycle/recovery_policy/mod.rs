//! RFC-0400 recovery policy translation layer.
//!
//! This module is the pure, side-effect-free policy core described by RFC-0400
//! ("Event-driven connection lifecycle and recovery"). It is deliberately
//! additive: it introduces the normative vocabulary and the reference reducer
//! without touching the pre-RFC [`super::connection_supervisor`] machines, which
//! a later phase reconciles. Nothing here performs I/O, reads an ambient clock,
//! or draws ambient randomness.
//!
//! Layout mirrors the RFC's "Policy translation" section:
//!
//! - [`machines`]: the three YASM state machines the executable reference was
//!   missing (`RecoveryMode`, `CleanupWork`, `RetryGate`);
//! - [`diagnosis`]: the typed effect diagnosis vocabulary, effect kinds, the
//!   producible-diagnosis matrix, and effect outcomes;
//! - [`classification`]: the failure classification table, release-mask
//!   algebra, and backoff arithmetic;
//! - [`translate`]: the single pure `translate(view, input, now, config,
//!   entropy) -> Decision` reducer, the composite action decision, and the
//!   derived send projection.
//!
//! The layer is not yet wired into any production path; the reconciliation
//! phase consumes it. The module-level `allow(dead_code)` records that
//! transitional state honestly rather than inflating the crate's public API.
#![allow(dead_code)]

pub(crate) mod classification;
pub(crate) mod diagnosis;
pub(crate) mod machines;
pub(crate) mod translate;

/// A monotonic or identity value in the account-session / resource families.
///
/// RFC-0400 keeps every generation a plain monotonic counter compared by
/// newer-than; the policy layer never inspects its internal structure.
pub(crate) type Generation = u64;

/// A monotonic instant in the supervisor's clock domain, measured from
/// supervisor start.
///
/// It is kept as a plain [`std::time::Duration`] so the translation layer stays
/// a pure, byte-comparable function with no ambient clock: `now` and every
/// armed deadline are values in this domain, and comparisons are ordinary
/// `Duration` ordering.
pub(crate) type PolicyInstant = std::time::Duration;

/// The causality version of desired policy (`policy_revision` in the RFC).
///
/// Allocated by the supervisor on a material change and compared by `<=` in
/// acknowledgement and supersession.
pub(crate) type Revision = u64;
