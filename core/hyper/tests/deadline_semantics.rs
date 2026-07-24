//! Deadline firing semantics on the tokio test runtime (RFC-0419 §6).
//!
//! RFC-0419 requires the same deadline contract on every runtime (native,
//! Web, test): a deadline fires *no earlier than* its instant, and the
//! virtual test clock advances deterministically without real sleeping.
//! These tests pin that contract for the runtime every deterministic hyper
//! test builds on — `#[tokio::test(start_paused = true)]` — with explicit
//! double-sided boundaries: still pending one millisecond before the
//! deadline, fired once the clock reaches it.
//!
//! Requirement map:
//! - R4: `timeout` / `sleep` never fire before their deadline.
//! - R6: virtual time only moves through `advance`, exactly by the amount
//!   advanced, without consuming real wall-clock time.
//! - R1 (facet expressible here): relative deadlines are driven by the
//!   monotonic clock; the wall clock neither advances them nor is consumed
//!   by them.

use std::time::Duration;

/// Poll the scheduler without advancing virtual time so spawned tasks can
/// observe timer state after an `advance`.
async fn yield_many(iterations: usize) {
    for _ in 0..iterations {
        tokio::task::yield_now().await;
    }
}

/// A `timeout` wrapping a forever-pending future is still pending one
/// millisecond before its deadline and resolves `Elapsed` once the virtual
/// clock reaches the deadline.
#[tokio::test(start_paused = true)]
async fn timeout_over_a_pending_future_fires_no_earlier_than_its_deadline() {
    const DEADLINE: Duration = Duration::from_secs(30);

    let timed = tokio::spawn(tokio::time::timeout(DEADLINE, std::future::pending::<()>()));
    tokio::task::yield_now().await;

    tokio::time::advance(DEADLINE - Duration::from_millis(1)).await;
    yield_many(50).await;
    assert!(
        !timed.is_finished(),
        "timeout must stay pending one tick before its deadline"
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    let result = timed.await.expect("timeout task should join");
    assert!(
        result.is_err(),
        "timeout must resolve Elapsed once the clock reaches its deadline"
    );
}

/// A plain `sleep` obeys the same double-sided boundary as `timeout`.
#[tokio::test(start_paused = true)]
async fn sleep_fires_no_earlier_than_its_deadline() {
    const DEADLINE: Duration = Duration::from_secs(7);

    let sleeper = tokio::spawn(tokio::time::sleep(DEADLINE));
    tokio::task::yield_now().await;

    tokio::time::advance(DEADLINE - Duration::from_millis(1)).await;
    yield_many(50).await;
    assert!(
        !sleeper.is_finished(),
        "sleep must stay pending one tick before its deadline"
    );

    tokio::time::advance(Duration::from_millis(1)).await;
    sleeper.await.expect("sleep task should join");
}

/// The virtual monotonic clock moves exactly by the advanced amounts and is
/// decoupled from the wall clock: an hour of virtual time costs (nearly) no
/// real time, so relative deadlines cannot be waiting on `SystemTime`.
#[tokio::test(start_paused = true)]
async fn virtual_monotonic_time_is_deterministic_and_wall_clock_free() {
    let virtual_start = tokio::time::Instant::now();
    let wall_start = std::time::SystemTime::now();

    // Deterministic accumulation: successive advances add exactly (R6).
    tokio::time::advance(Duration::from_secs(1800)).await;
    tokio::time::advance(Duration::from_millis(1)).await;
    tokio::time::advance(Duration::from_secs(1800)).await;
    assert_eq!(
        tokio::time::Instant::now() - virtual_start,
        Duration::from_secs(3600) + Duration::from_millis(1),
        "virtual monotonic time must advance exactly by the sum of advances"
    );

    // Decoupling from the wall clock (R1 facet): one virtual hour must not
    // consume anywhere near an hour of real time. The generous bound keeps
    // the assertion safe on heavily loaded CI machines.
    let wall_elapsed = wall_start.elapsed().unwrap_or_default();
    assert!(
        wall_elapsed < Duration::from_secs(600),
        "virtual advance must not wait on the wall clock (took {wall_elapsed:?})"
    );
}
