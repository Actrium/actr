//! SW 端错误处理器
//!
//! 负责接收和处理来自 DOM 的错误报告

use crate::WirePool;
use actr_web_common::{ErrorCategory, ErrorReport, ErrorSeverity};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

/// 错误处理回调类型
pub type ErrorCallback = Arc<dyn Fn(&ErrorReport) + Send + Sync>;

/// SW 错误处理器
pub struct SwErrorHandler {
    /// WirePool 引用（用于更新连接状态）
    wire_pool: Arc<WirePool>,

    /// 错误历史记录（最近 100 条）
    error_history: Arc<Mutex<VecDeque<ErrorReport>>>,

    /// 用户注册的错误回调
    error_callbacks: Arc<Mutex<Vec<ErrorCallback>>>,
}

impl SwErrorHandler {
    /// 创建新的错误处理器
    pub fn new(wire_pool: Arc<WirePool>) -> Self {
        log::info!("[ErrorHandler] Creating SW error handler");

        Self {
            wire_pool,
            error_history: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
            error_callbacks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// 处理来自 DOM 的错误报告
    pub fn handle_error_report(&self, report: ErrorReport) {
        log::warn!(
            "[ErrorHandler] Received error report: category={:?}, severity={:?}, message={}",
            report.category,
            report.severity,
            report.message
        );

        // 1. 记录错误到历史
        self.add_to_history(report.clone());

        // 2. 根据错误类型更新 WirePool 状态
        self.update_wirepool_state(&report);

        // 3. 调用用户回调
        self.invoke_callbacks(&report);

        // 4. 根据严重程度采取行动
        self.handle_severity(&report);
    }

    /// 添加到错误历史
    fn add_to_history(&self, report: ErrorReport) {
        let mut history = self.error_history.lock();

        // 保持最近 100 条
        if history.len() >= 100 {
            history.pop_front();
        }

        history.push_back(report);
    }

    /// 更新 WirePool 状态
    fn update_wirepool_state(&self, report: &ErrorReport) {
        // 根据错误类别和严重程度，决定是否标记连接为失败
        let should_mark_failed = match report.severity {
            ErrorSeverity::Critical | ErrorSeverity::Fatal => true,
            ErrorSeverity::Error => {
                // Error 级别：根据错误类别决定
                matches!(
                    report.category,
                    ErrorCategory::WebRTC | ErrorCategory::WebSocket | ErrorCategory::Transport
                )
            }
            ErrorSeverity::Warning => false,
        };

        if should_mark_failed {
            // 从上下文获取连接类型
            if let Some(ref context) = report.context {
                if let Some(conn_type) = context.conn_type {
                    log::warn!(
                        "[ErrorHandler] Marking {:?} connection as failed due to {:?} error",
                        conn_type,
                        report.severity
                    );

                    // 移除失效的连接
                    self.wire_pool.remove_connection(conn_type);
                }
            }
        }
    }

    /// 调用用户注册的错误回调
    fn invoke_callbacks(&self, report: &ErrorReport) {
        let callbacks = self.error_callbacks.lock();

        for callback in callbacks.iter() {
            callback(report);
        }
    }

    /// 根据严重程度处理
    fn handle_severity(&self, report: &ErrorReport) {
        match report.severity {
            ErrorSeverity::Warning => {
                // 警告级别：仅记录
                log::warn!("[ErrorHandler] Warning: {}", report.message);
            }
            ErrorSeverity::Error => {
                // 错误级别：记录并可能触发恢复
                log::error!("[ErrorHandler] Error: {}", report.message);

                // 根据错误类别决定是否触发恢复
                if matches!(report.category, ErrorCategory::WebRTC) {
                    log::info!("[ErrorHandler] WebRTC error detected, recovery may be needed");
                }
            }
            ErrorSeverity::Critical => {
                // 严重级别：记录并触发恢复
                log::error!("[ErrorHandler] CRITICAL error: {}", report.message);

                // TODO: 触发自动恢复机制
            }
            ErrorSeverity::Fatal => {
                // 致命级别：记录并可能需要完全重启
                log::error!("[ErrorHandler] FATAL error: {}", report.message);

                // TODO: 触发紧急恢复或通知用户
            }
        }
    }

    /// 注册错误处理回调
    pub fn register_callback(&self, callback: ErrorCallback) {
        let mut callbacks = self.error_callbacks.lock();
        callbacks.push(callback);
        log::info!(
            "[ErrorHandler] Error callback registered (total: {})",
            callbacks.len()
        );
    }

    /// 获取错误历史
    pub fn get_error_history(&self, limit: usize) -> Vec<ErrorReport> {
        let history = self.error_history.lock();
        history.iter().rev().take(limit).cloned().collect()
    }

    /// 获取特定类别的错误历史
    pub fn get_errors_by_category(
        &self,
        category: ErrorCategory,
        limit: usize,
    ) -> Vec<ErrorReport> {
        let history = self.error_history.lock();
        history
            .iter()
            .rev()
            .filter(|r| r.category == category)
            .take(limit)
            .cloned()
            .collect()
    }

    /// 获取特定严重级别的错误历史
    pub fn get_errors_by_severity(
        &self,
        severity: ErrorSeverity,
        limit: usize,
    ) -> Vec<ErrorReport> {
        let history = self.error_history.lock();
        history
            .iter()
            .rev()
            .filter(|r| r.severity == severity)
            .take(limit)
            .cloned()
            .collect()
    }

    /// 清空错误历史
    pub fn clear_history(&self) {
        let mut history = self.error_history.lock();
        history.clear();
        log::info!("[ErrorHandler] Error history cleared");
    }

    /// 获取统计信息
    pub fn get_stats(&self) -> ErrorStats {
        let history = self.error_history.lock();

        let mut by_category = std::collections::HashMap::new();
        let mut by_severity = std::collections::HashMap::new();

        for report in history.iter() {
            *by_category.entry(report.category).or_insert(0) += 1;
            *by_severity.entry(report.severity).or_insert(0) += 1;
        }

        ErrorStats {
            total_errors: history.len(),
            by_category,
            by_severity,
        }
    }
}

/// 错误统计信息
#[derive(Debug, Clone)]
pub struct ErrorStats {
    /// 总错误数
    pub total_errors: usize,

    /// 按类别统计
    pub by_category: std::collections::HashMap<ErrorCategory, usize>,

    /// 按严重级别统计
    pub by_severity: std::collections::HashMap<ErrorSeverity, usize>,
}

thread_local! {
    #[doc = "全局错误处理器实例"]
    static GLOBAL_ERROR_HANDLER: RefCell<Option<Arc<SwErrorHandler>>> = const { RefCell::new(None) };
}

/// 初始化全局错误处理器
pub fn init_global_error_handler(wire_pool: Arc<WirePool>) -> Arc<SwErrorHandler> {
    GLOBAL_ERROR_HANDLER.with(|cell| {
        if let Some(handler) = cell.borrow().as_ref() {
            return handler.clone();
        }

        let handler = Arc::new(SwErrorHandler::new(wire_pool));
        log::info!("[ErrorHandler] Global error handler initialized");
        *cell.borrow_mut() = Some(handler.clone());
        handler
    })
}

/// 获取全局错误处理器
pub fn get_global_error_handler() -> Option<Arc<SwErrorHandler>> {
    GLOBAL_ERROR_HANDLER.with(|cell| cell.borrow().as_ref().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_web_common::{ConnType, Dest, ErrorContext};
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // 创建测试用的 WirePool（模拟）
    fn create_test_wire_pool() -> Arc<WirePool> {
        Arc::new(WirePool::new())
    }

    #[wasm_bindgen_test]
    fn test_error_handler_creation() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        let stats = handler.get_stats();
        assert_eq!(stats.total_errors, 0);
        assert!(stats.by_category.is_empty());
        assert!(stats.by_severity.is_empty());
    }

    #[wasm_bindgen_test]
    fn test_handle_error_report() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        let report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Test error".to_string(),
        );

        handler.handle_error_report(report);

        let stats = handler.get_stats();
        assert_eq!(stats.total_errors, 1);
        assert_eq!(stats.by_category.get(&ErrorCategory::WebRTC), Some(&1));
        assert_eq!(stats.by_severity.get(&ErrorSeverity::Error), Some(&1));
    }

    #[wasm_bindgen_test]
    fn test_error_history() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // 添加多个错误
        for i in 0..5 {
            let report = ErrorReport::new(
                ErrorCategory::Transport,
                ErrorSeverity::Warning,
                format!("Error {}", i),
            );
            handler.handle_error_report(report);
        }

        let history = handler.get_error_history(10);
        assert_eq!(history.len(), 5);

        // 验证顺序（最新的在前）
        assert_eq!(history[0].message, "Error 4");
        assert_eq!(history[4].message, "Error 0");
    }

    #[wasm_bindgen_test]
    fn test_error_history_limit() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // 添加超过限制的错误
        for i in 0..150 {
            let report = ErrorReport::new(
                ErrorCategory::Internal,
                ErrorSeverity::Warning,
                format!("Error {}", i),
            );
            handler.handle_error_report(report);
        }

        let stats = handler.get_stats();
        // 应该只保留最近 100 条
        assert_eq!(stats.total_errors, 100);

        let history = handler.get_error_history(150);
        assert_eq!(history.len(), 100);

        // 验证最旧的是 Error 50（0-49 被丢弃）
        assert_eq!(history[99].message, "Error 50");
        assert_eq!(history[0].message, "Error 149");
    }

    #[wasm_bindgen_test]
    fn test_get_errors_by_category() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // 添加不同类别的错误
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "WebRTC error 1".to_string(),
        ));
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::WebSocket,
            ErrorSeverity::Error,
            "WebSocket error".to_string(),
        ));
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Warning,
            "WebRTC error 2".to_string(),
        ));

        let webrtc_errors = handler.get_errors_by_category(ErrorCategory::WebRTC, 10);
        assert_eq!(webrtc_errors.len(), 2);

        let websocket_errors = handler.get_errors_by_category(ErrorCategory::WebSocket, 10);
        assert_eq!(websocket_errors.len(), 1);
    }

    #[wasm_bindgen_test]
    fn test_get_errors_by_severity() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // 添加不同严重级别的错误
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Warning,
            "Warning 1".to_string(),
        ));
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Error,
            "Error 1".to_string(),
        ));
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Warning,
            "Warning 2".to_string(),
        ));
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Critical,
            "Critical 1".to_string(),
        ));

        let warnings = handler.get_errors_by_severity(ErrorSeverity::Warning, 10);
        assert_eq!(warnings.len(), 2);

        let errors = handler.get_errors_by_severity(ErrorSeverity::Error, 10);
        assert_eq!(errors.len(), 1);

        let critical = handler.get_errors_by_severity(ErrorSeverity::Critical, 10);
        assert_eq!(critical.len(), 1);
    }

    #[wasm_bindgen_test]
    fn test_error_stats() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // 添加各种错误
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Error 1".to_string(),
        ));
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Warning,
            "Warning 1".to_string(),
        ));
        handler.handle_error_report(ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Critical,
            "Critical 1".to_string(),
        ));

        let stats = handler.get_stats();
        assert_eq!(stats.total_errors, 3);
        assert_eq!(stats.by_category.get(&ErrorCategory::WebRTC), Some(&2));
        assert_eq!(stats.by_category.get(&ErrorCategory::Transport), Some(&1));
        assert_eq!(stats.by_severity.get(&ErrorSeverity::Error), Some(&1));
        assert_eq!(stats.by_severity.get(&ErrorSeverity::Warning), Some(&1));
        assert_eq!(stats.by_severity.get(&ErrorSeverity::Critical), Some(&1));
    }

    #[wasm_bindgen_test]
    fn test_clear_history() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // 添加错误
        for i in 0..10 {
            handler.handle_error_report(ErrorReport::new(
                ErrorCategory::Internal,
                ErrorSeverity::Warning,
                format!("Error {}", i),
            ));
        }

        let stats_before = handler.get_stats();
        assert_eq!(stats_before.total_errors, 10);

        handler.clear_history();

        let stats_after = handler.get_stats();
        assert_eq!(stats_after.total_errors, 0);
        assert!(stats_after.by_category.is_empty());
        assert!(stats_after.by_severity.is_empty());
    }

    #[wasm_bindgen_test]
    fn test_register_callback() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let called_clone = called.clone();

        let callback: ErrorCallback = Arc::new(move |_report| {
            called_clone.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        handler.register_callback(callback);

        let report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Test".to_string(),
        );
        handler.handle_error_report(report);

        assert!(called.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[wasm_bindgen_test]
    fn test_multiple_callbacks() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        // 注册多个回调
        for _ in 0..3 {
            let counter_clone = counter.clone();
            let callback: ErrorCallback = Arc::new(move |_report| {
                counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            });
            handler.register_callback(callback);
        }

        let report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Error,
            "Test".to_string(),
        );
        handler.handle_error_report(report);

        // 所有回调都应该被调用
        assert_eq!(counter.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[wasm_bindgen_test]
    fn test_callback_receives_correct_report() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        let received_category = Arc::new(Mutex::new(None));
        let received_category_clone = received_category.clone();

        let callback: ErrorCallback = Arc::new(move |report| {
            let mut cat = received_category_clone.lock();
            *cat = Some(report.category);
        });

        handler.register_callback(callback);

        let report = ErrorReport::new(
            ErrorCategory::Transport,
            ErrorSeverity::Critical,
            "Critical error".to_string(),
        );
        handler.handle_error_report(report);

        let cat = received_category.lock();
        assert_eq!(*cat, Some(ErrorCategory::Transport));
    }

    #[wasm_bindgen_test]
    fn test_update_wirepool_state_on_critical_error() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool.clone());

        let context = ErrorContext {
            dest: Some(Dest::Peer("peer1".to_string())),
            conn_type: Some(ConnType::WebRTC),
            debug_info: None,
        };

        let report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Critical,
            "Critical WebRTC error".to_string(),
        )
        .with_context(context);

        handler.handle_error_report(report);

        // 注意：实际测试需要验证 WirePool 状态变化
        // 这里只是确保不会 panic
    }

    #[wasm_bindgen_test]
    fn test_severity_levels_trigger_different_actions() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // Warning 不应该触发连接移除
        let warning_report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Warning,
            "Warning".to_string(),
        );
        handler.handle_error_report(warning_report);

        // Critical 应该触发连接移除（如果有 context）
        let context = ErrorContext {
            dest: Some(Dest::Peer("peer1".to_string())),
            conn_type: Some(ConnType::WebRTC),
            debug_info: None,
        };
        let critical_report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Critical,
            "Critical error".to_string(),
        )
        .with_context(context);
        handler.handle_error_report(critical_report);

        // 验证统计
        let stats = handler.get_stats();
        assert_eq!(stats.total_errors, 2);
    }
}
