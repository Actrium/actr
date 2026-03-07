//! Three-tier in-memory ring buffer for per-service metrics.
//!
//! Each service gets three retention tiers:
//! - Tier 0: 1-min granularity, 15 samples (15 min window)
//! - Tier 1: 15-min granularity, 16 samples (4 h window)
//! - Tier 2: 4-hour granularity, 18 samples (72 h window)
//!
//! A background sampler task reads atomic counters once per minute and
//! pushes samples into tier 0.  Rollups cascade automatically.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, RwLock};

// ---------------------------------------------------------------------------
// MetricSample
// ---------------------------------------------------------------------------

/// One sample point for a single service.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MetricSample {
    /// Unix epoch seconds.
    pub ts: i64,
    /// Current open connections (gauge).
    pub active_conns: u32,
    /// Request count within the interval (delta).
    pub requests: u64,
    /// Failed request count within the interval (delta).
    pub failed_requests: u64,
    /// 95th-percentile latency in milliseconds.
    pub latency_p95_ms: f64,
}

// ---------------------------------------------------------------------------
// ServiceCounters
// ---------------------------------------------------------------------------

/// Atomic counters that services increment in their hot paths.
///
/// The 1-minute sampler calls [`snapshot_and_reset`] to drain deltas and
/// produce a [`MetricSample`].
pub struct ServiceCounters {
    /// Current open connections (gauge — not reset on snapshot).
    pub active_conns: AtomicU32,
    /// Monotonic total requests since last snapshot.
    pub total_requests: AtomicU64,
    /// Monotonic failed requests since last snapshot.
    pub failed_requests: AtomicU64,
    /// Latencies collected during the current interval, drained on snapshot.
    latencies: Mutex<Vec<f64>>,
}

impl ServiceCounters {
    pub fn new() -> Self {
        Self {
            active_conns: AtomicU32::new(0),
            total_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            latencies: Mutex::new(Vec::new()),
        }
    }

    /// Increment the active-connections gauge.
    pub fn inc_conns(&self) {
        self.active_conns.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the active-connections gauge.
    pub fn dec_conns(&self) {
        self.active_conns.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record one completed request.
    ///
    /// Increments `total_requests` unconditionally, and `failed_requests` when
    /// `success` is false.  The latency value is pushed into a buffer that
    /// will be drained when the next sample is taken.
    pub async fn record_request(&self, success: bool, latency_ms: f64) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.failed_requests.fetch_add(1, Ordering::Relaxed);
        }
        self.latencies.lock().await.push(latency_ms);
    }

    /// Take a point-in-time snapshot and reset the delta counters.
    ///
    /// - `active_conns` is read but **not** reset (it is a gauge).
    /// - `total_requests` and `failed_requests` are swapped to zero.
    /// - Latencies are drained and the p95 value is computed.
    pub async fn snapshot_and_reset(&self) -> MetricSample {
        let active_conns = self.active_conns.load(Ordering::Relaxed);
        let requests = self.total_requests.swap(0, Ordering::Relaxed);
        let failed_requests = self.failed_requests.swap(0, Ordering::Relaxed);

        let mut lats = {
            let mut guard = self.latencies.lock().await;
            std::mem::take(&mut *guard)
        };

        let latency_p95_ms = compute_p95(&mut lats);

        MetricSample {
            ts: chrono::Utc::now().timestamp(),
            active_conns,
            requests,
            failed_requests,
            latency_p95_ms,
        }
    }
}

impl Default for ServiceCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ServiceCounters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceCounters")
            .field("active_conns", &self.active_conns.load(Ordering::Relaxed))
            .field(
                "total_requests",
                &self.total_requests.load(Ordering::Relaxed),
            )
            .field(
                "failed_requests",
                &self.failed_requests.load(Ordering::Relaxed),
            )
            .finish()
    }
}

/// Compute the 95th-percentile value from an unsorted slice.
/// Returns 0.0 for an empty slice.
fn compute_p95(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() as f64) * 0.95).ceil() as usize;
    let idx = idx.min(values.len()) - 1;
    values[idx]
}

// ---------------------------------------------------------------------------
// Ring
// ---------------------------------------------------------------------------

/// Fixed-capacity ring buffer backed by a `VecDeque`.
struct Ring {
    buf: VecDeque<MetricSample>,
    cap: usize,
}

impl Ring {
    fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// Push a sample, evicting the oldest entry when at capacity.
    fn push(&mut self, sample: MetricSample) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(sample);
    }

    fn to_vec(&self) -> Vec<MetricSample> {
        self.buf.iter().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// ServiceRings
// ---------------------------------------------------------------------------

/// Three retention tiers for one service, plus staging accumulators for
/// tier roll-ups.
struct ServiceRings {
    tier0: Ring, // cap=15, 1-min granularity
    tier1: Ring, // cap=16, 15-min granularity
    tier2: Ring, // cap=18, 4-hour granularity
    /// Accumulator for tier0 -> tier1 rollup (collects 15 samples).
    tier1_acc: Vec<MetricSample>,
    /// Accumulator for tier1 -> tier2 rollup (collects 16 samples).
    tier2_acc: Vec<MetricSample>,
}

impl ServiceRings {
    fn new() -> Self {
        Self {
            tier0: Ring::new(15),
            tier1: Ring::new(16),
            tier2: Ring::new(18),
            tier1_acc: Vec::with_capacity(15),
            tier2_acc: Vec::with_capacity(16),
        }
    }
}

// ---------------------------------------------------------------------------
// Rollup helpers
// ---------------------------------------------------------------------------

/// Aggregate a batch of samples into a single rolled-up sample.
///
/// Rules:
/// - `active_conns` -> mean
/// - `requests` -> sum
/// - `failed_requests` -> sum
/// - `latency_p95_ms` -> max
fn aggregate(samples: &[MetricSample]) -> MetricSample {
    debug_assert!(!samples.is_empty());

    let ts = samples.last().map(|s| s.ts).unwrap_or(0);

    let conns_sum: u64 = samples.iter().map(|s| s.active_conns as u64).sum();
    let active_conns = (conns_sum as f64 / samples.len() as f64).round() as u32;

    let requests: u64 = samples.iter().map(|s| s.requests).sum();
    let failed_requests: u64 = samples.iter().map(|s| s.failed_requests).sum();

    let latency_p95_ms = samples
        .iter()
        .map(|s| s.latency_p95_ms)
        .fold(0.0_f64, f64::max);

    MetricSample {
        ts,
        active_conns,
        requests,
        failed_requests,
        latency_p95_ms,
    }
}

// ---------------------------------------------------------------------------
// MetricsStore
// ---------------------------------------------------------------------------

/// Top-level per-service metrics store, keyed by service type (`ResourceType` as `i32`).
#[derive(Clone)]
pub struct MetricsStore {
    inner: Arc<RwLock<HashMap<i32, ServiceRings>>>,
}

impl MetricsStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Push a 1-minute sample for `service_type` into tier 0 and cascade
    /// rollups into tier 1 / tier 2 as needed.
    pub async fn push_sample(&self, service_type: i32, sample: MetricSample) {
        let mut map = self.inner.write().await;
        let rings = map.entry(service_type).or_insert_with(ServiceRings::new);

        // Push into tier 0.
        rings.tier0.push(sample.clone());

        // Accumulate for tier 1 rollup.
        rings.tier1_acc.push(sample);

        // Every 15 tier-0 samples -> roll up into one tier-1 sample.
        if rings.tier1_acc.len() == 15 {
            let rolled = aggregate(&rings.tier1_acc);
            rings.tier1_acc.clear();

            rings.tier1.push(rolled.clone());

            // Accumulate for tier 2 rollup.
            rings.tier2_acc.push(rolled);

            // Every 16 tier-1 samples -> roll up into one tier-2 sample.
            if rings.tier2_acc.len() == 16 {
                let rolled2 = aggregate(&rings.tier2_acc);
                rings.tier2_acc.clear();
                rings.tier2.push(rolled2);
            }
        }
    }

    /// Query samples at the given tier for a service.
    ///
    /// Returns an empty vec if no data exists for `service_type` or `tier`
    /// is out of range.
    pub async fn query(&self, service_type: i32, tier: u8) -> Vec<MetricSample> {
        let map = self.inner.read().await;
        let Some(rings) = map.get(&service_type) else {
            return Vec::new();
        };
        match tier {
            0 => rings.tier0.to_vec(),
            1 => rings.tier1.to_vec(),
            2 => rings.tier2.to_vec(),
            _ => Vec::new(),
        }
    }

    /// Spawn a background sampler task that periodically snapshots each
    /// service's counters and pushes the samples into the store.
    ///
    /// The task runs every `interval` (typically 60 s) until the runtime
    /// shuts down.
    pub fn start_sampler(
        self,
        counters: Arc<HashMap<i32, Arc<ServiceCounters>>>,
        interval: Duration,
    ) {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            loop {
                tick.tick().await;
                for (&svc_type, ctr) in counters.iter() {
                    let sample = ctr.snapshot_and_reset().await;
                    self.push_sample(svc_type, sample).await;
                }
            }
        });
    }
}

impl std::fmt::Debug for MetricsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsStore").finish()
    }
}

impl Default for MetricsStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_p95_basic() {
        // 20 values: 1..=20 -> p95 index = ceil(20*0.95)-1 = 19-1 = 18 -> value 19
        let mut vals: Vec<f64> = (1..=20).map(|v| v as f64).collect();
        assert!((compute_p95(&mut vals) - 19.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compute_p95_empty() {
        assert!((compute_p95(&mut []) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compute_p95_single() {
        assert!((compute_p95(&mut [42.0]) - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn ring_evicts_oldest() {
        let mut ring = Ring::new(3);
        for i in 0..5 {
            ring.push(MetricSample {
                ts: i,
                active_conns: 0,
                requests: 0,
                failed_requests: 0,
                latency_p95_ms: 0.0,
            });
        }
        let v = ring.to_vec();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].ts, 2);
        assert_eq!(v[2].ts, 4);
    }

    #[test]
    fn aggregate_applies_rules() {
        let samples = vec![
            MetricSample {
                ts: 100,
                active_conns: 10,
                requests: 50,
                failed_requests: 2,
                latency_p95_ms: 3.5,
            },
            MetricSample {
                ts: 200,
                active_conns: 20,
                requests: 60,
                failed_requests: 3,
                latency_p95_ms: 7.1,
            },
        ];
        let agg = aggregate(&samples);
        assert_eq!(agg.ts, 200); // latest
        assert_eq!(agg.active_conns, 15); // mean(10,20) = 15
        assert_eq!(agg.requests, 110); // sum
        assert_eq!(agg.failed_requests, 5); // sum
        assert!((agg.latency_p95_ms - 7.1).abs() < f64::EPSILON); // max
    }

    #[tokio::test]
    async fn push_and_query() {
        let store = MetricsStore::new();
        let sample = MetricSample {
            ts: 1000,
            active_conns: 5,
            requests: 100,
            failed_requests: 1,
            latency_p95_ms: 2.0,
        };
        store.push_sample(1, sample).await;

        let tier0 = store.query(1, 0).await;
        assert_eq!(tier0.len(), 1);
        assert_eq!(tier0[0].ts, 1000);

        // tier 1 should be empty (need 15 samples to trigger rollup)
        assert!(store.query(1, 1).await.is_empty());
    }

    #[tokio::test]
    async fn tier0_to_tier1_rollup() {
        let store = MetricsStore::new();
        for i in 0..15 {
            store
                .push_sample(
                    1,
                    MetricSample {
                        ts: i * 60,
                        active_conns: 10,
                        requests: 100,
                        failed_requests: 1,
                        latency_p95_ms: 5.0,
                    },
                )
                .await;
        }
        let tier1 = store.query(1, 1).await;
        assert_eq!(tier1.len(), 1);
        assert_eq!(tier1[0].active_conns, 10); // mean of uniform = same
        assert_eq!(tier1[0].requests, 1500); // 15 * 100
        assert_eq!(tier1[0].failed_requests, 15); // 15 * 1
        assert!((tier1[0].latency_p95_ms - 5.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn service_counters_snapshot() {
        let ctr = ServiceCounters::new();
        ctr.inc_conns();
        ctr.inc_conns();
        ctr.record_request(true, 1.0).await;
        ctr.record_request(false, 10.0).await;
        ctr.record_request(true, 5.0).await;

        let snap = ctr.snapshot_and_reset().await;
        assert_eq!(snap.active_conns, 2);
        assert_eq!(snap.requests, 3);
        assert_eq!(snap.failed_requests, 1);
        // p95 of [1.0, 5.0, 10.0] -> ceil(3*0.95)=3, idx=2 -> 10.0
        assert!((snap.latency_p95_ms - 10.0).abs() < f64::EPSILON);

        // After snapshot, deltas should be reset
        let snap2 = ctr.snapshot_and_reset().await;
        assert_eq!(snap2.requests, 0);
        assert_eq!(snap2.failed_requests, 0);
        assert!((snap2.latency_p95_ms - 0.0).abs() < f64::EPSILON);
        // Gauge stays
        assert_eq!(snap2.active_conns, 2);
    }
}
