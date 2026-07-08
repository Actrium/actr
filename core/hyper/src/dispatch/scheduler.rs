//! Per-key serial + budgeted-concurrent dispatch scheduler.
//!
//! The scheduler is the single entry point that feeds dispatch work to the B1
//! runner. It owns three invariants:
//!
//! * **Same-key FIFO, one-in-flight**: two jobs with equal [`ConflictKey`] run
//!   strictly in submission order, and the next never starts until the previous
//!   one's reply has resolved.
//! * **Distinct-key concurrency up to budget `C`**: jobs with different keys may
//!   run at the same time, capped at `C` in-flight, with round-robin fairness
//!   across ready keys so no key starves.
//! * **[`ConflictKey::Serial`] = global barrier**: a Serial job starts only when
//!   nothing else is in flight and runs alone; while a Serial job is pending, no
//!   new scoped job starts (anti-starvation).
//!
//! Submission acquires a semaphore permit from a pool of size `M` (queue cap)
//! *before* enqueuing; when the pool is exhausted, `submit` awaits — the
//! back-pressure point that propagates up to the node entry loop.

use super::conflict_key::ConflictKey;
use actr_protocol::{ActorResult, ActrError};
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

/// Boxed, owned dispatch body produced lazily at start time (so nothing runs
/// until the scheduler decides the key is startable).
pub(crate) type DispatchFn =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = ActorResult<Bytes>> + Send>> + Send>;

/// Error surfaced to a submitter when the scheduler is gone (all handles
/// dropped, or explicit shutdown) before its job could complete.
fn scheduler_terminated() -> ActrError {
    ActrError::Unavailable("dispatch scheduler terminated".to_string())
}

struct DispatchJob {
    key: ConflictKey,
    run: DispatchFn,
    reply: oneshot::Sender<ActorResult<Bytes>>,
    permit: OwnedSemaphorePermit,
    span: tracing::Span,
}

/// Cheap, `&self` handle to the scheduler task. Cloned onto the node behind an
/// `Arc`; when the last clone (and the cancel token) drops, the task drains its
/// in-flight jobs and exits, leaving no orphans.
pub(crate) struct SchedulerHandle {
    intake_tx: mpsc::Sender<DispatchJob>,
    slots: Arc<Semaphore>,
    cancel: CancellationToken,
    join: std::sync::Mutex<Option<JoinHandle<()>>>,
    budget: usize,
    queue_cap: usize,
}

impl std::fmt::Debug for SchedulerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchedulerHandle")
            .field("budget", &self.budget)
            .field("queue_cap", &self.queue_cap)
            .finish_non_exhaustive()
    }
}

impl SchedulerHandle {
    /// Spawn a scheduler with in-flight budget `budget` (`C`) and queue capacity
    /// `queue_cap` (`M`, total in-queue + in-flight bound).
    pub(crate) fn spawn(budget: usize, queue_cap: usize) -> Self {
        let budget = budget.max(1);
        let queue_cap = queue_cap.max(budget);
        // A small intake channel; real bounding is the `slots` semaphore.
        let (intake_tx, intake_rx) = mpsc::channel(queue_cap.max(1));
        let slots = Arc::new(Semaphore::new(queue_cap));
        let cancel = CancellationToken::new();
        let task = SchedulerTask::new(intake_rx, budget, cancel.clone());
        let join = tokio::spawn(task.run());
        SchedulerHandle {
            intake_tx,
            slots,
            cancel,
            join: std::sync::Mutex::new(Some(join)),
            budget,
            queue_cap,
        }
    }

    /// Submit one dispatch job. Acquires a queue slot first (awaiting when the
    /// queue is full — the back-pressure point), then enqueues in arrival order
    /// and returns a receiver that resolves with the dispatch result (or
    /// [`scheduler_terminated`] if the scheduler exits first).
    pub(crate) async fn submit(
        &self,
        key: ConflictKey,
        run: DispatchFn,
    ) -> oneshot::Receiver<ActorResult<Bytes>> {
        let (reply, rx) = oneshot::channel();
        // Acquire a slot: fast path, else await (back-pressure).
        let permit = match self.slots.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                tracing::debug!(
                    budget = self.budget,
                    queue_cap = self.queue_cap,
                    "dispatch scheduler queue full; submit awaiting a slot (back-pressure)"
                );
                match self.slots.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => {
                        // Semaphore closed → scheduler gone.
                        let _ = reply.send(Err(scheduler_terminated()));
                        return rx;
                    }
                }
            }
        };
        let job = DispatchJob {
            key,
            run,
            reply,
            permit,
            span: tracing::Span::current(),
        };
        if self.intake_tx.send(job).await.is_err() {
            // Scheduler gone; the job (and its reply sender) is returned in the
            // SendError and dropped here, so `rx` observes a closed channel. We
            // additionally cannot re-send, so surface terminated via a fresh
            // receiver would lose ordering — instead rely on rx erroring, which
            // the node maps to `scheduler_terminated`.
        }
        rx
    }

    /// Explicit, ordered teardown for tests: cancel intake, let in-flight jobs
    /// finish, then join the task.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn shutdown(&self) {
        self.cancel.cancel();
        self.join().await;
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn join(&self) {
        let handle = self.join.lock().unwrap().take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }

    #[cfg(test)]
    pub(crate) fn budget(&self) -> usize {
        self.budget
    }
}

/// A completed in-flight job yields its key so the scheduler can re-evaluate.
type Completion = Pin<Box<dyn Future<Output = ConflictKey> + Send>>;

struct SchedulerTask {
    intake_rx: mpsc::Receiver<DispatchJob>,
    cancel: CancellationToken,
    budget: usize,
    /// Per-key FIFO queues of pending jobs (includes the `Serial` root key).
    queues: HashMap<ConflictKey, VecDeque<DispatchJob>>,
    /// Scoped keys with pending work that are not currently active — round-robin
    /// order. `Serial` is never placed here; it is handled by the barrier rule.
    ready: VecDeque<ConflictKey>,
    /// Keys with a job currently in flight (≤ 1 per key).
    active: HashSet<ConflictKey>,
    inflight: FuturesUnordered<Completion>,
}

impl SchedulerTask {
    fn new(
        intake_rx: mpsc::Receiver<DispatchJob>,
        budget: usize,
        cancel: CancellationToken,
    ) -> Self {
        SchedulerTask {
            intake_rx,
            cancel,
            budget,
            queues: HashMap::new(),
            ready: VecDeque::new(),
            active: HashSet::new(),
            inflight: FuturesUnordered::new(),
        }
    }

    async fn run(mut self) {
        let mut intake_open = true;
        loop {
            tokio::select! {
                biased;
                _ = self.cancel.cancelled(), if intake_open => {
                    intake_open = false;
                    self.drop_queued();
                }
                maybe = self.intake_rx.recv(), if intake_open => {
                    match maybe {
                        Some(job) => {
                            self.enqueue(job);
                            self.pump();
                        }
                        None => {
                            intake_open = false;
                            self.drop_queued();
                        }
                    }
                }
                Some(key) = self.inflight.next(), if !self.inflight.is_empty() => {
                    self.on_complete(key);
                }
                else => break,
            }
        }
    }

    fn enqueue(&mut self, job: DispatchJob) {
        let key = job.key.clone();
        let q = self.queues.entry(key.clone()).or_default();
        let was_empty = q.is_empty();
        q.push_back(job);
        // A scoped key that just gained its first pending job and is not active
        // becomes eligible for round-robin.
        if was_empty && !key.is_serial() && !self.active.contains(&key) {
            self.ready.push_back(key);
        }
    }

    /// Start as many startable jobs as budget and the barrier rules allow.
    fn pump(&mut self) {
        // Serial barrier: a Serial job in flight runs alone.
        if self.active.contains(&ConflictKey::Serial) {
            return;
        }
        let serial_pending = self
            .queues
            .get(&ConflictKey::Serial)
            .is_some_and(|q| !q.is_empty());
        if serial_pending {
            // Serial may start only when nothing else is in flight; until then
            // block all scoped starts so the barrier cannot starve.
            if self.inflight.is_empty() {
                self.start(ConflictKey::Serial);
            }
            return;
        }
        // Scoped round-robin up to budget.
        while self.inflight.len() < self.budget {
            let Some(key) = self.ready.pop_front() else {
                break;
            };
            self.start(key);
        }
    }

    fn start(&mut self, key: ConflictKey) {
        let Some(q) = self.queues.get_mut(&key) else {
            return;
        };
        let Some(job) = q.pop_front() else {
            return;
        };
        self.active.insert(key.clone());
        let DispatchJob {
            key: _,
            run,
            reply,
            permit,
            span,
        } = job;
        let completed_key = key;
        self.inflight.push(Box::pin(async move {
            let result = run().instrument(span).await;
            let _ = reply.send(result);
            drop(permit);
            completed_key
        }));
    }

    fn on_complete(&mut self, key: ConflictKey) {
        self.active.remove(&key);
        // Drop the key's queue entry if empty; else re-arm it.
        let still_pending = self.queues.get(&key).is_some_and(|q| !q.is_empty());
        if !still_pending {
            self.queues.remove(&key);
        } else if !key.is_serial() {
            self.ready.push_back(key);
        }
        self.pump();
    }

    /// On shutdown / intake close: drop every queued (not yet started) job.
    /// Their reply senders drop with them, so submitters observe a closed
    /// channel → `scheduler_terminated`. In-flight jobs are left to finish.
    fn drop_queued(&mut self) {
        self.queues.clear();
        self.ready.clear();
    }
}
