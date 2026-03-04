//! Mailbox Processor
//!
//! 负责从 Mailbox 中取出消息，处理，然后 ack
//! 对标 actr 的 mailbox processor 逻辑
//!
//! # 事件驱动
//!
//! 使用 `MailboxNotifier` 通知机制而非轮询，当新消息入队时立即唤醒处理循环。

use actr_mailbox_web::{Mailbox, MessageRecord};
use actr_web_common::WebResult;
use futures::StreamExt;
use futures::channel::mpsc;
use std::rc::Rc;
use wasm_bindgen_futures::spawn_local;

/// 消息处理回调类型
///
/// 接收消息记录并处理
/// 注意：WASM 是单线程环境，不需要 Send + Sync
pub type MailboxMessageHandler = Rc<
    dyn Fn(MessageRecord) -> std::pin::Pin<Box<dyn std::future::Future<Output = WebResult<()>>>>,
>;

/// 通知句柄，用于唤醒 MailboxProcessor
///
/// 当 `InboundPacketDispatcher` 将新消息写入 Mailbox 后，
/// 调用 `notify()` 唤醒处理循环，实现事件驱动而非轮询。
#[derive(Clone)]
pub struct MailboxNotifier {
    tx: mpsc::UnboundedSender<()>,
}

impl MailboxNotifier {
    /// 通知 MailboxProcessor 有新消息可处理
    pub fn notify(&self) {
        let _ = self.tx.unbounded_send(());
    }
}

/// Mailbox 处理器
///
/// 负责 dequeue → process → ack 循环（事件驱动）
pub struct MailboxProcessor {
    /// Mailbox 实例
    mailbox: Rc<dyn Mailbox>,

    /// 消息处理回调
    handler: Option<MailboxMessageHandler>,

    /// 每批次处理的消息数
    batch_size: usize,

    /// 是否正在运行
    running: Rc<std::cell::RefCell<bool>>,

    /// 通知接收端（事件驱动）
    notify_rx: Option<mpsc::UnboundedReceiver<()>>,
}

impl MailboxProcessor {
    /// 创建新的处理器，同时返回配套的 `MailboxNotifier`
    ///
    /// `MailboxNotifier` 应传给 `InboundPacketDispatcher`，
    /// 使其在入队新消息后能唤醒处理循环。
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

    /// 设置消息处理回调
    pub fn set_handler(&mut self, handler: MailboxMessageHandler) {
        self.handler = Some(handler);
    }

    /// 启动处理循环
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

    /// 停止处理循环
    pub fn stop(&self) {
        *self.running.borrow_mut() = false;
        log::info!("[MailboxProcessor] Stopped");
    }

    /// 事件驱动处理循环
    ///
    /// 当 Mailbox 为空时，等待 `notify_rx` 通知而非轮询，
    /// 实现零轮询的事件驱动模式。
    async fn processing_loop(
        mailbox: Rc<dyn Mailbox>,
        handler: Option<MailboxMessageHandler>,
        batch_size: usize,
        running: Rc<std::cell::RefCell<bool>>,
        mut notify_rx: mpsc::UnboundedReceiver<()>,
    ) {
        loop {
            // 检查是否应该继续运行
            if !*running.borrow() {
                break;
            }

            // 1. Dequeue messages
            match mailbox.dequeue(batch_size).await {
                Ok(messages) => {
                    if messages.is_empty() {
                        // 事件驱动：等待通知而非轮询
                        if notify_rx.next().await.is_none() {
                            log::info!("[MailboxProcessor] Notify channel closed, stopping");
                            break;
                        }
                        continue;
                    }

                    log::debug!("[MailboxProcessor] Dequeued {} messages", messages.len());

                    // 2. Process each message
                    for msg in messages {
                        let msg_id = msg.id;

                        // 调用处理回调
                        if let Some(ref h) = handler {
                            match h(msg).await {
                                Ok(_) => {
                                    // 3. Ack successful processing
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
                                    // 处理失败时，消息不会被 ack，会保留在 Mailbox 中
                                }
                            }
                        } else {
                            log::warn!(
                                "[MailboxProcessor] No handler set, skipping message {}",
                                msg_id
                            );
                        }
                    }
                    // 处理完一批后立即尝试取下一批（无等待）
                }
                Err(e) => {
                    log::error!("[MailboxProcessor] Failed to dequeue: {}", e);
                    // 数据库错误，等待更长时间
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

        // 清空 mailbox
        mailbox.clear().await.expect("Failed to clear mailbox");

        // 入队一条消息
        let msg_id = mailbox
            .enqueue(
                b"test-sender".to_vec(),
                b"test-payload".to_vec(),
                MessagePriority::Normal,
            )
            .await
            .expect("Failed to enqueue");

        log::info!("Enqueued message: {}", msg_id);

        // 创建处理器
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox.clone(), 10);

        // 设置处理回调
        processor.set_handler(Rc::new(|msg| {
            Box::pin(async move {
                log::info!("Processing message: {}", msg.id);
                Ok(())
            })
        }));

        // 启动处理器
        processor.start();

        // 等待处理完成
        gloo_timers::future::sleep(std::time::Duration::from_millis(500)).await;

        // 停止处理器
        processor.stop();

        // 验证消息已被 ack（应该从 mailbox 中删除）
        let stats = mailbox.stats().await.expect("Failed to get stats");
        assert_eq!(stats.pending_messages, 0);
    }

    // 以下是不依赖浏览器环境的标准单元测试

    // Mock Mailbox 实现用于测试
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

        // 初始状态应该是未运行
        assert!(!*processor.running.borrow());

        // 初始没有 handler
        assert!(processor.handler.is_none());
    }

    #[test]
    fn test_set_handler() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // 设置 handler
        let handler: MailboxMessageHandler = Rc::new(|_msg| Box::pin(async move { Ok(()) }));

        processor.set_handler(handler);
        assert!(processor.handler.is_some());
    }

    #[test]
    fn test_stop_sets_running_to_false() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // 手动设置 running 为 true
        *processor.running.borrow_mut() = true;

        // 调用 stop
        processor.stop();

        // 验证 running 被设置为 false
        assert!(!*processor.running.borrow());
    }

    #[test]
    fn test_mailbox_processor_with_different_batch_sizes() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;

        // 测试不同的 batch_size 值
        for batch_size in vec![1, 5, 10, 20, 50, 100] {
            let (processor, _notifier) = MailboxProcessor::new(mailbox.clone(), batch_size);
            assert_eq!(processor.batch_size, batch_size);
        }
    }

    #[test]
    fn test_mailbox_processor_handler_can_be_reset() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // 第一次设置
        let handler1: MailboxMessageHandler = Rc::new(|_msg| Box::pin(async move { Ok(()) }));
        processor.set_handler(handler1);
        assert!(processor.handler.is_some());

        // 第二次设置（覆盖）
        let handler2: MailboxMessageHandler = Rc::new(|_msg| Box::pin(async move { Ok(()) }));
        processor.set_handler(handler2);
        assert!(processor.handler.is_some());
    }

    #[test]
    fn test_running_state_rc() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // 获取 running Rc 的克隆
        let running_clone = processor.running.clone();

        // 通过 processor 修改状态
        *processor.running.borrow_mut() = true;

        // 通过克隆的引用应该能看到相同的状态
        assert!(*running_clone.borrow());

        // 调用 stop
        processor.stop();

        // 两个引用都应该看到 false
        assert!(!*processor.running.borrow());
        assert!(!*running_clone.borrow());
    }

    #[test]
    fn test_mailbox_processor_with_zero_batch_size() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;

        // 即使是 0 也应该能创建（虽然实际使用中不推荐）
        let (processor, _notifier) = MailboxProcessor::new(mailbox, 0);
        assert_eq!(processor.batch_size, 0);
    }

    #[test]
    fn test_mailbox_processor_rc_sharing() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;

        let (processor1, _notifier1) = MailboxProcessor::new(mailbox.clone(), 10);
        let (processor2, _notifier2) = MailboxProcessor::new(mailbox.clone(), 20);

        // 两个处理器应该共享相同的 mailbox Rc
        assert!(Rc::ptr_eq(&processor1.mailbox, &processor2.mailbox));
    }

    #[test]
    fn test_multiple_start_calls_warning() {
        let mailbox = Rc::new(MockMailbox) as Rc<dyn Mailbox>;
        let (mut processor, _notifier) = MailboxProcessor::new(mailbox, 10);

        // 手动设置为 running
        *processor.running.borrow_mut() = true;

        // 第二次调用 start 应该会提前返回（记录警告）
        processor.start();

        // 状态应该仍然是 running
        assert!(*processor.running.borrow());
    }
}
