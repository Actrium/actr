//! Unit tests for the per-actor serial command runner.
//!
//! These drive the runner through a gating [`LinkedWorkloadHandle`] so we can
//! observe the in-flight count and command ordering deterministically. No
//! `sleep` is used for synchronization: entry/release handshakes plus a
//! `tokio::time::timeout` watchdog make every wait condition explicit.

use super::*;
use crate::context::RuntimeContext;
use crate::workload::{
    HostAbiFn, HostOperation, HostOperationResult, InvocationContext, LinkedWorkloadHandle,
    Workload,
};
use actr_protocol::{ActorResult, ActrError, ActrId, RpcEnvelope};
use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, Semaphore, mpsc};

const WATCHDOG: Duration = Duration::from_secs(5);

/// Shared observation + gating state for the test workload handle.
struct Gate {
    inflight: AtomicUsize,
    max_inflight: AtomicUsize,
    /// Push each entry tag in the order bodies actually start running.
    entry_order: AsyncMutex<Vec<u64>>,
    /// A body signals here when it has entered (incremented in-flight).
    entered: mpsc::UnboundedSender<u64>,
    /// A body waits here before finishing; main adds one permit per release.
    release: Semaphore,
}

impl Gate {
    async fn enter(&self, tag: u64) {
        let cur = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_inflight.fetch_max(cur, Ordering::SeqCst);
        self.entry_order.lock().await.push(tag);
        let _ = self.entered.send(tag);
        // Gate: block until main releases exactly this body.
        let permit = self.release.acquire().await.expect("semaphore open");
        permit.forget();
        self.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Detects that the runner task dropped the workload (and its handle) when the
/// channel closed — i.e. no orphaned runner.
struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);
impl Drop for DropSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

/// Linked handle whose lifecycle + dispatch entrypoints all funnel through the
/// same [`Gate`], so every command kind is observed on one serialization axis.
struct GateHandle {
    gate: Arc<Gate>,
    tag: AtomicUsize,
    _drop: DropSignal,
}

impl GateHandle {
    fn next_tag(&self) -> u64 {
        self.tag.fetch_add(1, Ordering::SeqCst) as u64
    }
}

#[async_trait]
impl LinkedWorkloadHandle for GateHandle {
    async fn on_start(&self, _ctx: &RuntimeContext) -> ActorResult<()> {
        self.gate.enter(self.next_tag()).await;
        Ok(())
    }
    async fn on_ready(&self, _ctx: &RuntimeContext) -> ActorResult<()> {
        self.gate.enter(self.next_tag()).await;
        Ok(())
    }
    async fn on_stop(&self, _ctx: &RuntimeContext) -> ActorResult<()> {
        self.gate.enter(self.next_tag()).await;
        Ok(())
    }
    async fn dispatch(
        &self,
        _envelope: RpcEnvelope,
        _ctx: Arc<RuntimeContext>,
    ) -> ActorResult<bytes::Bytes> {
        self.gate.enter(self.next_tag()).await;
        Ok(bytes::Bytes::new())
    }
}

fn test_ctx() -> RuntimeContext {
    crate::test_support::runtime_context_with_host_transport(
        ActrId::default(),
        Arc::new(crate::transport::HostTransport::new()),
    )
}

fn test_invocation() -> InvocationContext {
    InvocationContext {
        self_id: ActrId::default(),
        caller_id: None,
        request_id: "executor-test".to_string(),
    }
}

fn noop_host_abi() -> HostAbiFn {
    Arc::new(|_op: HostOperation| {
        Box::pin(async move { HostOperationResult::Done })
            as std::pin::Pin<Box<dyn std::future::Future<Output = HostOperationResult> + Send>>
    })
}

/// Build a gated runner. Returns the handle, the shared gate, and the runner's
/// drop-signal receiver.
fn gated_runner() -> (
    ActorHandle,
    Arc<Gate>,
    mpsc::UnboundedReceiver<u64>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (entered_tx, entered_rx) = mpsc::unbounded_channel();
    let gate = Arc::new(Gate {
        inflight: AtomicUsize::new(0),
        max_inflight: AtomicUsize::new(0),
        entry_order: AsyncMutex::new(Vec::new()),
        entered: entered_tx,
        release: Semaphore::new(0),
    });
    let (drop_tx, drop_rx) = tokio::sync::oneshot::channel();
    let handle = GateHandle {
        gate: gate.clone(),
        tag: AtomicUsize::new(0),
        _drop: DropSignal(Some(drop_tx)),
    };
    let workload = Workload::Linked(Arc::new(handle) as Arc<dyn LinkedWorkloadHandle>);
    (spawn_runner(workload), gate, entered_rx, drop_rx)
}

/// Directly enqueue a lifecycle `on_start` command, returning its reply
/// receiver without awaiting. Uses the parent module's private channel — a
/// child module may reach ancestor privates.
fn enqueue_on_start(handle: &ActorHandle) -> tokio::sync::oneshot::Receiver<ActorResult<()>> {
    let (reply, rx) = tokio::sync::oneshot::channel();
    let cmd = ActorCmd::Lifecycle {
        phase: LifecyclePhase::OnStart,
        ctx: test_ctx(),
        invocation: test_invocation(),
        host_abi: noop_host_abi(),
        span: tracing::Span::none(),
        reply,
    };
    handle.tx.try_send(cmd).expect("queue has capacity");
    rx
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn strict_serial_across_concurrent_submissions() {
    let (handle, gate, mut entered_rx, _drop_rx) = gated_runner();

    // Enqueue 8 commands in a known FIFO order (single task, sequential sends).
    let mut receivers = Vec::new();
    for _ in 0..8u64 {
        receivers.push(enqueue_on_start(&handle));
    }

    // Release one at a time; assert only ever one body is in-flight and that
    // bodies enter in FIFO submission order.
    for expected in 0..8u64 {
        let tag = tokio::time::timeout(WATCHDOG, entered_rx.recv())
            .await
            .expect("watchdog: body did not enter")
            .expect("entered channel open");
        assert_eq!(tag, expected, "bodies must enter in FIFO order");
        assert_eq!(
            gate.inflight.load(Ordering::SeqCst),
            1,
            "runner must never run two bodies at once"
        );
        assert_eq!(gate.max_inflight.load(Ordering::SeqCst), 1);
        gate.release.add_permits(1);
    }

    for rx in receivers {
        let res = tokio::time::timeout(WATCHDOG, rx)
            .await
            .expect("watchdog: reply")
            .expect("runner alive");
        assert!(res.is_ok());
    }
    assert_eq!(gate.max_inflight.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mixed_cmd_kinds_serialize() {
    let (handle, gate, mut entered_rx, _drop_rx) = gated_runner();

    // Interleave dispatch / lifecycle commands; all share the one channel.
    let (d_reply, d_rx) = tokio::sync::oneshot::channel();
    handle
        .tx
        .try_send(ActorCmd::Dispatch {
            envelope: RpcEnvelope::default(),
            ctx: test_ctx(),
            invocation: test_invocation(),
            host_abi: noop_host_abi(),
            span: tracing::Span::none(),
            reply: d_reply,
        })
        .expect("capacity");
    let l1 = enqueue_on_start(&handle);
    let l2 = enqueue_on_start(&handle);

    for _ in 0..3 {
        tokio::time::timeout(WATCHDOG, entered_rx.recv())
            .await
            .expect("watchdog: entered")
            .expect("open");
        assert_eq!(gate.inflight.load(Ordering::SeqCst), 1);
        gate.release.add_permits(1);
    }

    assert!(
        tokio::time::timeout(WATCHDOG, d_rx)
            .await
            .expect("watchdog")
            .expect("alive")
            .is_ok()
    );
    for rx in [l1, l2] {
        assert!(
            tokio::time::timeout(WATCHDOG, rx)
                .await
                .expect("watchdog")
                .expect("alive")
                .is_ok()
        );
    }
    assert_eq!(gate.max_inflight.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_cmd_exits_cleanly_and_rejects_new_work() {
    let (handle, _gate, _entered_rx, _drop_rx) = gated_runner();

    tokio::time::timeout(WATCHDOG, handle.shutdown())
        .await
        .expect("watchdog: shutdown join");

    // After shutdown the channel is closed; new work reports Unavailable.
    let err = handle
        .on_start(test_ctx(), test_invocation(), &noop_host_abi())
        .await
        .expect_err("runner is gone");
    assert!(matches!(err, ActrError::Unavailable(_)), "got {err:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_all_handles_closes_runner_without_orphan() {
    let (handle, _gate, _entered_rx, drop_rx) = gated_runner();

    // Dropping the only handle closes the channel; the runner loop ends and
    // drops the workload (firing DropSignal).
    drop(handle);

    tokio::time::timeout(WATCHDOG, drop_rx)
        .await
        .expect("watchdog: runner did not exit / workload orphaned")
        .expect("drop signal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_cmds_after_shutdown_get_no_reply() {
    let (handle, gate, mut entered_rx, _drop_rx) = gated_runner();

    // A occupies the runner (gated).
    let rx_a = enqueue_on_start(&handle);
    let _ = tokio::time::timeout(WATCHDOG, entered_rx.recv())
        .await
        .expect("watchdog: A entered")
        .expect("open");

    // Enqueue Shutdown BEFORE B, so B sits behind the break point.
    handle
        .tx
        .try_send(ActorCmd::Shutdown { done: None })
        .expect("capacity");
    let rx_b = enqueue_on_start(&handle);

    // Release A; runner finishes A, hits Shutdown, breaks — B is dropped.
    gate.release.add_permits(1);

    assert!(
        tokio::time::timeout(WATCHDOG, rx_a)
            .await
            .expect("watchdog: A reply")
            .expect("A completed")
            .is_ok()
    );
    // B's reply sender was dropped without sending → receiver errors. In the
    // real path ActorHandle::call maps this to ActrError::Unavailable.
    let b = tokio::time::timeout(WATCHDOG, rx_b)
        .await
        .expect("watchdog: B resolves");
    assert!(
        b.is_err(),
        "queued-behind-shutdown command must get no reply"
    );
}

// ── Interleaved runner (B2 native concurrency) ───────────────────────────────

/// Build a gated runner in `Interleaved` mode (native `Linked` concurrency).
fn gated_runner_interleaved() -> (
    ActorHandle,
    Arc<Gate>,
    mpsc::UnboundedReceiver<u64>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (entered_tx, entered_rx) = mpsc::unbounded_channel();
    let gate = Arc::new(Gate {
        inflight: AtomicUsize::new(0),
        max_inflight: AtomicUsize::new(0),
        entry_order: AsyncMutex::new(Vec::new()),
        entered: entered_tx,
        release: Semaphore::new(0),
    });
    let (drop_tx, drop_rx) = tokio::sync::oneshot::channel();
    let handle = GateHandle {
        gate: gate.clone(),
        tag: AtomicUsize::new(0),
        _drop: DropSignal(Some(drop_tx)),
    };
    let workload = Workload::Linked(Arc::new(handle) as Arc<dyn LinkedWorkloadHandle>);
    (
        spawn_runner_with_mode(workload, RunnerMode::Interleaved, None),
        gate,
        entered_rx,
        drop_rx,
    )
}

fn enqueue_dispatch(
    handle: &ActorHandle,
    request_id: &str,
) -> tokio::sync::oneshot::Receiver<ActorResult<bytes::Bytes>> {
    let (reply, rx) = tokio::sync::oneshot::channel();
    let envelope = RpcEnvelope {
        request_id: request_id.to_string(),
        ..RpcEnvelope::default()
    };
    let cmd = ActorCmd::Dispatch {
        envelope,
        ctx: test_ctx(),
        invocation: test_invocation(),
        host_abi: noop_host_abi(),
        span: tracing::Span::none(),
        reply,
    };
    handle.tx.try_send(cmd).expect("queue has capacity");
    rx
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interleaved_dispatches_run_concurrently() {
    let (handle, gate, mut entered_rx, _drop_rx) = gated_runner_interleaved();

    let rx1 = enqueue_dispatch(&handle, "d1");
    let rx2 = enqueue_dispatch(&handle, "d2");

    // Both dispatch bodies must be in flight at the same time.
    for _ in 0..2 {
        tokio::time::timeout(WATCHDOG, entered_rx.recv())
            .await
            .expect("watchdog: dispatch entered")
            .expect("open");
    }
    assert_eq!(
        gate.max_inflight.load(Ordering::SeqCst),
        2,
        "interleaved mode must run two dispatches concurrently"
    );

    gate.release.add_permits(2);
    assert!(
        tokio::time::timeout(WATCHDOG, rx1)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    assert!(
        tokio::time::timeout(WATCHDOG, rx2)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interleaved_lifecycle_is_a_barrier() {
    let (handle, gate, mut entered_rx, _drop_rx) = gated_runner_interleaved();

    // One dispatch in flight (gated).
    let rx_d = enqueue_dispatch(&handle, "d1");
    let tag = tokio::time::timeout(WATCHDOG, entered_rx.recv())
        .await
        .expect("watchdog: dispatch entered")
        .expect("open");
    assert_eq!(tag, 0);

    // A lifecycle command is a barrier: it must not enter while a dispatch is
    // still in flight.
    let rx_l = enqueue_on_start(&handle);
    assert!(
        matches!(entered_rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
        "lifecycle must wait for the in-flight dispatch to drain"
    );

    // Drain the dispatch → the barrier runs alone.
    gate.release.add_permits(1);
    assert!(
        tokio::time::timeout(WATCHDOG, rx_d)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    let l_tag = tokio::time::timeout(WATCHDOG, entered_rx.recv())
        .await
        .expect("watchdog: lifecycle entered after drain")
        .expect("open");
    assert_eq!(l_tag, 1, "lifecycle runs after the dispatch completes");
    assert_eq!(
        gate.inflight.load(Ordering::SeqCst),
        1,
        "barrier runs alone"
    );
    gate.release.add_permits(1);
    assert!(
        tokio::time::timeout(WATCHDOG, rx_l)
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
}

/// Linked handle whose dispatch panics for a specific request id, so we can
/// prove sibling isolation under interleaving.
struct SelectivePanicHandle;

#[async_trait]
impl LinkedWorkloadHandle for SelectivePanicHandle {
    async fn dispatch(
        &self,
        envelope: RpcEnvelope,
        _ctx: Arc<RuntimeContext>,
    ) -> ActorResult<bytes::Bytes> {
        if envelope.request_id == "boom" {
            panic!("intentional dispatch panic");
        }
        Ok(bytes::Bytes::from_static(b"ok"))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn interleaved_panic_isolated_from_siblings() {
    let workload =
        Workload::Linked(Arc::new(SelectivePanicHandle) as Arc<dyn LinkedWorkloadHandle>);
    let handle = spawn_runner_with_mode(workload, RunnerMode::Interleaved, None);

    let rx_boom = enqueue_dispatch(&handle, "boom");
    let rx_ok = enqueue_dispatch(&handle, "fine");

    // The panicking dispatch yields an Internal error; the sibling still succeeds.
    let boom = tokio::time::timeout(WATCHDOG, rx_boom)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(boom, Err(ActrError::Internal(_))), "got {boom:?}");
    let ok = tokio::time::timeout(WATCHDOG, rx_ok)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.unwrap(), bytes::Bytes::from_static(b"ok"));

    // Runner survives a panic: a third dispatch still completes.
    let rx_after = enqueue_dispatch(&handle, "after");
    let after = tokio::time::timeout(WATCHDOG, rx_after)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.unwrap(), bytes::Bytes::from_static(b"ok"));
}
