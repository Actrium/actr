//! Wall-clock absolute-expiry verdicts that tolerate clock jumps
//! (RFC-0419 §3).
//!
//! Credentials and tokens carry absolute Unix-second deadlines chosen by a
//! remote issuer; checking them is the one timing decision that must read
//! the local wall clock (the deadline survives restarts and crosses
//! machines, so a process-local monotonic clock cannot express it). That
//! makes the check sensitive to wall-clock jumps: a clock set backwards
//! inflates the apparent remaining lifetime without bound and silently
//! extends the validity of an already-expired artifact.
//!
//! The guard here bounds that failure. Every issuance domain has a maximum
//! TTL, so a remaining lifetime far above it cannot come from a well-formed
//! issuance — the plausible causes are a local wall-clock rollback or a
//! corrupt deadline. Such deadlines are conservatively treated as expired
//! (the RFC's "treat as due" option for absolute-deadline checks), which
//! routes the artifact through its normal re-issuance path instead of
//! honoring it indefinitely. Deadlines within the plausible window are
//! evaluated exactly as a naive comparison would, so behavior under a
//! correct clock is unchanged.

/// Upper bound on the plausible remaining lifetime of an AIS-issued access
/// credential (`IdentityClaims.expires_at`).
///
/// AIS issues access credentials with a 1-hour TTL by default
/// (`IssuerConfig::token_ttl_secs`); 24 h leaves ample headroom for
/// deployments with longer configured TTLs plus bounded issuer/client clock
/// skew. TURN credentials are minted with the same 1-hour default and share
/// this bound.
pub(crate) const MAX_CREDENTIAL_REMAINING_SECS: i64 = 24 * 60 * 60;

/// Upper bound on the plausible remaining lifetime of an AIS renewal token
/// (`renewal_token_expires_at`).
///
/// The default issuance TTL is 24 h (`IssuerConfig::renewal_token_ttl_secs`);
/// 7 days leaves ample headroom for deployments with longer configured TTLs
/// plus bounded issuer/client clock skew.
pub(crate) const MAX_RENEWAL_TOKEN_REMAINING_SECS: i64 = 7 * 24 * 60 * 60;

/// Returns `true` when the absolute deadline `expires_at_secs` must be
/// treated as expired at `now_secs` (both Unix seconds):
///
/// - it is genuinely due (`expires_at <= now`) — identical to the naive
///   comparison; or
/// - its remaining lifetime exceeds `max_remaining_secs`, which no
///   well-formed issuance in the artifact's domain can produce. Under a
///   rolled-back local clock the naive comparison would keep honoring an
///   expired artifact for as long as the rollback lasted; treating the
///   implausible deadline as expired instead forces re-issuance through the
///   artifact's normal renewal path, which is always safe to take early.
///
/// The implausible case logs at `warn`: it indicates a host wall-clock
/// problem (or corrupt state) that degrades every wall-clock decision on the
/// machine, and its immediate consequence — an early re-issuance or a
/// rejected peer — is recoverable but visible.
pub(crate) fn expired_or_implausibly_far(
    expires_at_secs: i64,
    now_secs: i64,
    max_remaining_secs: i64,
    what: &str,
) -> bool {
    let remaining = expires_at_secs.saturating_sub(now_secs);
    if remaining <= 0 {
        return true;
    }
    if remaining > max_remaining_secs {
        tracing::warn!(
            what,
            expires_at = expires_at_secs,
            now = now_secs,
            remaining_secs = remaining,
            max_remaining_secs,
            "expiry is implausibly far in the future for its issuance domain; \
             treating as expired (likely local wall-clock rollback or corrupt \
             deadline; the artifact will be re-issued through its normal path)"
        );
        return true;
    }
    false
}

#[cfg(test)]
#[path = "expiry_tests.rs"]
mod tests;
