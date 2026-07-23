//! Failure classification and backoff arithmetic (RFC-0400 "Failure
//! classification and escalation" and "Backoff arithmetic").
//!
//! Classification maps a `(effect kind, diagnosis, context)` triple to exactly
//! one [`Verdict`]. It is the only place that decides retry, escalation, park,
//! or abandon; effects never classify their own failures. The backoff
//! arithmetic computes one absolute deadline from injected deterministic
//! entropy so that tests are byte-reproducible.

use std::collections::BTreeSet;
use std::time::Duration;

use super::PolicyInstant;
use super::diagnosis::{DiagnosisFamily, DiagnosisTag, EffectDiagnosis, EffectKind};

/// A recorded park-cause entry.
///
/// Each entry lists (via [`ParkCause::clearing_triggers`]) the trigger classes
/// that clear it. A parked record's [`ReleaseMask`] holds one entry per cause
/// and is released only when the set empties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ParkCause {
    AuthRejected,
    ConfigRejected,
    InvariantViolation,
    ExplicitPause,
}

/// A class of external trigger that may clear park-cause entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum TriggerClass {
    /// A newly committed session generation.
    SessionActivated,
    /// A configuration change in scope.
    ConfigurationChanged,
    /// An explicit recovery command (`RecoveryRequested`).
    ExplicitCommand,
    /// A matching explicit resume (`RecoveryResumeRequested`).
    ExplicitResume,
}

impl ParkCause {
    /// The trigger classes that clear this park cause.
    ///
    /// A new route or `SignalingGenerationCommitted` is deliberately absent
    /// from every precondition entry: it can never clear an auth, config, or
    /// invariant park. An explicit resume clears only an explicit pause.
    pub(crate) fn clearing_triggers(self) -> &'static [TriggerClass] {
        match self {
            Self::AuthRejected => &[
                TriggerClass::SessionActivated,
                TriggerClass::ExplicitCommand,
            ],
            Self::ConfigRejected => &[
                TriggerClass::ConfigurationChanged,
                TriggerClass::ExplicitCommand,
            ],
            Self::InvariantViolation => &[TriggerClass::ExplicitCommand],
            Self::ExplicitPause => &[TriggerClass::ExplicitResume],
        }
    }

    /// Whether the given trigger clears this park cause.
    pub(crate) fn cleared_by(self, trigger: TriggerClass) -> bool {
        self.clearing_triggers().contains(&trigger)
    }

    /// The park cause implied by a precondition-family diagnosis tag.
    fn from_precondition_tag(tag: DiagnosisTag) -> Self {
        match tag {
            DiagnosisTag::AuthRejected => Self::AuthRejected,
            DiagnosisTag::ConfigRejected => Self::ConfigRejected,
            // InvariantViolation and any tag defensively routed here.
            _ => Self::InvariantViolation,
        }
    }
}

/// The non-empty set of park-cause entries recorded while a record is parked.
///
/// Satisfaction is *sticky*: a trigger clears every entry it matches, and the
/// record is freed only when no entries remain. A resume alone therefore cannot
/// free a record whose precondition entry survives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ReleaseMask {
    causes: BTreeSet<ParkCause>,
}

impl ReleaseMask {
    /// A mask holding exactly one park cause.
    pub(crate) fn single(cause: ParkCause) -> Self {
        let mut causes = BTreeSet::new();
        causes.insert(cause);
        Self { causes }
    }

    /// Whether the mask is empty (the record may be released).
    pub(crate) fn is_empty(&self) -> bool {
        self.causes.is_empty()
    }

    /// Whether the mask holds the given cause.
    pub(crate) fn contains(&self, cause: ParkCause) -> bool {
        self.causes.contains(&cause)
    }

    /// Record one more park cause (union of a single entry).
    pub(crate) fn insert(&mut self, cause: ParkCause) {
        self.causes.insert(cause);
    }

    /// Union another mask into this one.
    pub(crate) fn union_with(&mut self, other: &ReleaseMask) {
        for cause in &other.causes {
            self.causes.insert(*cause);
        }
    }

    /// Remove every entry the trigger clears; return whether any were removed.
    ///
    /// Sticky semantics: entries the trigger does not match are retained, so
    /// the caller releases the record only when [`ReleaseMask::is_empty`] then
    /// holds.
    pub(crate) fn clear_matching(&mut self, trigger: TriggerClass) -> bool {
        let before = self.causes.len();
        self.causes.retain(|cause| !cause.cleared_by(trigger));
        self.causes.len() != before
    }

    /// Whether the trigger would clear at least one currently-held entry,
    /// without mutating the mask.
    pub(crate) fn would_clear(&self, trigger: TriggerClass) -> bool {
        self.causes.iter().any(|cause| cause.cleared_by(trigger))
    }

    /// Iterate the recorded park causes in a deterministic order.
    pub(crate) fn causes(&self) -> impl Iterator<Item = ParkCause> + '_ {
        self.causes.iter().copied()
    }
}

/// The translation layer's ruling on one `(effect kind, diagnosis, context)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// `RetryableFailure`: back off toward one capped, jittered deadline.
    Retry,
    /// Promote the pending intent to a stronger kind via its normative
    /// transition, retaining the causal work revision.
    Escalate { to: EffectKind },
    /// `TerminalFailure`: park and record the release mask.
    Park { release_mask: ReleaseMask },
    /// Teardown kinds only: extinguish the obligation now with `Abandoned`
    /// residuals.
    Abandon,
}

/// The context a classification cell consults beyond `(kind, diagnosis)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ClassifyContext {
    /// Consecutive failures of the current record, starting at 1.
    pub attempt: u32,
    /// Whether the committed path is `Online` (the "while path stays Online"
    /// escalation context).
    pub path_online: bool,
    /// Whether recovery mode is `Terminating` (teardown invariant row).
    pub mode_terminating: bool,
    /// Whether the completion is processed at or after the obligation's overall
    /// teardown deadline.
    pub past_teardown_deadline: bool,
    /// The escalation threshold for probe timeouts.
    pub escalate_after_probe: u32,
    /// The escalation threshold for restore timeouts and transport impairment
    /// (documented default 2).
    pub escalate_after_restore: u32,
}

impl ClassifyContext {
    /// A context with the RFC's documented default thresholds and benign facts.
    pub(crate) fn with_defaults(attempt: u32) -> Self {
        Self {
            attempt,
            path_online: true,
            mode_terminating: false,
            past_teardown_deadline: false,
            escalate_after_probe: DEFAULT_ESCALATE_AFTER_PROBE,
            escalate_after_restore: DEFAULT_ESCALATE_AFTER_RESTORE,
        }
    }
}

/// Documented default escalation threshold for probe timeouts.
pub(crate) const DEFAULT_ESCALATE_AFTER_PROBE: u32 = 2;
/// Documented default escalation threshold for restore timeouts and transport
/// impairment (`escalate_after = 2`).
pub(crate) const DEFAULT_ESCALATE_AFTER_RESTORE: u32 = 2;

/// Map `(effect kind, diagnosis, context)` to exactly one verdict.
///
/// Within each effect-kind group the classification rows are evaluated top to
/// bottom and the first match applies. A diagnosis the kind cannot produce is
/// an effect-contract violation handled as this kind's `InvariantViolation`
/// cell.
pub(crate) fn classify(
    kind: EffectKind,
    diagnosis: &EffectDiagnosis,
    ctx: ClassifyContext,
) -> Verdict {
    // Effect-contract violation: a non-producible diagnosis is ruled as this
    // kind's InvariantViolation cell rather than under a family default.
    let tag = if kind.can_produce(diagnosis.tag()) {
        diagnosis.tag()
    } else {
        DiagnosisTag::InvariantViolation
    };
    let family = tag.family();

    match kind {
        EffectKind::Probe => classify_probe(tag, family, ctx),
        EffectKind::Restore => classify_restore(tag, family, ctx),
        EffectKind::Reconnect => classify_reconnect(family, tag),
        EffectKind::ConfirmedOfflineDisconnect | EffectKind::Cleanup => {
            classify_teardown(tag, family, ctx)
        }
    }
}

fn park_for(tag: DiagnosisTag) -> Verdict {
    Verdict::Park {
        release_mask: ReleaseMask::single(ParkCause::from_precondition_tag(tag)),
    }
}

fn classify_probe(tag: DiagnosisTag, family: DiagnosisFamily, ctx: ClassifyContext) -> Verdict {
    // | Probe | TransportImpaired | Escalate { Restore } |
    if tag == DiagnosisTag::TransportImpaired {
        return Verdict::Escalate {
            to: EffectKind::Restore,
        };
    }
    // | Probe | GenerationDead | Escalate { Reconnect } |
    if tag == DiagnosisTag::GenerationDead {
        return Verdict::Escalate {
            to: EffectKind::Reconnect,
        };
    }
    // | Probe | Timeout | >= escalate_after(Probe) while Online | Escalate { Reconnect } |
    if tag == DiagnosisTag::Timeout && ctx.path_online && ctx.attempt >= ctx.escalate_after_probe {
        return Verdict::Escalate {
            to: EffectKind::Reconnect,
        };
    }
    // | Probe | precondition family | Park, per-diagnosis mask |
    if family == DiagnosisFamily::Precondition {
        return park_for(tag);
    }
    // | Probe | availability family | Retry |
    Verdict::Retry
}

fn classify_restore(tag: DiagnosisTag, family: DiagnosisFamily, ctx: ClassifyContext) -> Verdict {
    // | Restore | GenerationDead | Escalate { Reconnect } |
    if tag == DiagnosisTag::GenerationDead {
        return Verdict::Escalate {
            to: EffectKind::Reconnect,
        };
    }
    // | Restore | TransportImpaired | >= escalate_after = 2 | Escalate { Reconnect } |
    if tag == DiagnosisTag::TransportImpaired && ctx.attempt >= ctx.escalate_after_restore {
        return Verdict::Escalate {
            to: EffectKind::Reconnect,
        };
    }
    // | Restore | Timeout | >= escalate_after(Restore) while Online | Escalate { Reconnect } |
    if tag == DiagnosisTag::Timeout && ctx.path_online && ctx.attempt >= ctx.escalate_after_restore
    {
        return Verdict::Escalate {
            to: EffectKind::Reconnect,
        };
    }
    // | Restore | precondition family | Park, per-diagnosis mask |
    if family == DiagnosisFamily::Precondition {
        return park_for(tag);
    }
    // | Restore | TransportImpaired or availability family | Retry |
    Verdict::Retry
}

fn classify_reconnect(family: DiagnosisFamily, tag: DiagnosisTag) -> Verdict {
    // | Reconnect | precondition family | Park, per-diagnosis mask |
    if family == DiagnosisFamily::Precondition {
        return park_for(tag);
    }
    // | Reconnect | availability and conclusive families | Retry, sustained cap |
    Verdict::Retry
}

fn classify_teardown(tag: DiagnosisTag, family: DiagnosisFamily, ctx: ClassifyContext) -> Verdict {
    // | Disconnect, Cleanup | InvariantViolation | mode Terminating | Abandon |
    if tag == DiagnosisTag::InvariantViolation && ctx.mode_terminating {
        return Verdict::Abandon;
    }
    // | Disconnect, Cleanup | InvariantViolation | otherwise | Park { ExplicitCommand } |
    if tag == DiagnosisTag::InvariantViolation {
        return Verdict::Park {
            release_mask: ReleaseMask::single(ParkCause::InvariantViolation),
        };
    }
    // | Disconnect, Cleanup | availability family | at/after teardown deadline | Abandon |
    if family == DiagnosisFamily::Availability && ctx.past_teardown_deadline {
        return Verdict::Abandon;
    }
    // | Disconnect, Cleanup | availability family | inside deadline | Retry, short backoff |
    if family == DiagnosisFamily::Availability {
        return Verdict::Retry;
    }
    // Teardown kinds cannot produce conclusive or non-invariant precondition
    // diagnoses; a contract violation was already substituted above. Fail
    // closed on anything that reaches here.
    Verdict::Park {
        release_mask: ReleaseMask::single(ParkCause::InvariantViolation),
    }
}

/// A deterministic source of jitter entropy.
///
/// The stream is part of supervisor state and is seeded per supervisor instance
/// so that clients and sessions de-correlate. A duplicate or stale input
/// consumes no entropy and MUST NOT resample.
pub(crate) trait EntropySource {
    /// Draw one jitter sample uniformly in `[0.0, 1.0)`. Consumed exactly once
    /// per newly armed deadline.
    fn next_unit(&mut self) -> f64;
}

/// Per-kind backoff curve parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BackoffParams {
    /// The delay for the first failure (`attempt == 1`).
    pub base_delay: Duration,
    /// The delay ceiling; sustained kinds retry here indefinitely.
    pub max_delay: Duration,
    /// The exponent ceiling; `2^exponent` saturates at `2^exponent_cap`.
    pub exponent_cap: u32,
    /// The jitter span: the sampled factor lies in `[1 - jitter_fraction, 1]`.
    pub jitter_fraction: f64,
}

/// Compute one absolute retry deadline from the backoff curve.
///
/// ```text
/// exponent = min(attempt - 1, exponent_cap)          saturating
/// delay    = min(base_delay * 2^exponent, max_delay)
/// jitter   = 1 - jitter_fraction * u,  u in [0, 1)   -> (1 - jitter_fraction, 1]
/// deadline = now + max(delay * jitter, retry_after)
/// ```
///
/// The `retry_after` floor is applied *after* jitter, so jitter can never move
/// the deadline below it. Entropy is drawn exactly once.
pub(crate) fn arm_backoff(
    now: PolicyInstant,
    attempt: u32,
    params: &BackoffParams,
    retry_after: Duration,
    entropy: &mut dyn EntropySource,
) -> PolicyInstant {
    let exponent = attempt.saturating_sub(1).min(params.exponent_cap);
    let factor = 1u32.checked_shl(exponent).unwrap_or(u32::MAX);
    let scaled = params.base_delay.saturating_mul(factor);
    let delay = scaled.min(params.max_delay);

    let unit = entropy.next_unit().clamp(0.0, 1.0);
    let jitter = 1.0 - params.jitter_fraction.clamp(0.0, 1.0) * unit;
    let jittered = delay.mul_f64(jitter);

    // Saturate: `retry_after` arrives from the wire, so `now + floor` must cap
    // at the far future instead of panicking on `Duration` overflow.
    now.saturating_add(jittered.max(retry_after))
}

/// The production jitter source: a self-contained SplitMix64 stream.
///
/// It draws no ambient randomness of its own; the supervisor seeds it once per
/// instance so that clients and sessions de-correlate. A duplicate or stale
/// input consumes no entropy because [`arm_backoff`] is the only caller.
#[derive(Debug, Clone)]
pub(crate) struct DefaultEntropy {
    state: u64,
}

impl DefaultEntropy {
    /// Seed the stream. Any non-zero seed produces a full-period stream; a zero
    /// seed is nudged so the first draw is still well distributed.
    pub(crate) fn seeded(seed: u64) -> Self {
        Self {
            state: seed ^ 0x9E37_79B9_7F4A_7C15,
        }
    }
}

impl EntropySource for DefaultEntropy {
    fn next_unit(&mut self) -> f64 {
        // SplitMix64: one wrapping add plus two avalanche multiplies.
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // 53-bit mantissa in [0, 1).
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
pub(crate) struct FixedEntropy {
    samples: Vec<f64>,
    index: usize,
}

#[cfg(test)]
impl FixedEntropy {
    pub(crate) fn new(samples: Vec<f64>) -> Self {
        assert!(!samples.is_empty(), "entropy needs at least one sample");
        Self { samples, index: 0 }
    }

    pub(crate) fn constant(value: f64) -> Self {
        Self::new(vec![value])
    }

    /// How many samples have been drawn, for asserting that duplicates or stale
    /// inputs consumed no entropy.
    pub(crate) fn drawn(&self) -> usize {
        self.index
    }
}

#[cfg(test)]
impl EntropySource for FixedEntropy {
    fn next_unit(&mut self) -> f64 {
        let value = self.samples[self.index % self.samples.len()];
        self.index += 1;
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_ctx(attempt: u32) -> ClassifyContext {
        ClassifyContext::with_defaults(attempt)
    }

    fn diag(tag: DiagnosisTag) -> EffectDiagnosis {
        match tag {
            DiagnosisTag::PathUnreachable => EffectDiagnosis::PathUnreachable { stage: "s".into() },
            DiagnosisTag::Timeout => EffectDiagnosis::Timeout { stage: "s".into() },
            DiagnosisTag::Overloaded => EffectDiagnosis::Overloaded {
                retry_after: Duration::ZERO,
            },
            DiagnosisTag::ResourceExhausted => EffectDiagnosis::ResourceExhausted {
                resource: "s".into(),
            },
            DiagnosisTag::TransportImpaired => EffectDiagnosis::TransportImpaired {
                scopes: vec!["c".into()],
            },
            DiagnosisTag::GenerationDead => EffectDiagnosis::GenerationDead { generation: 1 },
            DiagnosisTag::AuthRejected => EffectDiagnosis::AuthRejected { kind: "s".into() },
            DiagnosisTag::ConfigRejected => EffectDiagnosis::ConfigRejected { detail: "s".into() },
            DiagnosisTag::InvariantViolation => {
                EffectDiagnosis::InvariantViolation { detail: "s".into() }
            }
        }
    }

    // --- Probe group: one test per classification cell ---

    #[test]
    fn probe_transport_impaired_escalates_to_restore() {
        assert_eq!(
            classify(
                EffectKind::Probe,
                &diag(DiagnosisTag::TransportImpaired),
                probe_ctx(1)
            ),
            Verdict::Escalate {
                to: EffectKind::Restore
            }
        );
    }

    #[test]
    fn probe_generation_dead_escalates_to_reconnect() {
        assert_eq!(
            classify(
                EffectKind::Probe,
                &diag(DiagnosisTag::GenerationDead),
                probe_ctx(1)
            ),
            Verdict::Escalate {
                to: EffectKind::Reconnect
            }
        );
    }

    #[test]
    fn probe_timeout_escalates_only_at_threshold_while_online() {
        // Below threshold -> Retry.
        assert_eq!(
            classify(
                EffectKind::Probe,
                &diag(DiagnosisTag::Timeout),
                probe_ctx(1)
            ),
            Verdict::Retry
        );
        // At threshold while online -> Escalate to Reconnect.
        assert_eq!(
            classify(
                EffectKind::Probe,
                &diag(DiagnosisTag::Timeout),
                probe_ctx(2)
            ),
            Verdict::Escalate {
                to: EffectKind::Reconnect
            }
        );
        // At threshold but offline -> Retry (no escalation without Online).
        let mut offline = probe_ctx(3);
        offline.path_online = false;
        assert_eq!(
            classify(EffectKind::Probe, &diag(DiagnosisTag::Timeout), offline),
            Verdict::Retry
        );
    }

    #[test]
    fn probe_precondition_parks_with_matching_mask() {
        for (tag, cause) in [
            (DiagnosisTag::AuthRejected, ParkCause::AuthRejected),
            (DiagnosisTag::ConfigRejected, ParkCause::ConfigRejected),
            (
                DiagnosisTag::InvariantViolation,
                ParkCause::InvariantViolation,
            ),
        ] {
            assert_eq!(
                classify(EffectKind::Probe, &diag(tag), probe_ctx(1)),
                Verdict::Park {
                    release_mask: ReleaseMask::single(cause)
                }
            );
        }
    }

    #[test]
    fn probe_availability_retries() {
        for tag in [
            DiagnosisTag::PathUnreachable,
            DiagnosisTag::Overloaded,
            DiagnosisTag::ResourceExhausted,
        ] {
            assert_eq!(
                classify(EffectKind::Probe, &diag(tag), probe_ctx(9)),
                Verdict::Retry
            );
        }
    }

    // --- Restore group ---

    #[test]
    fn restore_generation_dead_escalates() {
        assert_eq!(
            classify(
                EffectKind::Restore,
                &diag(DiagnosisTag::GenerationDead),
                probe_ctx(1)
            ),
            Verdict::Escalate {
                to: EffectKind::Reconnect
            }
        );
    }

    #[test]
    fn restore_transport_impaired_retries_then_escalates_at_two() {
        assert_eq!(
            classify(
                EffectKind::Restore,
                &diag(DiagnosisTag::TransportImpaired),
                probe_ctx(1)
            ),
            Verdict::Retry
        );
        assert_eq!(
            classify(
                EffectKind::Restore,
                &diag(DiagnosisTag::TransportImpaired),
                probe_ctx(2)
            ),
            Verdict::Escalate {
                to: EffectKind::Reconnect
            }
        );
    }

    #[test]
    fn restore_timeout_escalates_at_threshold_online() {
        assert_eq!(
            classify(
                EffectKind::Restore,
                &diag(DiagnosisTag::Timeout),
                probe_ctx(1)
            ),
            Verdict::Retry
        );
        assert_eq!(
            classify(
                EffectKind::Restore,
                &diag(DiagnosisTag::Timeout),
                probe_ctx(2)
            ),
            Verdict::Escalate {
                to: EffectKind::Reconnect
            }
        );
    }

    #[test]
    fn restore_precondition_parks() {
        assert_eq!(
            classify(
                EffectKind::Restore,
                &diag(DiagnosisTag::AuthRejected),
                probe_ctx(1)
            ),
            Verdict::Park {
                release_mask: ReleaseMask::single(ParkCause::AuthRejected)
            }
        );
    }

    #[test]
    fn restore_availability_retries() {
        assert_eq!(
            classify(
                EffectKind::Restore,
                &diag(DiagnosisTag::PathUnreachable),
                probe_ctx(5)
            ),
            Verdict::Retry
        );
    }

    // --- Reconnect group ---

    #[test]
    fn reconnect_precondition_parks_but_others_sustain_backoff() {
        assert_eq!(
            classify(
                EffectKind::Reconnect,
                &diag(DiagnosisTag::ConfigRejected),
                probe_ctx(1)
            ),
            Verdict::Park {
                release_mask: ReleaseMask::single(ParkCause::ConfigRejected)
            }
        );
        for tag in [
            DiagnosisTag::PathUnreachable,
            DiagnosisTag::Timeout,
            DiagnosisTag::Overloaded,
            DiagnosisTag::ResourceExhausted,
            DiagnosisTag::TransportImpaired,
            DiagnosisTag::GenerationDead,
        ] {
            assert_eq!(
                classify(EffectKind::Reconnect, &diag(tag), probe_ctx(100)),
                Verdict::Retry,
                "reconnect never parks {tag:?}"
            );
        }
    }

    // --- Teardown group (cleanup and confirmed offline disconnect) ---

    #[test]
    fn teardown_invariant_violation_parks_or_abandons_by_mode() {
        for kind in [EffectKind::Cleanup, EffectKind::ConfirmedOfflineDisconnect] {
            let mut ctx = probe_ctx(1);
            ctx.mode_terminating = false;
            assert_eq!(
                classify(kind, &diag(DiagnosisTag::InvariantViolation), ctx),
                Verdict::Park {
                    release_mask: ReleaseMask::single(ParkCause::InvariantViolation)
                }
            );
            ctx.mode_terminating = true;
            assert_eq!(
                classify(kind, &diag(DiagnosisTag::InvariantViolation), ctx),
                Verdict::Abandon
            );
        }
    }

    #[test]
    fn teardown_availability_retries_inside_deadline_and_abandons_after() {
        for kind in [EffectKind::Cleanup, EffectKind::ConfirmedOfflineDisconnect] {
            let mut ctx = probe_ctx(3);
            ctx.past_teardown_deadline = false;
            assert_eq!(
                classify(kind, &diag(DiagnosisTag::Timeout), ctx),
                Verdict::Retry
            );
            ctx.past_teardown_deadline = true;
            assert_eq!(
                classify(kind, &diag(DiagnosisTag::Timeout), ctx),
                Verdict::Abandon
            );
        }
    }

    #[test]
    fn teardown_non_producible_diagnosis_is_ruled_as_invariant_violation() {
        // GenerationDead is not producible by a teardown effect; it must be
        // handled as the InvariantViolation cell (fail-closed park when active).
        assert_eq!(
            classify(
                EffectKind::Cleanup,
                &diag(DiagnosisTag::GenerationDead),
                probe_ctx(1)
            ),
            Verdict::Park {
                release_mask: ReleaseMask::single(ParkCause::InvariantViolation)
            }
        );
    }

    // --- Release mask algebra ---

    #[test]
    fn mask_release_is_sticky_and_empties_only_when_all_cleared() {
        let mut mask = ReleaseMask::single(ParkCause::AuthRejected);
        mask.insert(ParkCause::ExplicitPause);
        assert!(!mask.is_empty());

        // A resume clears only the explicit-pause entry; auth survives.
        assert!(mask.clear_matching(TriggerClass::ExplicitResume));
        assert!(mask.contains(ParkCause::AuthRejected));
        assert!(!mask.contains(ParkCause::ExplicitPause));
        assert!(!mask.is_empty());

        // A config change matches nothing here.
        assert!(!mask.clear_matching(TriggerClass::ConfigurationChanged));
        assert!(!mask.is_empty());

        // SessionActivated clears the auth entry; the mask now empties.
        assert!(mask.clear_matching(TriggerClass::SessionActivated));
        assert!(mask.is_empty());
    }

    #[test]
    fn resume_alone_cannot_free_a_precondition_park() {
        let mut mask = ReleaseMask::single(ParkCause::InvariantViolation);
        assert!(!mask.would_clear(TriggerClass::ExplicitResume));
        assert!(!mask.clear_matching(TriggerClass::ExplicitResume));
        assert!(!mask.is_empty());
        // Only an explicit command releases an invariant park.
        assert!(mask.would_clear(TriggerClass::ExplicitCommand));
        assert!(mask.clear_matching(TriggerClass::ExplicitCommand));
        assert!(mask.is_empty());
    }

    #[test]
    fn explicit_command_does_not_clear_explicit_pause() {
        let mut mask = ReleaseMask::single(ParkCause::ExplicitPause);
        assert!(!mask.would_clear(TriggerClass::ExplicitCommand));
        assert!(!mask.clear_matching(TriggerClass::ExplicitCommand));
        assert!(!mask.is_empty());
    }

    #[test]
    fn config_and_session_triggers_do_not_cross_clear() {
        let mut mask = ReleaseMask::single(ParkCause::AuthRejected);
        mask.insert(ParkCause::ConfigRejected);
        // A config change clears only config; a session clears only auth.
        assert!(mask.clear_matching(TriggerClass::ConfigurationChanged));
        assert!(mask.contains(ParkCause::AuthRejected));
        assert!(mask.clear_matching(TriggerClass::SessionActivated));
        assert!(mask.is_empty());
    }

    // --- Backoff arithmetic ---

    fn params() -> BackoffParams {
        BackoffParams {
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(10),
            exponent_cap: 6,
            jitter_fraction: 0.2,
        }
    }

    #[test]
    fn backoff_grows_exponentially_then_caps_at_exponent_and_max() {
        let mut e = FixedEntropy::constant(0.0); // jitter factor == 1.0
        let now = Duration::ZERO;
        let p = params();
        // attempt 1 -> 100ms, 2 -> 200ms, 3 -> 400ms.
        assert_eq!(
            arm_backoff(now, 1, &p, Duration::ZERO, &mut e),
            Duration::from_millis(100)
        );
        assert_eq!(
            arm_backoff(now, 2, &p, Duration::ZERO, &mut e),
            Duration::from_millis(200)
        );
        assert_eq!(
            arm_backoff(now, 3, &p, Duration::ZERO, &mut e),
            Duration::from_millis(400)
        );
        // The exponent saturates at cap = 6, so the curve settles at
        // 100ms * 2^6 = 6.4s and never grows past it (still under max_delay).
        assert_eq!(
            arm_backoff(now, 7, &p, Duration::ZERO, &mut e),
            Duration::from_millis(6400)
        );
        assert_eq!(
            arm_backoff(now, 1_000, &p, Duration::ZERO, &mut e),
            Duration::from_millis(6400)
        );
        // max_delay clamps when base * 2^exponent would exceed it: 5s * 2^2 =
        // 20s, clamped to the 8s ceiling.
        let clamped = BackoffParams {
            base_delay: Duration::from_secs(5),
            max_delay: Duration::from_secs(8),
            exponent_cap: 6,
            jitter_fraction: 0.2,
        };
        assert_eq!(
            arm_backoff(now, 3, &clamped, Duration::ZERO, &mut e),
            Duration::from_secs(8)
        );
    }

    #[test]
    fn jitter_scales_within_configured_span() {
        let now = Duration::ZERO;
        let p = params();
        // u == 1.0 (approached) -> factor 1 - 0.2 == 0.8 of the 100ms base.
        let mut e = FixedEntropy::constant(1.0);
        assert_eq!(
            arm_backoff(now, 1, &p, Duration::ZERO, &mut e),
            Duration::from_millis(80)
        );
    }

    #[test]
    fn retry_after_floor_is_applied_after_jitter() {
        let now = Duration::ZERO;
        let p = params();
        // Jitter would drop the 100ms base to 80ms, but the 5s floor wins and
        // is honored even though it exceeds the jittered delay.
        let mut e = FixedEntropy::constant(1.0);
        let deadline = arm_backoff(now, 1, &p, Duration::from_secs(5), &mut e);
        assert_eq!(deadline, Duration::from_secs(5));
    }

    #[test]
    fn retry_after_is_honored_even_above_max_delay() {
        let now = Duration::ZERO;
        let p = params();
        let mut e = FixedEntropy::constant(0.0);
        let deadline = arm_backoff(now, 1, &p, Duration::from_secs(30), &mut e);
        assert_eq!(deadline, Duration::from_secs(30));
    }

    #[test]
    fn each_armed_deadline_draws_exactly_one_sample() {
        let now = Duration::ZERO;
        let p = params();
        let mut e = FixedEntropy::new(vec![0.0, 0.5, 1.0]);
        let _ = arm_backoff(now, 1, &p, Duration::ZERO, &mut e);
        let _ = arm_backoff(now, 2, &p, Duration::ZERO, &mut e);
        assert_eq!(e.drawn(), 2);
    }

    #[test]
    fn extreme_retry_after_saturates_instead_of_panicking() {
        // A hostile or corrupt `retry_after` must never panic the policy
        // engine; the deadline saturates at the far future.
        let now = Duration::from_secs(1);
        let p = params();
        let mut e = FixedEntropy::constant(0.0);
        let deadline = arm_backoff(now, 1, &p, Duration::MAX, &mut e);
        assert_eq!(deadline, Duration::MAX);
    }
}
