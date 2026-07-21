//! Audited production-timer facade for RFC-0400.
//!
//! Every timer created by `actr-hyper` production code is assigned one stable
//! ID and one normative category in [`ids::ALL`]. Test-only code may continue
//! to use Tokio's clock directly. Timers owned by dependencies are registered
//! through [`register_external`] at the configuration boundary.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::time::{Instant, MissedTickBehavior};

/// RFC-0400 timer categories. Adding a category requires an RFC amendment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)] // Keep the complete normative RFC category set.
pub(crate) enum TimerCategory {
    BusinessHysteresis,
    ProtocolSelection,
    ProtocolSchedule,
    RetentionExpiry,
    FailureDeadline,
    FailureBackoff,
    CompatibilityPolling,
    CompatibilityDebounce,
}

impl TimerCategory {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BusinessHysteresis => "business_hysteresis",
            Self::ProtocolSelection => "protocol_selection",
            Self::ProtocolSchedule => "protocol_schedule",
            Self::RetentionExpiry => "retention_expiry",
            Self::FailureDeadline => "failure_deadline",
            Self::FailureBackoff => "failure_backoff",
            Self::CompatibilityPolling => "compatibility_polling",
            Self::CompatibilityDebounce => "compatibility_debounce",
        }
    }
}

/// One source-controlled inventory row.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // Inventory metadata is consumed by the drift test and tooling.
pub(crate) struct TimerDefinition {
    pub(crate) symbol: &'static str,
    pub(crate) id: &'static str,
    pub(crate) owner: &'static str,
    pub(crate) category: TimerCategory,
    pub(crate) duration_source: &'static str,
    pub(crate) arm_condition: &'static str,
    pub(crate) success_signal: &'static str,
    pub(crate) interrupt_source: &'static str,
    pub(crate) expiry_effect: &'static str,
    pub(crate) reset_rule: &'static str,
    pub(crate) external: bool,
}

macro_rules! timer_inventory {
    (
        $(
            $symbol:ident => {
                id: $id:literal,
                owner: $owner:literal,
                category: $category:ident,
                duration: $duration:literal,
                arm: $arm:literal,
                success: $success:literal,
                interrupt: $interrupt:literal,
                expiry: $expiry:literal,
                reset: $reset:literal,
                external: $external:literal
            }
        ),+ $(,)?
    ) => {
        pub(crate) mod ids {
            use super::{TimerCategory, TimerDefinition};

            $(
                #[allow(dead_code)] // Some entries are feature-gated in production.
                pub(crate) const $symbol: TimerDefinition = TimerDefinition {
                    symbol: stringify!($symbol),
                    id: $id,
                    owner: $owner,
                    category: TimerCategory::$category,
                    duration_source: $duration,
                    arm_condition: $arm,
                    success_signal: $success,
                    interrupt_source: $interrupt,
                    expiry_effect: $expiry,
                    reset_rule: $reset,
                    external: $external,
                };
            )+

            #[allow(dead_code)] // Read by audit tests and external inventory tooling.
            pub(crate) const ALL: &[TimerDefinition] = &[$($symbol),+];
        }
    };
}

timer_inventory! {
    ACTR_REF_TASK_SHUTDOWN => {
        id: "actr_ref.task_shutdown", owner: "actr_ref", category: FailureDeadline,
        duration: "fixed 5s", arm: "actor shutdown begins", success: "task JoinHandle completes",
        interrupt: "task completion", expiry: "abort unfinished task", reset: "never", external: false
    },
    EXECUTOR_DISPATCH => {
        id: "executor.dispatch", owner: "executor", category: FailureDeadline,
        duration: "DispatchConcurrency.dispatch_timeout", arm: "concurrent guest dispatch starts",
        success: "guarded dispatch future completes", interrupt: "runner shutdown or completion",
        expiry: "poison runner and return timeout", reset: "new dispatch only", external: false
    },
    DATA_CHUNK_REGISTRY_SHUTDOWN => {
        id: "inbound.data_chunk_registry.shutdown", owner: "data_chunk_registry", category: FailureDeadline,
        duration: "SHUTDOWN_JOIN_TIMEOUT", arm: "registry shutdown joins workers",
        success: "all workers join", interrupt: "worker completion", expiry: "abort remaining workers",
        reset: "never", external: false
    },
    CREDENTIAL_RATE_LIMIT => {
        id: "lifecycle.credential.rate_limit", owner: "credential_manager", category: FailureBackoff,
        duration: "server Retry-After", arm: "credential renewal is rate limited",
        success: "retry-after deadline", interrupt: "renewal task cancellation",
        expiry: "return rate-limited result", reset: "new Retry-After response", external: false
    },
    CREDENTIAL_RETRY_BACKOFF => {
        id: "lifecycle.credential.retry_backoff", owner: "credential_manager", category: FailureBackoff,
        duration: "credential Backoff policy", arm: "retryable credential renewal failure",
        success: "backoff deadline", interrupt: "renewal task cancellation",
        expiry: "return retryable result", reset: "new renewal sequence", external: false
    },
    HEARTBEAT_POWER_RESERVE => {
        id: "lifecycle.heartbeat.power_reserve", owner: "heartbeat", category: FailureDeadline,
        duration: "POWER_RESERVE_FETCH_TIMEOUT", arm: "heartbeat samples power reserve",
        success: "power reserve future completes", interrupt: "heartbeat task cancellation",
        expiry: "use conservative fallback", reset: "each heartbeat sample", external: false
    },
    HEARTBEAT_PONG => {
        id: "lifecycle.heartbeat.pong", owner: "heartbeat", category: FailureDeadline,
        duration: "40% heartbeat interval, minimum 1s", arm: "heartbeat ping is sent",
        success: "pong response", interrupt: "heartbeat task cancellation",
        expiry: "record heartbeat failure", reset: "next ping", external: false
    },
    HEARTBEAT_SCHEDULE => {
        id: "lifecycle.heartbeat.schedule", owner: "heartbeat", category: ProtocolSchedule,
        duration: "configured heartbeat_interval", arm: "heartbeat task starts",
        success: "next protocol tick", interrupt: "shutdown or actor-id update",
        expiry: "send heartbeat", reset: "configured interval only", external: false
    },
    RECOVERY_CLEANUP_TEARDOWN => {
        id: "lifecycle.recovery.cleanup_teardown", owner: "recovery_supervisor", category: FailureDeadline,
        duration: "remaining cleanup obligation deadline", arm: "cleanup effect starts",
        success: "cleanup reaches its local goal", interrupt: "effect cancellation or completion",
        expiry: "report abandoned teardown", reset: "new cleanup obligation only", external: false
    },
    RECOVERY_OFFLINE_TEARDOWN => {
        id: "lifecycle.recovery.offline_teardown", owner: "recovery_supervisor", category: FailureDeadline,
        duration: "remaining offline obligation deadline", arm: "confirmed-offline effect starts",
        success: "offline teardown reaches its local goal", interrupt: "path reversal or completion",
        expiry: "report abandoned teardown", reset: "new offline obligation only", external: false
    },
    RECOVERY_POLICY_DEADLINE => {
        id: "lifecycle.recovery.policy_deadline", owner: "recovery_supervisor", category: BusinessHysteresis,
        duration: "absolute translated policy deadline", arm: "supervisor emits TimerDirective::Arm",
        success: "matching timer is cancelled by material input", interrupt: "replacement timer identity",
        expiry: "enqueue identity-bearing policy input", reset: "material policy transition only", external: false
    },
    NETWORK_EVENT_ACCEPTANCE => {
        id: "lifecycle.network_event.acceptance", owner: "network_event_handle", category: FailureDeadline,
        duration: "NetworkEventHandle.result_timeout", arm: "caller submits lifecycle fact",
        success: "supervisor acknowledges acceptance", interrupt: "response channel close",
        expiry: "return acceptance timeout without cancelling work", reset: "new submitted fact", external: false
    },
    EXTERNAL_AIS_HTTP_REQUEST => {
        id: "external.ais.http_request", owner: "reqwest", category: FailureDeadline,
        duration: "fixed 30s client request timeout", arm: "AIS HTTP client sends a request",
        success: "HTTP response future completes", interrupt: "request cancellation or client drop",
        expiry: "return reqwest timeout", reset: "new HTTP request only", external: true
    },
    NODE_RPC_RESPONSE => {
        id: "lifecycle.node.rpc_response", owner: "node", category: FailureDeadline,
        duration: "request envelope deadline", arm: "node awaits actor response",
        success: "response future completes", interrupt: "node shutdown or completion",
        expiry: "return unavailable timeout", reset: "new request only", external: false
    },
    NODE_UNREGISTER => {
        id: "lifecycle.node.unregister", owner: "node", category: FailureDeadline,
        duration: "fixed 5s", arm: "node unregisters during shutdown",
        success: "unregister response", interrupt: "shutdown completion",
        expiry: "continue bounded shutdown", reset: "never", external: false
    },
    MAILBOX_DEPTH_FALLBACK => {
        id: "lifecycle.mailbox.depth_fallback", owner: "node.mailbox", category: CompatibilityPolling,
        duration: "fixed 1s", arm: "third-party mailbox lacks depth observer",
        success: "mailbox status sample", interrupt: "shutdown or observer availability",
        expiry: "read mailbox depth", reset: "each compatibility sample", external: false
    },
    MAILBOX_EMPTY_FALLBACK => {
        id: "lifecycle.mailbox.empty_fallback", owner: "node.mailbox", category: CompatibilityPolling,
        duration: "fixed 10ms", arm: "third-party mailbox is empty and lacks enqueue observation",
        success: "enqueue observation when supported", interrupt: "shutdown or in-flight completion",
        expiry: "retry dequeue", reset: "each empty compatibility read", external: false
    },
    MAILBOX_ERROR_BACKOFF => {
        id: "lifecycle.mailbox.error_backoff", owner: "node.mailbox", category: FailureBackoff,
        duration: "fixed 1s", arm: "mailbox dequeue fails",
        success: "backoff deadline", interrupt: "node shutdown", expiry: "retry dequeue",
        reset: "new dequeue failure", external: false
    },
    PEER_SEND_BACKOFF => {
        id: "outbound.peer.send_backoff", owner: "peer_gate", category: FailureBackoff,
        duration: "payload retry policy", arm: "transient peer send fails",
        success: "backoff deadline", interrupt: "send future cancellation",
        expiry: "retry send", reset: "successful send or new request", external: false
    },
    PEER_REQUEST_DEADLINE => {
        id: "outbound.peer.request", owner: "peer_gate", category: FailureDeadline,
        duration: "caller or payload send budget", arm: "peer request/send starts",
        success: "send and response path completes", interrupt: "caller cancellation or completion",
        expiry: "close stale transport and return timeout", reset: "new request only", external: false
    },
    CORRELATION_RESPONSE => {
        id: "transport.correlation.response", owner: "correlation", category: FailureDeadline,
        duration: "caller response timeout", arm: "pending RPC awaits response",
        success: "oneshot response", interrupt: "response channel close",
        expiry: "remove pending correlation and return timeout", reset: "new RPC only", external: false
    },
    LANE_INITIAL_READY => {
        id: "transport.lane.initial_ready", owner: "lane", category: FailureDeadline,
        duration: "INITIAL_CONNECTION_TIMEOUT", arm: "lane waits for initial open state",
        success: "watch state becomes ready or terminal", interrupt: "terminal state",
        expiry: "return connection timeout", reset: "replacement lane only", external: false
    },
    PEER_TRANSPORT_CREATE => {
        id: "transport.peer.create", owner: "peer_transport", category: FailureDeadline,
        duration: "fixed 10s", arm: "transport creator awaits flight completion",
        success: "flight publishes or fails", interrupt: "close or creator cancellation",
        expiry: "release creator ownership and fail", reset: "replacement flight only", external: false
    },
    PEER_TRANSPORT_HEALTH => {
        id: "transport.peer.health", owner: "peer_transport", category: ProtocolSchedule,
        duration: "test-utils configured interval", arm: "test health checker starts",
        success: "health sample tick", interrupt: "task cancellation",
        expiry: "sample transport health", reset: "configured interval only", external: false
    },
    WIRE_POOL_CONNECT_BACKOFF => {
        id: "transport.wire_pool.connect_backoff", owner: "wire_pool", category: FailureBackoff,
        duration: "wire connection retry policy", arm: "wire creation attempt fails",
        success: "backoff deadline", interrupt: "pool close",
        expiry: "retry wire creation", reset: "successful connection or new flight", external: false
    },
    WASM_INSTANTIATION => {
        id: "wasm.instantiation", owner: "wasm_host", category: FailureDeadline,
        duration: "WasmRuntimeLimits.invocation_timeout", arm: "component instantiation starts",
        success: "component instantiates", interrupt: "host cancellation",
        expiry: "fail and poison store", reset: "new store only", external: false
    },
    WASM_GUEST_INVOCATION => {
        id: "wasm.guest_invocation", owner: "wasm_host", category: FailureDeadline,
        duration: "WasmRuntimeLimits.invocation_timeout", arm: "guest entry starts",
        success: "guest entry returns", interrupt: "host cancellation",
        expiry: "interrupt and poison store", reset: "each guest entry", external: false
    },
    WASM_RESIDENT_DISPATCH => {
        id: "wasm.resident_dispatch", owner: "wasm_host_v2", category: FailureDeadline,
        duration: "DispatchConcurrency.dispatch_timeout", arm: "resident-region dispatch starts",
        success: "region reports completion", interrupt: "generation replacement or shutdown",
        expiry: "emit generation-tagged deadline", reset: "new region only", external: false
    },
    DATA_CHANNEL_DRAIN => {
        id: "webrtc.data_channel.drain", owner: "webrtc_connection", category: FailureDeadline,
        duration: "DATA_CHANNEL_DRAIN_TIMEOUT", arm: "connection close waits for buffered drain",
        success: "buffered-low notification", interrupt: "peer connection terminal state",
        expiry: "continue close with residual", reset: "new close only", external: false
    },
    PEER_CONNECTION_CLOSE => {
        id: "webrtc.peer_connection.close", owner: "webrtc_connection", category: FailureDeadline,
        duration: "PEER_CONNECTION_CLOSE_TIMEOUT", arm: "physical peer close starts",
        success: "peer connection closes", interrupt: "connection drop",
        expiry: "return bounded close error", reset: "new connection only", external: false
    },
    WEBRTC_CLEANUP_BARRIER => {
        id: "webrtc.coordinator.cleanup_barrier", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "CLEANUP_BARRIER_WAIT_TIMEOUT", arm: "peer operation waits for cleanup barrier",
        success: "barrier clears", interrupt: "session replacement",
        expiry: "return cleanup timeout", reset: "new guarded operation", external: false
    },
    WEBRTC_CLOSE_FALLBACK => {
        id: "webrtc.coordinator.close_fallback", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "PEER_CONNECTION_CLOSE_FALLBACK_TIMEOUT", arm: "normal close path leaves peer open",
        success: "fallback close completes", interrupt: "peer drop",
        expiry: "drop remaining peer resources", reset: "new peer only", external: false
    },
    WEBRTC_PEER_SENDABLE => {
        id: "webrtc.coordinator.peer_sendable", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "caller readiness timeout", arm: "send waits for current ready session",
        success: "state event plus authoritative ready check", interrupt: "event stream close",
        expiry: "return no sendable session", reset: "new send wait only", external: false
    },
    WEBRTC_INITIAL_READINESS => {
        id: "webrtc.coordinator.initial_readiness", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "ICE and DataChannel readiness budgets", arm: "new peer session awaits readiness",
        success: "retained state or readiness event", interrupt: "terminal event or replacement session",
        expiry: "fail connection attempt", reset: "ICE-connected transition starts DataChannel stage", external: false
    },
    STALE_PEER_EXPIRY => {
        id: "webrtc.coordinator.stale_peer_expiry", owner: "webrtc_coordinator", category: RetentionExpiry,
        duration: "nearest peer-state retention deadline", arm: "peer enters retained non-ready state",
        success: "peer state changes", interrupt: "state-change notification or shutdown",
        expiry: "reap exact stale peer", reset: "material peer-state transition only", external: false
    },
    WEBRTC_CLOSE_ALL_QUIESCE => {
        id: "webrtc.coordinator.close_all_quiesce", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "one CLOSE_ALL_QUIESCE_TIMEOUT deadline", arm: "close-all fences creators",
        success: "all restart/creator tasks quiesce", interrupt: "all tasks complete",
        expiry: "abort remaining tasks and continue teardown", reset: "one close-all operation", external: false
    },
    WEBRTC_CLOSE_ALL_HOOK => {
        id: "webrtc.coordinator.close_all_hook", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "shared CLOSE_ALL_HOOK_TIMEOUT deadline", arm: "close-all invokes peer hooks",
        success: "hook completes", interrupt: "shared overall deadline",
        expiry: "skip unfinished hook and close peer", reset: "one close-all operation", external: false
    },
    WEBRTC_CONNECTION_READY => {
        id: "webrtc.coordinator.connection_ready", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "INITIAL_CONNECTION_TIMEOUT or caller wait budget", arm: "connection attempt awaits ready signal",
        success: "ready oneshot or authoritative state", interrupt: "cancellation token or terminal state",
        expiry: "fail current attempt", reset: "replacement attempt only", external: false
    },
    WEBRTC_CANDIDATE_FLUSH => {
        id: "webrtc.coordinator.candidate_flush", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "REMOTE_CANDIDATE_FLUSH_TIMEOUT", arm: "buffered remote ICE candidates become applicable",
        success: "all current candidates apply", interrupt: "session replacement",
        expiry: "retain/report candidate failure", reset: "new candidate batch", external: false
    },
    WEBRTC_SEND_OVERALL => {
        id: "webrtc.coordinator.send_overall", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "OVERALL_TIMEOUT or caller send budget", arm: "coordinator send/create operation starts",
        success: "operation completes", interrupt: "caller cancellation or close",
        expiry: "return timed out", reset: "new operation only", external: false
    },
    WEBRTC_RETRY_BACKOFF => {
        id: "webrtc.coordinator.retry_backoff", owner: "webrtc_coordinator", category: FailureBackoff,
        duration: "connection/send/ICE retry policy", arm: "retryable WebRTC operation fails",
        success: "absolute retry deadline", interrupt: "shutdown, wake notification, or generation replacement",
        expiry: "retry current generation", reset: "material state transition only", external: false
    },
    WEBRTC_ICE_GATHERING => {
        id: "webrtc.coordinator.ice_gathering", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "remaining ICE gathering/restart budget", arm: "ICE restart waits for gathering",
        success: "gathering-complete notification", interrupt: "restart wake or session replacement",
        expiry: "abort restart attempt", reset: "new restart generation", external: false
    },
    WEBRTC_ICE_RESTART_WAIT => {
        id: "webrtc.coordinator.ice_restart_wait", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "per-attempt restart timeout", arm: "ICE restart attempt awaits committed connected state",
        success: "state event plus authoritative state", interrupt: "restart wake or session replacement",
        expiry: "classify attempt timeout", reset: "new restart attempt", external: false
    },
    ANSWERER_SIGNALING_RECONNECT => {
        id: "webrtc.coordinator.answerer_signaling_reconnect", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "ANSWERER_SIGNALING_RECONNECT_WAIT_TIMEOUT", arm: "answerer restart needs signaling recovery",
        success: "signaling Connected event", interrupt: "session replacement",
        expiry: "fail restart precondition", reset: "new restart attempt", external: false
    },
    WEBRTC_ROLE_NEGOTIATION => {
        id: "webrtc.coordinator.role_negotiation", owner: "webrtc_coordinator", category: FailureDeadline,
        duration: "ROLE_NEGOTIATION_TIMEOUT", arm: "simultaneous connection negotiates role",
        success: "role result channel", interrupt: "peer/session replacement",
        expiry: "fail negotiation", reset: "new negotiation generation", external: false
    },
    SIGNALING_RECONNECT_BACKOFF => {
        id: "signaling.reconnect_backoff", owner: "signaling_client", category: FailureBackoff,
        duration: "ReconnectConfig backoff", arm: "signaling connect fails",
        success: "backoff deadline", interrupt: "explicit reconnect notification or shutdown",
        expiry: "retry current signaling generation", reset: "successful commit or explicit request", external: false
    },
    SIGNALING_RECONNECT_COOLDOWN => {
        id: "signaling.reconnect_cooldown", owner: "signaling_client", category: FailureBackoff,
        duration: "2 * configured maximum delay", arm: "reconnect retry sequence exhausts",
        success: "cooldown deadline", interrupt: "explicit reconnect notification or shutdown",
        expiry: "start another retry sequence", reset: "successful commit", external: false
    },
    SIGNALING_CONNECT => {
        id: "signaling.connect", owner: "signaling_client", category: FailureDeadline,
        duration: "configured connect timeout", arm: "WebSocket connect starts",
        success: "WebSocket handshake", interrupt: "generation invalidation",
        expiry: "fail current connect generation", reset: "new generation only", external: false
    },
    SIGNALING_REQUEST_RESPONSE => {
        id: "signaling.request_response", owner: "signaling_client", category: FailureDeadline,
        duration: "RESPONSE_TIMEOUT_SECS", arm: "signaling request awaits correlated response",
        success: "response oneshot", interrupt: "disconnect or waiter cancellation",
        expiry: "remove waiter and return timeout", reset: "new request only", external: false
    },
    SIGNALING_PING_SCHEDULE => {
        id: "signaling.ping_schedule", owner: "signaling_client", category: ProtocolSchedule,
        duration: "PING_INTERVAL_SECS", arm: "committed signaling generation starts ping task",
        success: "protocol tick", interrupt: "disconnect or generation replacement",
        expiry: "send ping", reset: "new committed generation", external: false
    },
    SIGNALING_SEND => {
        id: "signaling.send", owner: "signaling_client", category: FailureDeadline,
        duration: "SIGNALING_SEND_TIMEOUT_SECS or probe budget", arm: "WebSocket message/ping send starts",
        success: "sink send completes", interrupt: "disconnect or generation replacement",
        expiry: "fail send and trigger recovery", reset: "new send only", external: false
    },
    SIGNALING_DISCONNECT_LOCK => {
        id: "signaling.disconnect_lock", owner: "signaling_client", category: FailureDeadline,
        duration: "DISCONNECT_LOCK_TIMEOUT_SECS", arm: "disconnect acquires task/sink/stream ownership",
        success: "ownership lock acquired", interrupt: "disconnect cancellation",
        expiry: "skip blocked resource and continue teardown", reset: "one disconnect operation", external: false
    },
    SIGNALING_DISCONNECT_CLOSE => {
        id: "signaling.disconnect_close", owner: "signaling_client", category: FailureDeadline,
        duration: "DISCONNECT_CLOSE_TIMEOUT_SECS", arm: "disconnect closes WebSocket sink",
        success: "close frame completes", interrupt: "socket drop",
        expiry: "drop sink and continue teardown", reset: "one signaling generation", external: false
    },
    SIGNALING_CONNECTED_WAIT => {
        id: "signaling.connected_wait", owner: "signaling_client", category: FailureDeadline,
        duration: "caller-provided connected deadline", arm: "caller waits for signaling Connected",
        success: "retained connected state or state event", interrupt: "generation replacement",
        expiry: "return not connected", reset: "new wait only", external: false
    },
    SIGNALING_PROBE => {
        id: "signaling.probe", owner: "signaling_client", category: FailureDeadline,
        duration: "caller probe timeout", arm: "liveness probe sends ping",
        success: "matching pong", interrupt: "disconnect or generation replacement",
        expiry: "report path unreachable", reset: "new probe only", external: false
    },
    WEBSOCKET_REASSEMBLY_EXPIRY => {
        id: "websocket.reassembly_expiry", owner: "websocket_server", category: RetentionExpiry,
        duration: "fixed 100ms scan cadence for legacy chunk assembler", arm: "server owns partial chunks",
        success: "chunk completes", interrupt: "server shutdown",
        expiry: "expire stale partial chunks", reset: "new chunk activity", external: false
    },
    EXTERNAL_ICE_CANDIDATE_SELECTION => {
        id: "external.webrtc.ice_candidate_selection", owner: "webrtc_ice", category: ProtocolSelection,
        duration: "WebRtcAdvancedConfig host/srflx/prflx/relay waits", arm: "SettingEngine is configured",
        success: "candidate meeting selection policy", interrupt: "ICE generation cancellation",
        expiry: "admit next configured candidate class", reset: "new ICE generation", external: true
    },
    EXTERNAL_WASMTIME_EPOCH_TICK => {
        id: "external.wasmtime.epoch_tick", owner: "wasmtime_engine", category: ProtocolSchedule,
        duration: "WasmRuntimeLimits.epoch_tick", arm: "EpochTicker thread starts",
        success: "ticker thread wake", interrupt: "EpochTicker drop/unpark",
        expiry: "increment Wasmtime engine epoch", reset: "fixed configured schedule", external: true
    }
}

fn trace_armed(definition: TimerDefinition, duration: Duration, deadline: Instant) {
    tracing::trace!(
        timer_id = definition.id,
        timer_category = definition.category.as_str(),
        timer_owner = definition.owner,
        ?duration,
        ?deadline,
        "timer armed"
    );
}

fn trace_finished(definition: TimerDefinition, started: Instant, deadline: Instant, expired: bool) {
    tracing::trace!(
        timer_id = definition.id,
        timer_category = definition.category.as_str(),
        timer_owner = definition.owner,
        ?deadline,
        elapsed = ?started.elapsed(),
        expired,
        "timer finished"
    );
}

/// A sleep future that records arm and expiry under its inventory identity.
pub(crate) struct AuditedSleep {
    inner: Pin<Box<tokio::time::Sleep>>,
    definition: TimerDefinition,
    started: Instant,
    deadline: Instant,
    finished: bool,
}

impl AuditedSleep {
    pub(crate) fn reset(mut self: Pin<&mut Self>, deadline: Instant) {
        let this = self.as_mut().get_mut();
        this.started = Instant::now();
        this.deadline = deadline;
        this.finished = false;
        trace_armed(
            this.definition,
            deadline.saturating_duration_since(this.started),
            deadline,
        );
        this.inner.as_mut().reset(deadline);
    }
}

impl Future for AuditedSleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.as_mut().get_mut();
        match this.inner.as_mut().poll(cx) {
            Poll::Ready(()) => {
                if !this.finished {
                    this.finished = true;
                    trace_finished(this.definition, this.started, this.deadline, true);
                }
                Poll::Ready(())
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

pub(crate) fn sleep(definition: TimerDefinition, duration: Duration) -> AuditedSleep {
    let started = Instant::now();
    let deadline = started + duration;
    trace_armed(definition, duration, deadline);
    AuditedSleep {
        inner: Box::pin(tokio::time::sleep_until(deadline)),
        definition,
        started,
        deadline,
        finished: false,
    }
}

pub(crate) fn sleep_until(definition: TimerDefinition, deadline: Instant) -> AuditedSleep {
    let started = Instant::now();
    trace_armed(
        definition,
        deadline.saturating_duration_since(started),
        deadline,
    );
    AuditedSleep {
        inner: Box::pin(tokio::time::sleep_until(deadline)),
        definition,
        started,
        deadline,
        finished: false,
    }
}

pub(crate) async fn timeout<F>(
    definition: TimerDefinition,
    duration: Duration,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed>
where
    F: Future,
{
    let started = Instant::now();
    let deadline = started + duration;
    trace_armed(definition, duration, deadline);
    let result = tokio::time::timeout_at(deadline, future).await;
    trace_finished(definition, started, deadline, result.is_err());
    result
}

pub(crate) async fn timeout_at<F>(
    definition: TimerDefinition,
    deadline: Instant,
    future: F,
) -> Result<F::Output, tokio::time::error::Elapsed>
where
    F: Future,
{
    let started = Instant::now();
    trace_armed(
        definition,
        deadline.saturating_duration_since(started),
        deadline,
    );
    let result = tokio::time::timeout_at(deadline, future).await;
    trace_finished(definition, started, deadline, result.is_err());
    result
}

pub(crate) struct AuditedInterval {
    inner: tokio::time::Interval,
    definition: TimerDefinition,
    period: Duration,
}

impl AuditedInterval {
    pub(crate) fn set_missed_tick_behavior(&mut self, behavior: MissedTickBehavior) {
        self.inner.set_missed_tick_behavior(behavior);
    }

    pub(crate) async fn tick(&mut self) -> Instant {
        let started = Instant::now();
        let tick = self.inner.tick().await;
        trace_finished(self.definition, started, tick, true);
        tick
    }
}

pub(crate) fn interval(definition: TimerDefinition, period: Duration) -> AuditedInterval {
    trace_armed(definition, period, Instant::now() + period);
    AuditedInterval {
        inner: tokio::time::interval(period),
        definition,
        period,
    }
}

impl Drop for AuditedInterval {
    fn drop(&mut self) {
        tracing::trace!(
            timer_id = self.definition.id,
            timer_category = self.definition.category.as_str(),
            timer_owner = self.definition.owner,
            period = ?self.period,
            "timer interval dropped"
        );
    }
}

/// Register an equivalent timer owned by a dependency at its configuration
/// boundary. The dependency still owns the clock; this call makes its policy
/// classification and configured duration observable.
pub(crate) fn register_external(definition: TimerDefinition, duration: Duration) {
    debug_assert!(definition.external);
    tracing::trace!(
        timer_id = definition.id,
        timer_category = definition.category.as_str(),
        timer_owner = definition.owner,
        ?duration,
        "external timer registered"
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;

    fn rust_files(root: &Path, output: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(root).expect("read source directory") {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "test_support") {
                    continue;
                }
                rust_files(&path, output);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("_tests.rs"))
            {
                output.push(path);
            }
        }
    }

    #[test]
    fn inv24_inventory_metadata_is_complete_and_unique() {
        let mut stable_ids = HashSet::new();
        let mut symbols = HashSet::new();
        for entry in ids::ALL {
            assert!(
                stable_ids.insert(entry.id),
                "duplicate timer ID: {}",
                entry.id
            );
            assert!(
                symbols.insert(entry.symbol),
                "duplicate timer symbol: {}",
                entry.symbol
            );
            for (field, value) in [
                ("id", entry.id),
                ("owner", entry.owner),
                ("duration_source", entry.duration_source),
                ("arm_condition", entry.arm_condition),
                ("success_signal", entry.success_signal),
                ("interrupt_source", entry.interrupt_source),
                ("expiry_effect", entry.expiry_effect),
                ("reset_rule", entry.reset_rule),
            ] {
                assert!(
                    !value.trim().is_empty(),
                    "{} missing for {}",
                    field,
                    entry.symbol
                );
            }
        }
    }

    #[test]
    fn inv24_production_timer_calls_and_inventory_do_not_drift() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let facade = source_root.join("timer.rs");
        let mut files = Vec::new();
        rust_files(&source_root, &mut files);

        let forbidden = [
            "tokio::time::sleep(",
            "tokio::time::sleep_until(",
            "tokio::time::interval(",
            "tokio::time::interval_at(",
            "tokio::time::timeout(",
            "tokio::time::timeout_at(",
        ];
        let mut production = String::new();
        let mut violations = Vec::new();
        for path in files {
            if path == facade {
                continue;
            }
            let source = fs::read_to_string(&path).expect("read Rust source");
            for (line_index, line) in source.lines().enumerate() {
                if forbidden.iter().any(|needle| line.contains(needle))
                    && !line.trim_start().starts_with("///")
                    && !line.trim_start().starts_with("//!")
                {
                    violations.push(format!(
                        "{}:{}: {}",
                        path.display(),
                        line_index + 1,
                        line.trim()
                    ));
                }
            }
            production.push_str(&source);
        }
        assert!(
            violations.is_empty(),
            "raw production timers bypass the audited facade:\n{}",
            violations.join("\n")
        );

        for entry in ids::ALL {
            let needle = format!("timer::ids::{}", entry.symbol);
            assert!(
                production.contains(&needle),
                "unused inventory entry {} ({})",
                entry.symbol,
                entry.id
            );
        }
    }
}
