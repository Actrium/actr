# Metrics Ring Buffer — Design

Single-node, in-memory metrics store for lightweight health diagnostics.
Not a replacement for log-based observability; purpose is fast local dashboards.

## 1. Data Requirements (from UI)

Each **service** (Signaling / STUN / TURN / AIS / KS) needs four time-series:

| Metric           | Type    | Source                            | Semantics                  |
|------------------|---------|-----------------------------------|----------------------------|
| `active_conns`   | gauge   | snapshot at sample time           | current open connections   |
| `requests`       | counter | delta since previous sample       | request count in interval  |
| `failed_requests`| counter | delta since previous sample       | failure count in interval  |
| `latency_p95_ms` | summary | p95 of latencies within interval  | 95th-percentile latency    |

Derived on read:
- `success_rate` = `(requests - failed_requests) / requests * 100`

## 2. Three-Tier Retention

```
Tier 0 (fine)    1-min  buckets × 15  = 15 min window
Tier 1 (medium)  15-min buckets × 16  = 4 h   window
Tier 2 (coarse)  4-h    buckets × 18  = 72 h  window
```

Each tier is a fixed-size **ring buffer** (VecDeque). No heap growth, no GC.

### Roll-up rules

When a Tier 0 bucket expires (older than 15 min), the oldest 15 Tier-0 buckets
are aggregated into one Tier 1 bucket. Same pattern for Tier 1 → Tier 2.

Aggregation per metric:
- `active_conns` → **mean** of the 15 (or 16) samples
- `requests`     → **sum** of deltas
- `failed_requests` → **sum** of deltas
- `latency_p95_ms`  → **max** of p95 values (conservative upper bound)

### Timing

A background `tokio::spawn` task runs once per minute:
1. Sample all services → push to Tier 0 ring.
2. If Tier 0 length > 15, pop oldest, accumulate into a staging buffer.
3. Every 15 pops (= 15 min), flush staging → push to Tier 1.
4. Same cascade for Tier 1 → Tier 2 every 16 Tier-1 pops (= 4 h).

## 3. Data Structures

```rust
/// One sample point for a single service.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MetricSample {
    pub ts: i64,               // unix epoch seconds
    pub active_conns: u32,
    pub requests: u64,
    pub failed_requests: u64,
    pub latency_p95_ms: f64,
}

/// Per-service ring buffer at one tier.
struct Ring {
    buf: VecDeque<MetricSample>,
    cap: usize,
}

/// All three tiers for one service.
struct ServiceRings {
    tier0: Ring,  // cap=15
    tier1: Ring,  // cap=16
    tier2: Ring,  // cap=18
}

/// Top-level store, keyed by service type (ResourceType as i32).
pub struct MetricsStore {
    inner: Arc<RwLock<HashMap<i32, ServiceRings>>>,
}
```

Total memory per service: `(15 + 16 + 18) × ~40 bytes ≈ 2 KB`.
For 5 services: ~10 KB. Negligible.

## 4. Sampling — Where Counters Come From

Currently `ServiceInfo → ServiceStatus` returns zeros for
`active_connections`, `total_requests`, `failed_requests`, `average_latency_ms`.

Each service implementation needs to maintain **atomic counters**:

```rust
pub struct ServiceCounters {
    pub active_conns: AtomicU32,
    pub total_requests: AtomicU64,
    pub failed_requests: AtomicU64,
    /// Accumulator for p95 approximation (e.g. HDR histogram or T-Digest).
    /// For simplicity at this scale: track max latency per interval as an
    /// approximation, or keep a small sorted buffer and pick the 95th value.
    pub latency_tracker: Mutex<LatencyTracker>,
}
```

Signaling, AIS, KS already have request handlers — instrument there.
STUN/TURN have packet-level processing — increment on allocate/bind.

The 1-minute sampler reads these counters, computes deltas, resets the
latency tracker, and pushes a `MetricSample`.

## 5. API Endpoint

`/admin/api/` is the BFF (Backend-For-Frontend) path used exclusively by
the admin UI SPA. Defined in `actrixd/src/service/http/admin_api.rs`.

However, **MetricsStore itself lives in `control`** (always-on, no feature
gate), so it's available regardless of whether admin-ui is compiled in.
The admin_api route is just the HTTP exposure layer.

```
GET /admin/api/metrics/timeseries?service_type={type}&tier={0|1|2}
```

Response:
```json
{
  "service_type": 3,
  "tier": 0,
  "interval_secs": 60,
  "samples": [
    { "ts": 1709712000, "active_conns": 128, "requests": 3420, "failed_requests": 2, "latency_p95_ms": 3.8 },
    ...
  ]
}
```

The frontend requests all three tiers and maps them to the three time-range
tabs (15min / 4h / 72h). Each tier's data is already at the correct
granularity — no client-side aggregation needed.

## 6. Integration Points

```
platform::monitoring
├── service_info.rs      (existing — add ServiceCounters)
├── service_registry.rs  (existing — ServiceCollector)
└── metrics_store.rs     (NEW — MetricsStore, Ring, sampling task)

control::service
└── AdminApiService      (add metrics_store field, query methods)

actrixd::service::http::admin_api   (admin-ui feature only)
└── GET /admin/api/metrics/timeseries  (thin BFF route)
```

### Startup sequence

1. `MetricsStore::new()` — created in main, passed to AdminApiService.
2. `MetricsStore::start_sampler(service_collector)` — spawns the 1-min
   tick task. Needs a handle to ServiceCollector to read current counters.
3. Admin API serves the ring data on request.

## 7. Scope Boundaries

**In scope:**
- In-memory ring buffers, no persistence across restarts
- 4 metrics × 5 services × 3 tiers
- Single API endpoint, JSON response
- Frontend switches from demo data to real data

**Out of scope:**
- Disk persistence (process restart = empty rings, fills in 15 min)
- Cross-node aggregation (control plane concern)
- Alerting thresholds (use external monitoring)
- Sub-minute granularity (overkill for dashboard)

## 8. Implementation Order

1. **`ServiceCounters`** — add to `ServiceInfo`, instrument Signaling/AIS/KS request handlers and STUN/TURN packet paths.
2. **`MetricsStore`** — ring buffer + sampler task in `platform::monitoring`.
3. **API endpoint** — wire into admin_api.
4. **Frontend** — replace `DEMO_STATS` / `genSeries` with real API calls.
