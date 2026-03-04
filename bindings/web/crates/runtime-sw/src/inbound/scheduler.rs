//! Scheduler - 消息调度器
//!
//! 负责串行调度消息到 Actor 执行
//! 对标 actr 的 Scheduler 逻辑
//!
//! # 架构位置
//!
//! ```text
//! Mailbox → MailboxProcessor → Scheduler → Actor
//! ```
//!
//! Scheduler 保证：
//! - 同一个 Actor 的消息串行执行
//! - 不同 Actor 的消息可以并发执行（在 WASM 单线程中通过 async 实现）
//! - 支持优先级调度

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use actr_mailbox_web::MessageRecord;
use actr_protocol::ActrId;
use actr_web_common::WebResult;
use wasm_bindgen_futures::spawn_local;

/// Actor 处理回调类型
pub type ActorHandler = Rc<dyn Fn(MessageRecord) -> Pin<Box<dyn Future<Output = WebResult<()>>>>>;

/// 调度项
struct ScheduleItem {
    message: MessageRecord,
    on_complete: Option<Box<dyn FnOnce(WebResult<()>)>>,
}

/// Actor 队列状态
struct ActorQueue {
    /// 待处理消息队列
    pending: VecDeque<ScheduleItem>,

    /// 是否正在处理中
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
    /// Actor 处理回调
    handler: RefCell<Option<ActorHandler>>,

    /// 每个 Actor 的队列
    actor_queues: RefCell<HashMap<ActrId, ActorQueue>>,

    /// 是否正在运行
    running: RefCell<bool>,
}

/// Scheduler - 消息调度器
///
/// 保证同一个 Actor 的消息串行执行，不同 Actor 的消息可以并发执行。
/// 使用 `Rc<SchedulerInner>` 使得异步处理循环可以持有必要的引用。
#[derive(Clone)]
pub struct Scheduler {
    inner: Rc<SchedulerInner>,
}

impl Scheduler {
    /// 创建新的调度器
    pub fn new() -> Self {
        Self {
            inner: Rc::new(SchedulerInner {
                handler: RefCell::new(None),
                actor_queues: RefCell::new(HashMap::new()),
                running: RefCell::new(false),
            }),
        }
    }

    /// 设置 Actor 处理回调
    pub fn set_handler(&self, handler: ActorHandler) {
        *self.inner.handler.borrow_mut() = Some(handler);
    }

    /// 启动调度器
    pub fn start(&self) {
        *self.inner.running.borrow_mut() = true;
        log::info!("[Scheduler] Started");
    }

    /// 停止调度器
    pub fn stop(&self) {
        *self.inner.running.borrow_mut() = false;
        log::info!("[Scheduler] Stopped");
    }

    /// 调度消息
    ///
    /// 将消息加入对应 Actor 的队列，保证串行执行
    pub fn schedule(&self, actor_id: ActrId, message: MessageRecord) {
        self.schedule_with_callback(actor_id, message, None);
    }

    /// 调度消息（带完成回调）
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

        // 加入队列
        let should_process = {
            let mut queues = self.inner.actor_queues.borrow_mut();
            let queue = queues
                .entry(actor_id.clone())
                .or_insert_with(ActorQueue::new);
            queue.pending.push_back(ScheduleItem {
                message,
                on_complete,
            });

            // 如果队列没有在处理中，则启动处理
            !queue.processing
        };

        if should_process {
            Self::process_actor_queue(Rc::clone(&self.inner), actor_id);
        }
    }

    /// 处理 Actor 队列（连续处理直到队列为空）
    ///
    /// 使用 `Rc<SchedulerInner>` 而非 `&self`，使得 `spawn_local` 闭包可以持有
    /// 必要的引用，并在处理完每条消息后继续处理下一条。
    fn process_actor_queue(inner: Rc<SchedulerInner>, actor_id: ActrId) {
        // 标记为处理中
        {
            let mut queues = inner.actor_queues.borrow_mut();
            if let Some(queue) = queues.get_mut(&actor_id) {
                if queue.processing {
                    return; // 已经在处理中
                }
                queue.processing = true;
            } else {
                return; // 队列不存在
            }
        }

        let handler = inner.handler.borrow().clone();
        let Some(handler) = handler else {
            log::warn!("[Scheduler] No handler set");
            // Reset processing flag since we can't proceed
            let mut queues = inner.actor_queues.borrow_mut();
            if let Some(queue) = queues.get_mut(&actor_id) {
                queue.processing = false;
            }
            return;
        };

        // 使用 spawn_local 异步处理
        // 连续处理队列中的所有消息，直到队列为空
        spawn_local(async move {
            loop {
                // 从队列中取出下一条消息
                let item = {
                    let mut queues = inner.actor_queues.borrow_mut();
                    queues
                        .get_mut(&actor_id)
                        .and_then(|q| q.pending.pop_front())
                };

                let Some(item) = item else {
                    // 队列为空，标记为未处理
                    let mut queues = inner.actor_queues.borrow_mut();
                    if let Some(queue) = queues.get_mut(&actor_id) {
                        queue.processing = false;
                    }
                    break;
                };

                let msg_id = item.message.id;
                log::debug!("[Scheduler] Processing message {}", msg_id);

                // 执行处理
                let result = handler(item.message).await;

                // 调用完成回调
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

                // 让出执行权，允许其他任务运行（协作式调度）
                gloo_timers::future::sleep(std::time::Duration::from_millis(0)).await;
            }
        });
    }

    /// 获取待处理消息数
    pub fn pending_count(&self) -> usize {
        self.inner
            .actor_queues
            .borrow()
            .values()
            .map(|q| q.pending.len())
            .sum()
    }

    /// 获取正在处理的 Actor 数
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
