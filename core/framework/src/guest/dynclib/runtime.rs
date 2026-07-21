//! Tokio runtime owned by one loaded dynclib workload image.

use std::future::Future;
use std::sync::{Mutex, MutexGuard};

use actr_protocol::{ActorResult, ActrError};
use tokio::runtime::{Handle, Runtime};
use tokio::task::JoinHandle;

tokio::task_local! {
    static ACTIVE_BRIDGE_TOKEN: u64;
}

struct ManagedTasks {
    accepting: bool,
    handles: Vec<JoinHandle<()>>,
}

struct DynclibRuntime {
    handle: Handle,
    owner: Option<Runtime>,
    tasks: ManagedTasks,
}

// Per shared-library image. A `Mutex<Option<...>>` (rather than `OnceLock`)
// lets `shutdown` clear the slot so a subsequent `initialize` can rebuild a
// fresh runtime. `OnceLock` can never be reset, so when a host loads → inits →
// shuts down → unloads → reloads the same image within one process — as the
// integration tests do, and as glibc may do without truly unmapping the
// library on `dlclose` — the next `actr_init` sees the stale runtime and
// returns `INIT_FAILED`.
static RUNTIME: Mutex<Option<DynclibRuntime>> = Mutex::new(None);

fn lock() -> ActorResult<MutexGuard<'static, Option<DynclibRuntime>>> {
    RUNTIME
        .lock()
        .map_err(|_| ActrError::Internal("dynclib runtime lock poisoned".into()))
}

/// Initialize the runtime for this shared-library image.
pub fn initialize() -> ActorResult<()> {
    let mut guard = lock()?;
    if guard.is_some() {
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
    *guard = Some(DynclibRuntime {
        handle: runtime.handle().clone(),
        owner: Some(runtime),
        tasks: ManagedTasks {
            accepting: true,
            handles: Vec::new(),
        },
    });
    Ok(())
}

/// Drive a guest future to completion with the invocation's bridge token.
pub fn block_on<F>(bridge_token: u64, future: F) -> ActorResult<F::Output>
where
    F: Future,
{
    // Clone the handle and drop the guard before `block_on`: the driven future
    // may call `spawn`, which also locks `RUNTIME`, so holding the guard here
    // would deadlock the runtime worker.
    let handle = {
        let guard = lock()?;
        let Some(runtime) = guard.as_ref() else {
            return Err(ActrError::Internal(
                "dynclib Tokio runtime is not initialized".into(),
            ));
        };
        if runtime.owner.is_none() {
            return Err(ActrError::Internal(
                "dynclib Tokio runtime is shut down".into(),
            ));
        }
        runtime.handle.clone()
    };

    Ok(handle.block_on(ACTIVE_BRIDGE_TOKEN.scope(bridge_token, future)))
}

/// Spawn a background task owned by the current dynclib workload.
///
/// Managed tasks are aborted and joined before the shared library is unloaded.
/// Clone any [`crate::Context`] used by the task before passing it here; the
/// cloned context retains its host bridge until the task exits.
pub fn spawn<F>(future: F) -> ActorResult<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut guard = lock()?;
    let Some(runtime) = guard.as_mut() else {
        return Err(ActrError::Internal(
            "dynclib Tokio runtime is not initialized".into(),
        ));
    };
    if !runtime.tasks.accepting {
        return Err(ActrError::Unavailable(
            "dynclib workload is shutting down".into(),
        ));
    }

    runtime.tasks.handles.retain(|handle| !handle.is_finished());
    runtime.tasks.handles.push(runtime.handle.spawn(future));
    Ok(())
}

pub(crate) fn active_bridge_token() -> Option<u64> {
    ACTIVE_BRIDGE_TOKEN.try_with(|token| *token).ok()
}

/// Stop managed tasks and shut down the runtime before `dlclose`.
pub fn shutdown() -> ActorResult<()> {
    // Take the runtime out of the global slot. This clears the cell so the
    // next `initialize` can rebuild a fresh runtime even if the shared library
    // is not actually unmapped by `dlclose` (e.g. glibc keeping the mapping).
    let mut runtime = match lock()?.take() {
        Some(runtime) => runtime,
        None => return Ok(()),
    };

    runtime.tasks.accepting = false;
    let handles = std::mem::take(&mut runtime.tasks.handles);

    // Abort/join managed tasks while the runtime is still owned, then drop the
    // `Runtime` so its worker threads exit before the host calls `dlclose`.
    runtime.handle.block_on(async move {
        for handle in &handles {
            handle.abort();
        }
        for handle in handles {
            let _ = handle.await;
        }
    });

    runtime.owner = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // The runtime cell is process-global; serialise these tests so they do not
    // race with each other over `RUNTIME`.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// `shutdown` must clear the cell so the next `initialize` can rebuild the
    /// runtime. With `OnceLock` the cell never reset, so a second `initialize`
    /// after `shutdown` saw stale state and failed — surfacing as `actr_init`
    /// returning `INIT_FAILED` when a host reloaded a dynclib image within one
    /// process (e.g. glibc keeping the mapping after `dlclose`).
    #[test]
    fn shutdown_clears_cell_for_reinitialize() {
        let _guard = SERIAL.lock().unwrap();
        initialize().expect("first initialize");
        // A second initialize while running is still rejected.
        assert!(
            initialize().is_err(),
            "initialize should fail while already running"
        );
        shutdown().expect("shutdown");

        // After shutdown the cell must be reusable.
        initialize().expect("reinitialize after shutdown");
        shutdown().expect("second shutdown");
    }

    /// `shutdown` is idempotent: calling it with no active runtime is a no-op.
    #[test]
    fn shutdown_without_runtime_is_noop() {
        let _guard = SERIAL.lock().unwrap();
        // Drain anything left by other tests so this assertion is meaningful.
        let _ = shutdown();
        assert!(shutdown().is_ok());
    }
}
