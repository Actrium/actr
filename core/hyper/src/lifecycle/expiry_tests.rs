use super::*;

const NOW: i64 = 1_700_000_000;

#[test]
fn past_and_present_deadlines_are_expired() {
    assert!(expired_or_implausibly_far(NOW - 1, NOW, 3600, "test"));
    assert!(expired_or_implausibly_far(0, NOW, 3600, "test"));
    // Exactly now -> expired (`<=`), matching the naive comparison.
    assert!(expired_or_implausibly_far(NOW, NOW, 3600, "test"));
}

#[test]
fn deadlines_within_the_plausible_window_pass_unchanged() {
    assert!(!expired_or_implausibly_far(NOW + 1, NOW, 3600, "test"));
    assert!(!expired_or_implausibly_far(NOW + 3599, NOW, 3600, "test"));
    // Exactly at the bound is still plausible (`>` triggers, not `>=`).
    assert!(!expired_or_implausibly_far(NOW + 3600, NOW, 3600, "test"));
}

#[test]
fn implausibly_far_deadlines_are_treated_as_expired() {
    // One second past the bound.
    assert!(expired_or_implausibly_far(NOW + 3601, NOW, 3600, "test"));

    // Wall-clock rollback simulation: a credential issued with a 1 h TTL at
    // real time NOW looks 30 days + 1 h "remaining" after the local clock is
    // set back 30 days; the naive comparison would honor it for the whole
    // rollback.
    let issued_expiry = NOW + 3600;
    let rolled_back_now = NOW - 30 * 24 * 3600;
    assert!(expired_or_implausibly_far(
        issued_expiry,
        rolled_back_now,
        MAX_CREDENTIAL_REMAINING_SECS,
        "test"
    ));

    // Absurd persisted value (corrupt state).
    assert!(expired_or_implausibly_far(
        i64::MAX,
        NOW,
        MAX_RENEWAL_TOKEN_REMAINING_SECS,
        "test"
    ));
}

#[test]
fn saturating_arithmetic_handles_extreme_inputs() {
    // `expires_at - now` would overflow i64; must saturate, not panic.
    assert!(expired_or_implausibly_far(i64::MAX, i64::MIN, 3600, "test"));
    // Deadline before the epoch with `now` far ahead: plainly expired.
    assert!(expired_or_implausibly_far(i64::MIN, i64::MAX, 3600, "test"));
}
