//! Tokio runtime owned by one loaded dynclib workload image.
//!
//! Runtime-driving entrypoints are serialized here as well as by Hyper's host
//! FFI gate. Shutdown gives managed tasks five seconds to observe cancellation;
//! on timeout it returns an error and leaves the runtime in a terminal leaked
//! state so the host can keep the library mapped instead of blocking forever.

use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use actr_protocol::{ActorResult, ActrError};
use tokio::runtime::{Handle, Runtime};
use tokio::task::JoinHandle;

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(10);

tokio::task_local! {
    static ACTIVE_BRIDGE_TOKEN: u64;
}

struct ManagedTasks {
    accepting: bool,
    handles: Vec<JoinHandle<()>>,
}

enum RuntimeOwner {
    Running(Runtime),
    ShutDown,
    ShutdownTimedOut,
}

struct DynclibRuntime {
    handle: Handle,
    /// Serializes guest entrypoints that drive or tear down the runtime.
    ///
    /// Hyper also serializes `actr_handle` and `actr_shutdown` with its
    /// `ffi_gate`, but keeping the invariant here closes the check-then-act
    /// window for callers that invoke this public guest API directly.
    entry_gate: Mutex<()>,
    owner: Mutex<RuntimeOwner>,
    tasks: Mutex<ManagedTasks>,
}

static RUNTIME: OnceLock<DynclibRuntime> = OnceLock::new();

fn runtime() -> ActorResult<&'static DynclibRuntime> {
    RUNTIME
        .get()
        .ok_or_else(|| ActrError::Internal("dynclib Tokio runtime is not initialized".into()))
}

/// Initialize the runtime for this shared-library image.
pub fn initialize() -> ActorResult<()> {
    if RUNTIME.get().is_some() {
        return Err(ActrError::Internal(
            "dynclib Tokio runtime is already initialized".into(),
        ));
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name("actr-dynclib")
        .build()
        .map_err(|error| {
            ActrError::Internal(format!(
                "failed to initialize dynclib Tokio runtime: {error}"
            ))
        })?;
    let state = DynclibRuntime {
        handle: runtime.handle().clone(),
        entry_gate: Mutex::new(()),
        owner: Mutex::new(RuntimeOwner::Running(runtime)),
        tasks: Mutex::new(ManagedTasks {
            accepting: true,
            handles: Vec::new(),
        }),
    };

    RUNTIME
        .set(state)
        .map_err(|_| ActrError::Internal("dynclib Tokio runtime is already initialized".into()))
}

/// Drive a guest future to completion with the invocation's bridge token.
pub fn block_on<F>(bridge_token: u64, future: F) -> ActorResult<F::Output>
where
    F: Future,
{
    let runtime = runtime()?;
    let _entry_guard = runtime
        .entry_gate
        .lock()
        .map_err(|_| ActrError::Internal("dynclib runtime entry lock poisoned".into()))?;
    let owner = runtime
        .owner
        .lock()
        .map_err(|_| ActrError::Internal("dynclib runtime owner lock poisoned".into()))?;
    match &*owner {
        RuntimeOwner::Running(_) => {}
        RuntimeOwner::ShutDown => {
            return Err(ActrError::Internal(
                "dynclib Tokio runtime is shut down".into(),
            ));
        }
        RuntimeOwner::ShutdownTimedOut => {
            return Err(ActrError::Internal(
                "dynclib Tokio runtime shutdown previously timed out".into(),
            ));
        }
    }
    drop(owner);

    Ok(runtime
        .handle
        .block_on(ACTIVE_BRIDGE_TOKEN.scope(bridge_token, future)))
}

/// Spawn a background task owned by the current dynclib workload.
///
/// Managed tasks are aborted and given a bounded interval to finish before the
/// shared library is unloaded. A task that does not yield causes shutdown to
/// fail, and a conforming host keeps the library mapped for process lifetime.
/// Clone any [`crate::Context`] used by the task before passing it here; the
/// cloned context retains its host bridge until the task exits.
pub fn spawn<F>(future: F) -> ActorResult<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let runtime = runtime()?;
    let mut tasks = runtime
        .tasks
        .lock()
        .map_err(|_| ActrError::Internal("dynclib task registry lock poisoned".into()))?;
    if !tasks.accepting {
        return Err(ActrError::Unavailable(
            "dynclib workload is shutting down".into(),
        ));
    }

    tasks.handles.retain(|handle| !handle.is_finished());
    tasks.handles.push(runtime.handle.spawn(future));
    Ok(())
}

pub(crate) fn active_bridge_token() -> Option<u64> {
    ACTIVE_BRIDGE_TOKEN.try_with(|token| *token).ok()
}

fn abort_and_wait(handles: Vec<JoinHandle<()>>, timeout: Duration) -> Result<(), usize> {
    for handle in &handles {
        handle.abort();
    }

    let deadline = Instant::now() + timeout;
    loop {
        let unfinished = handles
            .iter()
            .filter(|handle| !handle.is_finished())
            .count();
        if unfinished == 0 {
            return Ok(());
        }

        let now = Instant::now();
        if now >= deadline {
            return Err(unfinished);
        }
        std::thread::sleep(SHUTDOWN_POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

/// Stop managed tasks and shut down the runtime before `dlclose`.
///
/// A non-cooperative task is detached after [`SHUTDOWN_TIMEOUT`], reported as
/// an error, and deliberately leaves the runtime unusable. The host must treat
/// that error as "do not unload this library".
pub fn shutdown() -> ActorResult<()> {
    let runtime = runtime()?;
    let _entry_guard = runtime
        .entry_gate
        .lock()
        .map_err(|_| ActrError::Internal("dynclib runtime entry lock poisoned".into()))?;
    let mut owner = runtime
        .owner
        .lock()
        .map_err(|_| ActrError::Internal("dynclib runtime owner lock poisoned".into()))?;
    match &*owner {
        RuntimeOwner::Running(_) => {}
        RuntimeOwner::ShutDown => return Ok(()),
        RuntimeOwner::ShutdownTimedOut => {
            return Err(ActrError::Internal(
                "dynclib Tokio runtime shutdown previously timed out".into(),
            ));
        }
    }

    let handles = {
        let mut tasks = runtime
            .tasks
            .lock()
            .map_err(|_| ActrError::Internal("dynclib task registry lock poisoned".into()))?;
        tasks.accepting = false;
        std::mem::take(&mut tasks.handles)
    };

    match abort_and_wait(handles, SHUTDOWN_TIMEOUT) {
        Ok(()) => {
            let RuntimeOwner::Running(runtime_owner) =
                std::mem::replace(&mut *owner, RuntimeOwner::ShutDown)
            else {
                unreachable!("dynclib runtime state changed while entry gate was held");
            };
            drop(owner);
            drop(runtime_owner);
            Ok(())
        }
        Err(unfinished) => {
            let RuntimeOwner::Running(runtime_owner) =
                std::mem::replace(&mut *owner, RuntimeOwner::ShutdownTimedOut)
            else {
                unreachable!("dynclib runtime state changed while entry gate was held");
            };
            drop(owner);

            tracing::error!(
                unfinished_tasks = unfinished,
                timeout_ms = SHUTDOWN_TIMEOUT.as_millis(),
                "dynclib shutdown timed out; leaking the guest runtime and requiring the host to retain the library"
            );
            runtime_owner.shutdown_background();
            Err(ActrError::Internal(format!(
                "dynclib shutdown timed out after {} ms with {unfinished} managed task(s) still running",
                SHUTDOWN_TIMEOUT.as_millis()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_and_wait_drains_cancellable_task() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build test runtime");
        let task = runtime.spawn(std::future::pending::<()>());

        assert_eq!(abort_and_wait(vec![task], Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn abort_and_wait_times_out_non_yielding_task() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build test runtime");
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(1);
        let task = runtime.spawn(async move {
            started_tx.send(()).expect("signal task start");
            std::thread::sleep(Duration::from_millis(100));
        });
        started_rx.recv().expect("wait for task start");

        assert_eq!(
            abort_and_wait(vec![task], Duration::from_millis(10)),
            Err(1)
        );
        runtime.shutdown_timeout(Duration::from_secs(1));
    }
}
