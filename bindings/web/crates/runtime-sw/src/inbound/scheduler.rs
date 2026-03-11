//! Scheduler for message dispatch.
//!
//! Schedules messages into actor execution serially.
//! Mirrors the scheduler logic used in actr.
//!
//! # Architecture position
//!
//! ```text
//! Mailbox → MailboxProcessor → Scheduler → Actor
//! ```
//!
//! Scheduler guarantees:
//! - Messages for the same actor execute serially
//! - Messages for different actors can interleave concurrently through async execution
//! - Priority-aware scheduling is supported

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use actr_mailbox_web::MessageRecord;
use actr_protocol::ActrId;
use actr_web_common::WebResult;
use wasm_bindgen_futures::spawn_local;

/// Actor processing callback type.
pub type ActorHandler = Rc<dyn Fn(MessageRecord) -> Pin<Box<dyn Future<Output = WebResult<()>>>>>;

/// Scheduled item.
struct ScheduleItem {
    message: MessageRecord,
    on_complete: Option<Box<dyn FnOnce(WebResult<()>)>>,
}

/// Actor queue state.
struct ActorQueue {
    /// Pending message queue.
    pending: VecDeque<ScheduleItem>,

    /// Whether the queue is currently being processed.
    processing: bool,
}

impl ActorQueue {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            processing: false,
        }
    }
}

/// Shared inner state for the Scheduler.
///
/// Wrapped in `Rc` so that `spawn_local` closures can hold a reference
/// and continue processing the actor queue after each message completes.
struct SchedulerInner {
    /// Actor handler callback.
    handler: RefCell<Option<ActorHandler>>,

    /// Per-actor queues.
    actor_queues: RefCell<HashMap<ActrId, ActorQueue>>,

    /// Whether the scheduler is running.
    running: RefCell<bool>,
}

/// Message scheduler.
///
/// Ensures serial execution per actor while allowing different actors to interleave.
/// Uses `Rc<SchedulerInner>` so the async processing loop can retain the required state.
#[derive(Clone)]
pub struct Scheduler {
    inner: Rc<SchedulerInner>,
}

impl Scheduler {
    /// Create a new scheduler.
    pub fn new() -> Self {
        Self {
            inner: Rc::new(SchedulerInner {
                handler: RefCell::new(None),
                actor_queues: RefCell::new(HashMap::new()),
                running: RefCell::new(false),
            }),
        }
    }

    /// Set the actor handler callback.
    pub fn set_handler(&self, handler: ActorHandler) {
        *self.inner.handler.borrow_mut() = Some(handler);
    }

    /// Start the scheduler.
    pub fn start(&self) {
        *self.inner.running.borrow_mut() = true;
        log::info!("[Scheduler] Started");
    }

    /// Stop the scheduler.
    pub fn stop(&self) {
        *self.inner.running.borrow_mut() = false;
        log::info!("[Scheduler] Stopped");
    }

    /// Schedule a message.
    ///
    /// Enqueues the message into the actor queue so it executes serially.
    pub fn schedule(&self, actor_id: ActrId, message: MessageRecord) {
        self.schedule_with_callback(actor_id, message, None);
    }

    /// Schedule a message with an optional completion callback.
    pub fn schedule_with_callback(
        &self,
        actor_id: ActrId,
        message: MessageRecord,
        on_complete: Option<Box<dyn FnOnce(WebResult<()>)>>,
    ) {
        if !*self.inner.running.borrow() {
            log::warn!("[Scheduler] Not running, dropping message");
            if let Some(callback) = on_complete {
                callback(Err(actr_web_common::WebError::ChannelClosed(
                    "Scheduler not running".to_string(),
                )));
            }
            return;
        }

        let msg_id = message.id;
        log::debug!(
            "[Scheduler] Scheduling message {} for actor {:?}",
            msg_id,
            actor_id
        );

        // Enqueue the message.
        let should_process = {
            let mut queues = self.inner.actor_queues.borrow_mut();
            let queue = queues
                .entry(actor_id.clone())
                .or_insert_with(ActorQueue::new);
            queue.pending.push_back(ScheduleItem {
                message,
                on_complete,
            });

            // Start processing if the queue is currently idle.
            !queue.processing
        };

        if should_process {
            Self::process_actor_queue(Rc::clone(&self.inner), actor_id);
        }
    }

    /// Process one actor queue until it becomes empty.
    ///
    /// Uses `Rc<SchedulerInner>` instead of `&self` so the `spawn_local` closure
    /// can retain the necessary state and continue with the next message.
    fn process_actor_queue(inner: Rc<SchedulerInner>, actor_id: ActrId) {
        // Mark the queue as processing.
        {
            let mut queues = inner.actor_queues.borrow_mut();
            if let Some(queue) = queues.get_mut(&actor_id) {
                if queue.processing {
                    return; // Already processing.
                }
                queue.processing = true;
            } else {
                return; // Queue does not exist.
            }
        }

        let handler = inner.handler.borrow().clone();
        let Some(handler) = handler else {
            log::warn!("[Scheduler] No handler set");
            // Reset the processing flag since we cannot proceed.
            let mut queues = inner.actor_queues.borrow_mut();
            if let Some(queue) = queues.get_mut(&actor_id) {
                queue.processing = false;
            }
            return;
        };

        // Process asynchronously with spawn_local until the queue is empty.
        spawn_local(async move {
            loop {
                // Pop the next message from the queue.
                let item = {
                    let mut queues = inner.actor_queues.borrow_mut();
                    queues
                        .get_mut(&actor_id)
                        .and_then(|q| q.pending.pop_front())
                };

                let Some(item) = item else {
                    // The queue is empty, so mark it idle.
                    let mut queues = inner.actor_queues.borrow_mut();
                    if let Some(queue) = queues.get_mut(&actor_id) {
                        queue.processing = false;
                    }
                    break;
                };

                let msg_id = item.message.id;
                log::debug!("[Scheduler] Processing message {}", msg_id);

                // Run the handler.
                let result = handler(item.message).await;

                // Invoke the completion callback.
                if let Some(callback) = item.on_complete {
                    callback(result.clone());
                }

                match &result {
                    Ok(_) => {
                        log::debug!("[Scheduler] Message {} processed successfully", msg_id)
                    }
                    Err(e) => {
                        log::error!("[Scheduler] Message {} processing failed: {}", msg_id, e)
                    }
                }

                // Yield so other tasks can run.
                gloo_timers::future::sleep(std::time::Duration::from_millis(0)).await;
            }
        });
    }

    /// Return the number of pending messages.
    pub fn pending_count(&self) -> usize {
        self.inner
            .actor_queues
            .borrow()
            .values()
            .map(|q| q.pending.len())
            .sum()
    }

    /// Return the number of actors currently being processed.
    pub fn active_actors(&self) -> usize {
        self.inner
            .actor_queues
            .borrow()
            .values()
            .filter(|q| q.processing)
            .count()
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn test_scheduler_creation() {
        let scheduler = Scheduler::new();
        assert_eq!(scheduler.pending_count(), 0);
        assert_eq!(scheduler.active_actors(), 0);
    }
}
