//! Service Worker-side error handler.
//!
//! Receives and processes error reports from the DOM side.

use crate::WirePool;
use actr_web_common::{ErrorCategory, ErrorReport, ErrorSeverity};
use parking_lot::Mutex;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

/// Error handler callback type.
pub type ErrorCallback = Arc<dyn Fn(&ErrorReport) + Send + Sync>;

/// Service Worker error handler.
pub struct SwErrorHandler {
    /// WirePool reference used to update connection state.
    wire_pool: Arc<WirePool>,

    /// Error history, capped at the most recent 100 entries.
    error_history: Arc<Mutex<VecDeque<ErrorReport>>>,

    /// User-registered error callbacks.
    error_callbacks: Arc<Mutex<Vec<ErrorCallback>>>,
}

impl SwErrorHandler {
    /// Create a new error handler.
    pub fn new(wire_pool: Arc<WirePool>) -> Self {
        log::info!("[ErrorHandler] Creating SW error handler");

        Self {
            wire_pool,
            error_history: Arc::new(Mutex::new(VecDeque::with_capacity(100))),
            error_callbacks: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Handle an error report from the DOM side.
    pub fn handle_error_report(&self, report: ErrorReport) {
        log::warn!(
            "[ErrorHandler] Received error report: category={:?}, severity={:?}, message={}",
            report.category,
            report.severity,
            report.message
        );

        // 1. Record the error in history.
        self.add_to_history(report.clone());

        // 2. Update WirePool state according to the error.
        self.update_wirepool_state(&report);

        // 3. Invoke user callbacks.
        self.invoke_callbacks(&report);

        // 4. React according to severity.
        self.handle_severity(&report);
    }

    /// Add an entry to the error history.
    fn add_to_history(&self, report: ErrorReport) {
        let mut history = self.error_history.lock();

        // Keep only the latest 100 entries.
        if history.len() >= 100 {
            history.pop_front();
        }

        history.push_back(report);
    }

    /// Update WirePool state.
    fn update_wirepool_state(&self, report: &ErrorReport) {
        // Decide whether to mark the connection as failed based on category and severity.
        let should_mark_failed = match report.severity {
            ErrorSeverity::Critical | ErrorSeverity::Fatal => true,
            ErrorSeverity::Error => {
                // For Error severity, decide based on category.
                matches!(
                    report.category,
                    ErrorCategory::WebRTC | ErrorCategory::WebSocket | ErrorCategory::Transport
                )
            }
            ErrorSeverity::Warning => false,
        };

        if should_mark_failed {
            // Read the connection type from the context.
            if let Some(ref context) = report.context {
                if let Some(conn_type) = context.conn_type {
                    log::warn!(
                        "[ErrorHandler] Marking {:?} connection as failed due to {:?} error",
                        conn_type,
                        report.severity
                    );

                    // Remove the failed connection.
                    self.wire_pool.remove_connection(conn_type);
                }
            }
        }
    }

    /// Invoke user-registered callbacks.
    fn invoke_callbacks(&self, report: &ErrorReport) {
        let callbacks = self.error_callbacks.lock();

        for callback in callbacks.iter() {
            callback(report);
        }
    }

    /// Handle the report based on severity.
    fn handle_severity(&self, report: &ErrorReport) {
        match report.severity {
            ErrorSeverity::Warning => {
                // Warning level: only log it.
                log::warn!("[ErrorHandler] Warning: {}", report.message);
            }
            ErrorSeverity::Error => {
                // Error level: log it and potentially trigger recovery.
                log::error!("[ErrorHandler] Error: {}", report.message);

                // Decide whether recovery should be triggered based on category.
                if matches!(report.category, ErrorCategory::WebRTC) {
                    log::info!("[ErrorHandler] WebRTC error detected, recovery may be needed");
                }
            }
            ErrorSeverity::Critical => {
                // Critical level: log it and trigger recovery.
                log::error!("[ErrorHandler] CRITICAL error: {}", report.message);

                // TODO: trigger automatic recovery.
            }
            ErrorSeverity::Fatal => {
                // Fatal level: log it and potentially require a full restart.
                log::error!("[ErrorHandler] FATAL error: {}", report.message);

                // TODO: trigger emergency recovery or notify the user.
            }
        }
    }

    /// Register an error callback.
    pub fn register_callback(&self, callback: ErrorCallback) {
        let mut callbacks = self.error_callbacks.lock();
        callbacks.push(callback);
        log::info!(
            "[ErrorHandler] Error callback registered (total: {})",
            callbacks.len()
        );
    }

    /// Get error history.
    pub fn get_error_history(&self, limit: usize) -> Vec<ErrorReport> {
        let history = self.error_history.lock();
        history.iter().rev().take(limit).cloned().collect()
    }

    /// Get error history filtered by category.
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

    /// Get error history filtered by severity.
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

    /// Clear the error history.
    pub fn clear_history(&self) {
        let mut history = self.error_history.lock();
        history.clear();
        log::info!("[ErrorHandler] Error history cleared");
    }

    /// Get error statistics.
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

/// Error statistics.
#[derive(Debug, Clone)]
pub struct ErrorStats {
    /// Total error count.
    pub total_errors: usize,

    /// Counts grouped by category.
    pub by_category: std::collections::HashMap<ErrorCategory, usize>,

    /// Counts grouped by severity.
    pub by_severity: std::collections::HashMap<ErrorSeverity, usize>,
}

thread_local! {
    #[doc = "Global error handler instance"]
    static GLOBAL_ERROR_HANDLER: RefCell<Option<Arc<SwErrorHandler>>> = const { RefCell::new(None) };
}

/// Initialize the global error handler.
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

/// Get the global error handler.
pub fn get_global_error_handler() -> Option<Arc<SwErrorHandler>> {
    GLOBAL_ERROR_HANDLER.with(|cell| cell.borrow().as_ref().cloned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actr_web_common::{ConnType, Dest, ErrorContext};
    use wasm_bindgen_test::*;

    wasm_bindgen_test_configure!(run_in_browser);

    // Create a test WirePool mock.
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

        // Add multiple errors.
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

        // Verify ordering with the newest error first.
        assert_eq!(history[0].message, "Error 4");
        assert_eq!(history[4].message, "Error 0");
    }

    #[wasm_bindgen_test]
    fn test_error_history_limit() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // Add more errors than the retention limit.
        for i in 0..150 {
            let report = ErrorReport::new(
                ErrorCategory::Internal,
                ErrorSeverity::Warning,
                format!("Error {}", i),
            );
            handler.handle_error_report(report);
        }

        let stats = handler.get_stats();
        // Only the newest 100 entries should remain.
        assert_eq!(stats.total_errors, 100);

        let history = handler.get_error_history(150);
        assert_eq!(history.len(), 100);

        // Verify that the oldest remaining entry is Error 50 because 0-49 were dropped.
        assert_eq!(history[99].message, "Error 50");
        assert_eq!(history[0].message, "Error 149");
    }

    #[wasm_bindgen_test]
    fn test_get_errors_by_category() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // Add errors from different categories.
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

        // Add errors with different severity levels.
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

        // Add a mix of errors.
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

        // Add errors.
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

        // Register multiple callbacks.
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

        // All callbacks should be called.
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

        // A full integration test should verify WirePool state changes.
        // This test only ensures the code path does not panic.
    }

    #[wasm_bindgen_test]
    fn test_severity_levels_trigger_different_actions() {
        let wire_pool = create_test_wire_pool();
        let handler = SwErrorHandler::new(wire_pool);

        // Warning should not trigger connection removal.
        let warning_report = ErrorReport::new(
            ErrorCategory::WebRTC,
            ErrorSeverity::Warning,
            "Warning".to_string(),
        );
        handler.handle_error_report(warning_report);

        // Critical should trigger connection removal when context exists.
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

        // Verify the statistics.
        let stats = handler.get_stats();
        assert_eq!(stats.total_errors, 2);
    }
}
