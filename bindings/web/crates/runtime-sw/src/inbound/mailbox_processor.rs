//! Mailbox Processor
//!
//! Dequeues messages from the Mailbox, processes them, and acknowledges them.
//! Mirrors actr's mailbox processor logic.
//!
//! # Event-driven behavior
//!
//! Uses `MailboxNotifier` instead of polling so the processing loop wakes as soon as
//! a new message is enqueued.

use actr_mailbox_web::{Mailbox, MessageRecord};
use actr_web_common::WebResult;
use futures::StreamExt;
use futures::channel::mpsc;
use std::rc::Rc;
use wasm_bindgen_futures::spawn_local;

/// Message processing callback type.
///
/// Receives a message record and processes it.
/// Note: WASM is single-threaded here, so `Send + Sync` is unnecessary.
pub type MailboxMessageHandler = Rc<
    dyn Fn(MessageRecord) -> std::pin::Pin<Box<dyn std::future::Future<Output = WebResult<()>>>>,
>;

/// Notification handle used to wake MailboxProcessor.
///
/// After `InboundPacketDispatcher` writes a new message into the Mailbox,
/// it calls `notify()` to wake the processing loop without polling.
#[derive(Clone)]
pub struct MailboxNotifier {
    tx: mpsc::UnboundedSender<()>,
}

impl MailboxNotifier {
    /// Notify MailboxProcessor that new work is available.
    pub fn notify(&self) {
        let _ = self.tx.unbounded_send(());
    }
}

/// Mailbox processor.
///
/// Runs the dequeue -> process -> ack loop in an event-driven way.
pub struct MailboxProcessor {
    /// Mailbox instance.
    mailbox: Rc<dyn Mailbox>,

    /// Message processing callback.
    handler: Option<MailboxMessageHandler>,

    /// Number of messages processed per batch.
    batch_size: usize,

    /// Whether the processor is currently running.
    running: Rc<std::cell::RefCell<bool>>,

    /// Notification receiver for event-driven wakeups.
    notify_rx: Option<mpsc::UnboundedReceiver<()>>,
}

impl MailboxProcessor {
    /// Create a new processor and the corresponding `MailboxNotifier`.
    ///
    /// `MailboxNotifier` should be passed to `InboundPacketDispatcher` so it can
    /// wake the processing loop after enqueueing new messages.
    pub fn new(mailbox: Rc<dyn Mailbox>, batch_size: usize) -> (Self, MailboxNotifier) {
        let (tx, rx) = mpsc::unbounded();
        (
            Self {
                mailbox,
                handler: None,
                batch_size,
                running: Rc::new(std::cell::RefCell::new(false)),
                notify_rx: Some(rx),
            },
            MailboxNotifier { tx },
        )
    }

    /// Set the message processing callback.
    pub fn set_handler(&mut self, handler: MailboxMessageHandler) {
        self.handler = Some(handler);
    }

    /// Start the processing loop.
    pub fn start(&mut self) {
        {
            let mut running = self.running.borrow_mut();
            if *running {
                log::warn!("[MailboxProcessor] Already running");
                return;
            }
            *running = true;
        }

        let Some(notify_rx) = self.notify_rx.take() else {
            log::warn!("[MailboxProcessor] Cannot start: notify channel already consumed");
            *self.running.borrow_mut() = false;
            return;
        };

        log::info!("[MailboxProcessor] Starting event-driven processing loop");

        let mailbox = self.mailbox.clone();
        let handler = self.handler.clone();
        let batch_size = self.batch_size;
        let running = self.running.clone();

        spawn_local(async move {
            Self::processing_loop(mailbox, handler, batch_size, running, notify_rx).await;
        });
    }

    /// Stop the processing loop.
    pub fn stop(&self) {
        *self.running.borrow_mut() = false;
        log::info!("[MailboxProcessor] Stopped");
    }

    /// Event-driven processing loop.
    ///
    /// When the Mailbox is empty, wait for `notify_rx` instead of polling.
    async fn processing_loop(
        mailbox: Rc<dyn Mailbox>,
        handler: Option<MailboxMessageHandler>,
        batch_size: usize,
        running: Rc<std::cell::RefCell<bool>>,
        mut notify_rx: mpsc::UnboundedReceiver<()>,
    ) {
        loop {
            // Check whether processing should continue.
            if !*running.borrow() {
                break;
            }

            // 1. Dequeue messages.
            match mailbox.dequeue(batch_size).await {
                Ok(messages) => {
                    if messages.is_empty() {
                        // Event-driven wait instead of polling.
                        if notify_rx.next().await.is_none() {
                            log::info!("[MailboxProcessor] Notify channel closed, stopping");
                            break;
                        }
                        continue;
                    }

                    log::debug!("[MailboxProcessor] Dequeued {} messages", messages.len());

                    // 2. Process each message.
                    for msg in messages {
                        let msg_id = msg.id;

                        // Invoke the handler callback.
                        if let Some(ref h) = handler {
                            match h(msg).await {
                                Ok(_) => {
                                    // 3. Acknowledge successful processing.
                                    if let Err(e) = mailbox.ack(msg_id).await {
                                        log::error!(
                                            "[MailboxProcessor] Failed to ack message {}: {}",
                                            msg_id,
                                            e
                                        );
                                    } else {
                                        log::debug!("[MailboxProcessor] Message {} acked", msg_id);
                                    }
                                }
                                Err(e) => {
                                    log::error!(
                                        "[MailboxProcessor] Failed to process message {}: {}",
                                        msg_id,
                                        e
                                    );
                                    // On failure, the message is left unacknowledged in the Mailbox.
                                }
                            }
                        } else {
                            log::warn!(
                                "[MailboxProcessor] No handler set, skipping message {}",
                                msg_id
                            );
                        }
                    }
                    // After one batch finishes, immediately try the next batch.
                }
                Err(e) => {
                    log::error!("[MailboxProcessor] Failed to dequeue: {}", e);
                    // On database errors, back off for longer.
                    gloo_timers::future::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }

        log::info!("[MailboxProcessor] Processing loop terminated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_mailbox_web::{IndexedDbMailbox, MessagePriority};
    use std::sync::Arc;
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    async fn test_mailbox_processor_creation() {
        let mailbox = Rc::new(
            IndexedDbMailbox::new()
                .await
                .expect("Failed to create mailbox"),
        );
        let (_processor, _notifier) = MailboxProcessor::new(mailbox, 10);
    }

    #[wasm_bindgen_test]
    async fn test_mailbox_processor_dequeue_ack() {
        let mailbox = Rc::new(
            IndexedDbMailbox::new()
                .await
                .expect("Failed to create mailbox"),
        );

        // Clear the mailbox.
        mailbox.clear().await.expect("Failed to clear mailbox");

        // Enqueue one message.
        let msg_id = mailbox
            .enqueue(
                b"test-sender".to_vec(),
                b"test-payload".to_vec(),
                MessagePriority::Normal,
            )
            .await
            .expect("Failed to enqueue");

        log::info!("Enqueued message: {}", msg_id);

        // Create the processor.
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox.clone(), 10);

        // Set the handler callback.
        processor.set_handler(Rc::new(|msg| {
            Box::pin(async move {
                log::info!("Processing message: {}", msg.id);
                Ok(())
            })
        }));

        // Start the processor.
        processor.start();

        // Wait for processing to complete.
        gloo_timers::future::sleep(std::time::Duration::from_millis(500)).await;

        // Stop the processor.
        processor.stop();

        // Verify that the message was acked and removed from the mailbox.
        let stats = mailbox.stats().await.expect("Failed to get stats");
        assert_eq!(stats.pending_messages, 0);
    }

    // The tests below are standard unit tests that do not depend on a browser.

    // Mock Mailbox implementation used for testing.
    struct MockMailbox;

    use actr_mailbox_web::MailboxError;
    use uuid::Uuid;

    #[async_trait::async_trait(?Send)]
    impl Mailbox for MockMailbox {
        async fn enqueue(
            &self,
            _from: Vec<u8>,
            _payload: Vec<u8>,
            _priority: MessagePriority,
        ) -> Result<Uuid, MailboxError> {
            Ok(Uuid::new_v4())
        }

        async fn dequeue(&self, _limit: usize) -> Result<Vec<MessageRecord>, MailboxError> {
            Ok(vec![])
        }

        async fn ack(&self, _id: Uuid) -> Result<(), MailboxError> {
            Ok(())
        }

        async fn clear(&self) -> Result<(), MailboxError> {
            Ok(())
        }

        async fn stats(&self) -> Result<actr_mailbox_web::MailboxStats, MailboxError> {
            Ok(actr_mailbox_web::MailboxStats {
                pending_messages: 0,
                total_messages: 0,
                processing_messages: 0,
                high_priority_count: 0,
                normal_priority_count: 0,
                low_priority_count: 0,
            })
        }
    }

    #[test]
    fn test_mailbox_processor_batch_size() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;

        let (processor1, _notifier1) = MailboxProcessor::new(mailbox.clone(), 10);
        assert_eq!(processor1.batch_size, 10);

        let (processor2, _notifier2) = MailboxProcessor::new(mailbox.clone(), 50);
        assert_eq!(processor2.batch_size, 50);

        let (processor3, _notifier3) = MailboxProcessor::new(mailbox.clone(), 1);
        assert_eq!(processor3.batch_size, 1);
    }

    #[test]
    fn test_mailbox_processor_initial_state() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // The initial state should be stopped.
        assert!(!*processor.running.borrow());

        // No handler should be set initially.
        assert!(processor.handler.is_none());
    }

    #[test]
    fn test_set_handler() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // Install the handler.
        let handler: MailboxMessageHandler = Rc::new(|_msg| Box::pin(async move { Ok(()) }));

        processor.set_handler(handler);
        assert!(processor.handler.is_some());
    }

    #[test]
    fn test_stop_sets_running_to_false() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // Manually set `running` to true.
        *processor.running.borrow_mut() = true;

        // Call `stop`.
        processor.stop();

        // Verify that `running` becomes false.
        assert!(!*processor.running.borrow());
    }

    #[test]
    fn test_mailbox_processor_with_different_batch_sizes() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;

        // Test different `batch_size` values.
        for batch_size in vec![1, 5, 10, 20, 50, 100] {
            let (processor, _notifier) = MailboxProcessor::new(mailbox.clone(), batch_size);
            assert_eq!(processor.batch_size, batch_size);
        }
    }

    #[test]
    fn test_mailbox_processor_handler_can_be_reset() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // First assignment.
        let handler1: MailboxMessageHandler = Rc::new(|_msg| Box::pin(async move { Ok(()) }));
        processor.set_handler(handler1);
        assert!(processor.handler.is_some());

        // Second assignment overrides the first one.
        let handler2: MailboxMessageHandler = Rc::new(|_msg| Box::pin(async move { Ok(()) }));
        processor.set_handler(handler2);
        assert!(processor.handler.is_some());
    }

    #[test]
    fn test_running_state_rc() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // Clone the shared `running` handle.
        let running_clone = processor.running.clone();

        // Change the state through the processor.
        *processor.running.borrow_mut() = true;

        // The cloned reference should observe the same state.
        assert!(*running_clone.borrow());

        // Call `stop`.
        processor.stop();

        // Both references should now observe `false`.
        assert!(!*processor.running.borrow());
        assert!(!*running_clone.borrow());
    }

    #[test]
    fn test_mailbox_processor_with_zero_batch_size() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;

        // Construction should still succeed with `0`, even if that is not recommended.
        let (processor, _notifier) = MailboxProcessor::new(mailbox, 0);
        assert_eq!(processor.batch_size, 0);
    }

    #[test]
    fn test_mailbox_processor_rc_sharing() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;

        let (processor1, _notifier1) = MailboxProcessor::new(mailbox.clone(), 10);
        let (processor2, _notifier2) = MailboxProcessor::new(mailbox.clone(), 20);

        // Both processors should share the same mailbox `Rc`.
        assert!(Rc::ptr_eq(&processor1.mailbox, &processor2.mailbox));
    }

    #[test]
    fn test_multiple_start_calls_warning() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // Manually set the processor to running.
        *processor.running.borrow_mut() = true;

        // A second `start` call should return early and log a warning.
        processor.start();

        // The state should still be running.
        assert!(*processor.running.borrow());
    }
}
