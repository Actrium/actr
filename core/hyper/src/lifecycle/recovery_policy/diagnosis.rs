//! Typed effect diagnosis vocabulary (RFC-0400 "Typed effect diagnosis").
//!
//! An effect reports *what it observed*, never a verdict. The translation layer
//! is the only place that turns a `(kind, diagnosis, context)` triple into a
//! retry / escalate / park / abandon ruling. Everything here is data.

use std::time::Duration;

use super::Generation;

/// The three diagnosis families defined by RFC-0400.
///
/// Family membership sets the *default* shape of a verdict; context can still
/// escalate an availability-family diagnosis where stronger work is the
/// plausible remedy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DiagnosisFamily {
    /// Inconclusive: the same operation may succeed later with no policy change
    /// (`PathUnreachable`, `Timeout`, `Overloaded`, `ResourceExhausted`).
    Availability,
    /// A verified statement about the current generation or its channels
    /// (`TransportImpaired`, `GenerationDead`).
    Conclusive,
    /// Cannot improve by repeating the same operation (`AuthRejected`,
    /// `ConfigRejected`, `InvariantViolation`).
    Precondition,
}

/// A data-free discriminant for [`EffectDiagnosis`], used by the
/// producible-diagnosis matrix and by classification cells that key on the
/// diagnosis kind rather than its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum DiagnosisTag {
    PathUnreachable,
    Timeout,
    Overloaded,
    ResourceExhausted,
    TransportImpaired,
    GenerationDead,
    AuthRejected,
    ConfigRejected,
    InvariantViolation,
}

impl DiagnosisTag {
    /// The family this diagnosis tag belongs to.
    pub(crate) fn family(self) -> DiagnosisFamily {
        match self {
            Self::PathUnreachable | Self::Timeout | Self::Overloaded | Self::ResourceExhausted => {
                DiagnosisFamily::Availability
            }
            Self::TransportImpaired | Self::GenerationDead => DiagnosisFamily::Conclusive,
            Self::AuthRejected | Self::ConfigRejected | Self::InvariantViolation => {
                DiagnosisFamily::Precondition
            }
        }
    }
}

/// A typed, identity-validated observation reported by an effect.
///
/// The payloads carry only what policy and diagnostics need; the policy layer
/// never inspects opaque detail strings for control flow.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum EffectDiagnosis {
    /// No endpoint answered; nothing is known about the remote session.
    PathUnreachable { stage: String },
    /// A bounded sub-operation exceeded its failure deadline.
    Timeout { stage: String },
    /// The remote explicitly deferred the request; `retry_after` is a floor.
    Overloaded { retry_after: Duration },
    /// Local sockets, memory, or quota were unavailable.
    ResourceExhausted { resource: String },
    /// The session generation is alive; the listed channels are not.
    TransportImpaired { scopes: Vec<String> },
    /// The remote authoritatively no longer knows this generation.
    GenerationDead { generation: Generation },
    /// Credentials expired, invalid, or revoked.
    AuthRejected { kind: String },
    /// Protocol, version, or capability mismatch.
    ConfigRejected { detail: String },
    /// An internal contract was broken.
    InvariantViolation { detail: String },
}

impl EffectDiagnosis {
    /// The data-free discriminant of this diagnosis.
    pub(crate) fn tag(&self) -> DiagnosisTag {
        match self {
            Self::PathUnreachable { .. } => DiagnosisTag::PathUnreachable,
            Self::Timeout { .. } => DiagnosisTag::Timeout,
            Self::Overloaded { .. } => DiagnosisTag::Overloaded,
            Self::ResourceExhausted { .. } => DiagnosisTag::ResourceExhausted,
            Self::TransportImpaired { .. } => DiagnosisTag::TransportImpaired,
            Self::GenerationDead { .. } => DiagnosisTag::GenerationDead,
            Self::AuthRejected { .. } => DiagnosisTag::AuthRejected,
            Self::ConfigRejected { .. } => DiagnosisTag::ConfigRejected,
            Self::InvariantViolation { .. } => DiagnosisTag::InvariantViolation,
        }
    }

    /// The family this diagnosis belongs to.
    pub(crate) fn family(&self) -> DiagnosisFamily {
        self.tag().family()
    }

    /// Whether this diagnosis is in the availability family (inconclusive).
    pub(crate) fn is_availability_family(&self) -> bool {
        self.family() == DiagnosisFamily::Availability
    }

    /// Whether this diagnosis is in the conclusive family.
    pub(crate) fn is_conclusive_family(&self) -> bool {
        self.family() == DiagnosisFamily::Conclusive
    }

    /// Whether this diagnosis is in the precondition family (cannot improve by
    /// repeating the same operation).
    pub(crate) fn is_precondition_family(&self) -> bool {
        self.family() == DiagnosisFamily::Precondition
    }

    /// The `retry_after` floor carried by an `Overloaded` diagnosis, if any.
    ///
    /// The backoff arithmetic applies this floor *after* jitter so jitter can
    /// never move a deadline below it.
    pub(crate) fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Overloaded { retry_after } => Some(*retry_after),
            _ => None,
        }
    }
}

/// The kind of side effect that produced a completion.
///
/// Effect kinds are the classification-table key alongside the diagnosis. The
/// two teardown kinds are grouped in the table because they share one
/// bounded-completion contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EffectKind {
    Probe,
    Restore,
    Reconnect,
    ConfirmedOfflineDisconnect,
    Cleanup,
}

impl EffectKind {
    /// Whether this kind is a teardown effect (confirmed offline disconnect or
    /// cleanup), which shares the bounded-completion contract and the
    /// `Abandon` verdict.
    pub(crate) fn is_teardown(&self) -> bool {
        matches!(self, Self::ConfirmedOfflineDisconnect | Self::Cleanup)
    }

    /// Whether this kind is an intent-bearing recovery effect (probe, restore,
    /// or reconnect), which acknowledges only `RecoveryIntent`.
    pub(crate) fn is_recovery(&self) -> bool {
        matches!(self, Self::Probe | Self::Restore | Self::Reconnect)
    }

    /// Whether this kind can produce the given diagnosis.
    ///
    /// Recovery kinds can produce all nine diagnoses; teardown kinds may only
    /// report the availability family or `InvariantViolation`. A diagnosis
    /// outside the producible set is an effect-contract violation, handled by
    /// the caller as this kind's `InvariantViolation` cell.
    pub(crate) fn can_produce(&self, tag: DiagnosisTag) -> bool {
        if self.is_recovery() {
            return true;
        }
        // Teardown kinds: availability family plus InvariantViolation.
        matches!(
            tag,
            DiagnosisTag::PathUnreachable
                | DiagnosisTag::Timeout
                | DiagnosisTag::Overloaded
                | DiagnosisTag::ResourceExhausted
                | DiagnosisTag::InvariantViolation
        )
    }
}

/// Why an effect task aborted, per RFC-0400 completion procedure step 6.
///
/// The cause, not a blanket class, decides policy: a recorded cancellation is
/// handled as an ordinary supervisor cancellation, a panic or contract
/// violation is ruled as `InvariantViolation`, a runtime shutdown takes the
/// terminating bounded-teardown path, and an unclassified abort is an
/// implementation failure recorded as an ERROR.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum AbortCause {
    /// A recorded supervisor cancellation request (handled as step 5).
    SupervisorCancellation,
    /// A panic or effect-contract violation, ruled as `InvariantViolation`.
    PanicOrContractViolation,
    /// A runtime shutdown; takes the `Terminating` bounded-teardown path.
    RuntimeShutdown,
    /// An unclassified abort: an implementation failure, recorded as an ERROR
    /// and handled as this kind's `InvariantViolation` cell.
    Unclassified,
}

/// The outcome an effect reports through `EffectCompleted`.
///
/// `CompletedWithResiduals` and `Abandoned` are valid only for teardown kinds
/// and are success-class: both extinguish the obligation while reporting
/// residual diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum EffectOutcome {
    /// The effect met its full contract.
    Succeeded,
    /// Teardown reached its logical goal but left residual diagnostics.
    CompletedWithResiduals { residuals: Vec<EffectDiagnosis> },
    /// Teardown was abandoned at its overall deadline before the goal, with
    /// recorded residual diagnostics; still success-class for extinguishment.
    Abandoned { residuals: Vec<EffectDiagnosis> },
    /// The effect failed with a typed diagnosis (never a verdict).
    Failed { diagnosis: EffectDiagnosis },
    /// The effect was cancelled by a recorded supervisor request.
    Cancelled,
    /// The effect task aborted; ruled by cause.
    Aborted { cause: AbortCause },
}

impl EffectOutcome {
    /// Whether this outcome is success-class (`Succeeded`,
    /// `CompletedWithResiduals`, or bounded `Abandoned`), i.e. it extinguishes
    /// the obligation.
    pub(crate) fn is_success_class(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::CompletedWithResiduals { .. } | Self::Abandoned { .. }
        )
    }

    /// The residual diagnostics recorded by a teardown success-class outcome.
    pub(crate) fn residuals(&self) -> &[EffectDiagnosis] {
        match self {
            Self::CompletedWithResiduals { residuals } | Self::Abandoned { residuals } => residuals,
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_diagnoses() -> Vec<EffectDiagnosis> {
        vec![
            EffectDiagnosis::PathUnreachable {
                stage: "connect".into(),
            },
            EffectDiagnosis::Timeout {
                stage: "handshake".into(),
            },
            EffectDiagnosis::Overloaded {
                retry_after: Duration::from_secs(3),
            },
            EffectDiagnosis::ResourceExhausted {
                resource: "sockets".into(),
            },
            EffectDiagnosis::TransportImpaired {
                scopes: vec!["data".into()],
            },
            EffectDiagnosis::GenerationDead { generation: 7 },
            EffectDiagnosis::AuthRejected {
                kind: "expired".into(),
            },
            EffectDiagnosis::ConfigRejected {
                detail: "version".into(),
            },
            EffectDiagnosis::InvariantViolation {
                detail: "broken".into(),
            },
        ]
    }

    #[test]
    fn families_partition_all_nine_diagnoses() {
        let (mut avail, mut concl, mut precond) = (0, 0, 0);
        for d in all_diagnoses() {
            match d.family() {
                DiagnosisFamily::Availability => {
                    assert!(d.is_availability_family());
                    avail += 1;
                }
                DiagnosisFamily::Conclusive => {
                    assert!(d.is_conclusive_family());
                    concl += 1;
                }
                DiagnosisFamily::Precondition => {
                    assert!(d.is_precondition_family());
                    precond += 1;
                }
            }
        }
        assert_eq!((avail, concl, precond), (4, 2, 3));
    }

    #[test]
    fn recovery_kinds_produce_every_diagnosis() {
        for kind in [
            EffectKind::Probe,
            EffectKind::Restore,
            EffectKind::Reconnect,
        ] {
            assert!(kind.is_recovery());
            assert!(!kind.is_teardown());
            for d in all_diagnoses() {
                assert!(kind.can_produce(d.tag()), "{kind:?} must produce {d:?}");
            }
        }
    }

    #[test]
    fn teardown_kinds_only_produce_availability_and_invariant_violation() {
        for kind in [EffectKind::Cleanup, EffectKind::ConfirmedOfflineDisconnect] {
            assert!(kind.is_teardown());
            for d in all_diagnoses() {
                let expected =
                    d.is_availability_family() || d.tag() == DiagnosisTag::InvariantViolation;
                assert_eq!(
                    kind.can_produce(d.tag()),
                    expected,
                    "{kind:?} producibility of {d:?}"
                );
            }
        }
    }

    #[test]
    fn retry_after_is_carried_only_by_overloaded() {
        for d in all_diagnoses() {
            match d {
                EffectDiagnosis::Overloaded { retry_after } => {
                    assert_eq!(d.retry_after(), Some(retry_after));
                }
                other => assert_eq!(other.retry_after(), None),
            }
        }
    }

    #[test]
    fn success_class_outcomes_and_residuals() {
        let residual = vec![EffectDiagnosis::Timeout {
            stage: "remote-notify".into(),
        }];
        assert!(EffectOutcome::Succeeded.is_success_class());
        assert!(
            EffectOutcome::CompletedWithResiduals {
                residuals: residual.clone(),
            }
            .is_success_class()
        );
        let abandoned = EffectOutcome::Abandoned {
            residuals: residual.clone(),
        };
        assert!(abandoned.is_success_class());
        assert_eq!(abandoned.residuals(), residual.as_slice());

        assert!(!EffectOutcome::Cancelled.is_success_class());
        assert!(
            !EffectOutcome::Failed {
                diagnosis: EffectDiagnosis::GenerationDead { generation: 1 },
            }
            .is_success_class()
        );
        assert!(
            !EffectOutcome::Aborted {
                cause: AbortCause::RuntimeShutdown,
            }
            .is_success_class()
        );
        assert_eq!(EffectOutcome::Succeeded.residuals(), &[]);
    }
}
