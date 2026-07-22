//! Optional hybrid logical clock (HLC) library for actr, implementing
//! RFC-0419 §4.
//!
//! This crate is standalone: it has no dependency on the actr protocol and
//! never stamps any envelope automatically. A protocol that opts in defines
//! where timestamps live in its payload, which protocol points map to local,
//! send and receive events, and how clock state is persisted (RFC-0419 §5).
//!
//! # Naming
//!
//! Following the RFC naming rule (bare names inside a dedicated `hlc`
//! crate/module), the sketch names used in the RFC map to this crate as:
//!
//! | RFC sketch        | This crate    |
//! |-------------------|---------------|
//! | `HybridTimestamp` | [`Timestamp`] |
//! | `HlcState`        | [`State`]     |
//! | `HybridClock`     | [`Clock`]     |
//! | `HlcError`        | [`Error`]     |
//!
//! # Recovery safety
//!
//! [`Clock::export_state`] and [`Clock::from_state`] alone do not guarantee
//! monotonicity across restarts: if timestamps were issued after the last
//! export and before the process died, a clock restored from that stale state
//! re-issues timestamps that are not greater than — possibly equal to or even
//! smaller than — already issued ones, breaking the causality guarantee.
//! Callers must ensure that the state used for recovery is not smaller than
//! the largest timestamp the clock has issued, for example by:
//!
//! - **persist-before-issue** — persisting the state before handing out each
//!   timestamp;
//! - **persisted future upper bound** — periodically persisting an upper
//!   bound that issuance never crosses, and waiting for or jumping to that
//!   bound after recovery (amortising the write cost from every issue to once
//!   per period);
//! - **forward jump on recovery** — advancing the restored physical component
//!   by no less than the maximum clock error of the deployment environment.
//!
//! The library does not provide this guarantee implicitly, because its cost —
//! persistence on the issue path, upper-bound lease management, or an extra
//! forward jump on recovery — must be chosen by the embedding protocol
//! according to its own consistency needs.
//!
//! # Remote validation and overflow
//!
//! [`Clock::observe`] runs the merge algorithm on any input and performs no
//! policy validation; callers must validate untrusted remote timestamps
//! before observing them (see [`validate_remote`]). The looser the
//! future-skew policy, the further remote values may push the clock ahead of
//! physical time and the more the logical counter can be driven towards
//! overflow — remote skew validation and overflow handling are two layers of
//! the same defense.
//!
//! # Example
//!
//! ```
//! use actr_hlc::{Clock, SystemClock};
//!
//! let mut clock = Clock::new(SystemClock);
//! let a = clock.local_event()?;
//! let b = clock.local_event()?;
//! assert!(b > a);
//! # Ok::<(), actr_hlc::Error>(())
//! ```

use std::fmt;

/// Source of physical time for a [`Clock`].
///
/// Calibrated, simulated or domain-specific time bases plug in through this
/// trait without changing the HLC algorithm.
pub trait PhysicalClock {
    /// Milliseconds since the Unix epoch. All participants exchanging
    /// timestamps must share the same time base; cross-node comparability of
    /// the physical component depends entirely on it.
    fn now_ms(&self) -> i64;
}

/// System UTC source. Not available on wasm32 (`SystemTime::now` panics
/// there).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

#[cfg(not(target_arch = "wasm32"))]
impl PhysicalClock for SystemClock {
    fn now_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(since) => i64::try_from(since.as_millis()).unwrap_or(i64::MAX),
            // The system clock is set before the Unix epoch: report a
            // negative reading, saturating on (theoretical) overflow.
            Err(before) => i64::try_from(before.duration().as_millis()).map_or(i64::MIN, |ms| -ms),
        }
    }
}

/// A hybrid logical clock timestamp.
///
/// Ordering is lexicographic over `(physical_ms, logical)`; the derived
/// [`Ord`] relies on the field declaration order, which therefore must not
/// change. If event A causally precedes event B — under the calling
/// discipline documented on [`Clock`] and safe state recovery — then
/// `A < B`. The converse does not hold: `A < B` does not imply causality,
/// and equality does not imply that two timestamps refer to the same event.
///
/// The physical component is milliseconds rather than nanoseconds on
/// purpose: millisecond Unix time stays below 2^53 and is therefore safely
/// representable in JavaScript and JSON (required by the Web bindings), and
/// concurrent events within one millisecond are absorbed by the `u32`
/// logical counter.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Timestamp {
    /// Logical physical component; may run ahead of current UTC after
    /// observing a remote timestamp or after a logical-counter carry.
    pub physical_ms: i64,
    /// Logical counter disambiguating events within one millisecond.
    pub logical: u32,
}

/// Persistable clock state.
///
/// The layout may grow over time (for example a persisted upper bound), so
/// the struct is non-exhaustive. [`State::new`] fills any future field with a
/// conservative default, and that default must keep satisfying the
/// recovery-safety precondition (see the crate-level documentation) even when
/// the field is absent from older persisted snapshots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct State {
    /// The last timestamp issued by (or restored into) the clock.
    pub last: Timestamp,
}

impl State {
    /// Creates a state whose last issued timestamp is `last`.
    pub fn new(last: Timestamp) -> Self {
        Self { last }
    }
}

/// Errors reported by [`Clock`] operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The physical component would overflow `i64` milliseconds. Timestamps
    /// never wrap around; the clock state is left unchanged.
    PhysicalOverflow,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PhysicalOverflow => {
                f.write_str("hybrid logical clock physical component overflowed i64 milliseconds")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Rejection reasons reported by [`validate_remote`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RemoteError {
    /// The remote physical component is negative.
    NegativePhysical,
    /// The remote physical component is further ahead of the local physical
    /// clock than the caller-supplied limit allows.
    FutureSkew {
        /// How many milliseconds the remote timestamp is ahead of the local
        /// physical reading (saturating).
        ahead_ms: i64,
    },
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NegativePhysical => {
                f.write_str("remote timestamp has a negative physical component")
            }
            Self::FutureSkew { ahead_ms } => write!(
                f,
                "remote timestamp is {ahead_ms} ms ahead of the local physical clock"
            ),
        }
    }
}

impl std::error::Error for RemoteError {}

/// Validates a remote timestamp against range and future-skew policy
/// (RFC-0419 §5).
///
/// `now_ms` is the caller's current physical reading and
/// `max_future_skew_ms` is the policy limit chosen by the caller; the
/// library ships the mechanism only. Callers must validate untrusted remote
/// timestamps with this helper (or an equivalent policy) before passing them
/// to [`Clock::observe`] — `observe` itself runs the merge algorithm on any
/// input without policy checks.
///
/// The skew is computed with saturating subtraction, so extreme inputs
/// cannot overflow.
pub fn validate_remote(
    now_ms: i64,
    remote: Timestamp,
    max_future_skew_ms: i64,
) -> Result<(), RemoteError> {
    if remote.physical_ms < 0 {
        return Err(RemoteError::NegativePhysical);
    }
    let ahead_ms = remote.physical_ms.saturating_sub(now_ms);
    if ahead_ms > max_future_skew_ms {
        return Err(RemoteError::FutureSkew { ahead_ms });
    }
    Ok(())
}

/// A hybrid logical clock (RFC-0419 §4).
///
/// Call [`Clock::local_event`] once for every local or send event, and
/// [`Clock::observe`] once for every receive event before it is handed to
/// processing logic. Both are event-driven operations; nothing advances
/// periodically. Under that discipline — and safe state recovery, see the
/// crate-level documentation — every issued timestamp is strictly greater
/// than all timestamps previously issued or observed by this clock.
#[derive(Debug)]
pub struct Clock<P: PhysicalClock> {
    physical: P,
    state: State,
}

impl<P: PhysicalClock> Clock<P> {
    /// Creates a clock with the initial state `(physical_ms: 0, logical: 0)`.
    ///
    /// The first [`Clock::local_event`] or [`Clock::observe`] converges to
    /// the current physical reading.
    pub fn new(physical: P) -> Self {
        Self::from_state(
            physical,
            State::new(Timestamp {
                physical_ms: 0,
                logical: 0,
            }),
        )
    }

    /// Issues a timestamp for a local or send event.
    ///
    /// The physical component becomes `max(now, p)`; if it did not advance,
    /// the logical counter increments, otherwise it resets to zero. On
    /// [`Error::PhysicalOverflow`] the clock state is unchanged.
    pub fn local_event(&mut self) -> Result<Timestamp, Error> {
        let now = self.physical.now_ms();
        let last = self.state.last;
        let next_p = now.max(last.physical_ms);
        let next = if next_p == last.physical_ms {
            Self::bump(next_p, last.logical)?
        } else {
            Timestamp {
                physical_ms: next_p,
                logical: 0,
            }
        };
        self.state.last = next;
        Ok(next)
    }

    /// Merges a remote timestamp for a receive event, issuing a timestamp
    /// strictly greater than both the remote timestamp and everything this
    /// clock issued so far.
    ///
    /// The physical component becomes `max(now, p, rp)`; the logical counter
    /// continues past whichever logical components share that physical value,
    /// or resets to zero when `now` alone wins. This method runs the
    /// algorithm on any input; apart from physical overflow it performs no
    /// policy validation, so callers must validate untrusted remote values
    /// first (see [`validate_remote`]). On [`Error::PhysicalOverflow`] the
    /// clock state is unchanged.
    pub fn observe(&mut self, remote: Timestamp) -> Result<Timestamp, Error> {
        let now = self.physical.now_ms();
        let last = self.state.last;
        let next_p = now.max(last.physical_ms).max(remote.physical_ms);
        let next = if next_p == last.physical_ms && next_p == remote.physical_ms {
            Self::bump(next_p, last.logical.max(remote.logical))?
        } else if next_p == last.physical_ms {
            Self::bump(next_p, last.logical)?
        } else if next_p == remote.physical_ms {
            Self::bump(next_p, remote.logical)?
        } else {
            Timestamp {
                physical_ms: next_p,
                logical: 0,
            }
        };
        self.state.last = next;
        Ok(next)
    }

    /// Returns the current persistable state.
    pub fn export_state(&self) -> State {
        self.state
    }

    /// Restores a clock from a previously exported state.
    ///
    /// Restoring alone does not guarantee cross-restart monotonicity; the
    /// caller must satisfy the recovery-safety precondition described in the
    /// crate-level documentation.
    pub fn from_state(physical: P, state: State) -> Self {
        Self { physical, state }
    }

    /// Increments the logical counter at `physical_ms`, carrying one
    /// millisecond into the physical component when the counter overflows.
    fn bump(physical_ms: i64, logical: u32) -> Result<Timestamp, Error> {
        match logical.checked_add(1) {
            Some(next_l) => Ok(Timestamp {
                physical_ms,
                logical: next_l,
            }),
            None => {
                let carried = physical_ms.checked_add(1).ok_or(Error::PhysicalOverflow)?;
                Ok(Timestamp {
                    physical_ms: carried,
                    logical: 0,
                })
            }
        }
    }
}
