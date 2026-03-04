# 错误处理机制

## 概述

actr-web 实现了完整的跨进程错误处理机制，能够将 DOM 端的错误传递到 Service Worker，并最终通知到 Actor 层。

## 架构

```
DOM Runtime
  └─> DomErrorReporter
       ├─> [优先] MessagePort (control_port)
       └─> [备用] ServiceWorker.postMessage
           ↓
Service Worker Runtime
  └─> SwLifecycleManager (接收消息)
       └─> SwErrorHandler
            ├─> 更新 WirePool 连接状态
            ├─> 记录错误历史
            └─> 调用用户注册的回调
                 ↓
           Actor 层（用户代码）
```

## 错误类型

### 错误类别 (ErrorCategory)

- `WebRTC`: WebRTC 连接错误
- `WebSocket`: WebSocket 连接错误
- `MessagePort`: MessagePort 通信错误
- `Transport`: 数据传输错误
- `Serialization`: 序列化/反序列化错误
- `Timeout`: 超时错误
- `Internal`: 内部逻辑错误

### 错误严重级别 (ErrorSeverity)

- `Warning`: 警告，非致命错误，系统可继续运行
- `Error`: 错误，影响功能，但系统整体可用
- `Critical`: 严重，关键功能失效，需要立即处理
- `Fatal`: 致命，系统无法继续运行

## 使用方法

### 1. 在 DOM 端报告错误

DOM 端的错误会被自动捕获并报告，例如：

```rust
// WebRTC 连接错误会自动报告
// 在 webrtc/coordinator.rs 中：
if let Some(reporter) = get_global_error_reporter() {
    reporter.report_webrtc_error(
        &dest,
        format!("Failed to create P2P connection: {}", e),
        ErrorSeverity::Error,
    );
}

// MessagePort 错误会自动报告
reporter.report_messageport_error(
    format!("Failed to send message: {}", e),
    ErrorSeverity::Warning,
);

// 自定义错误报告
reporter.report_error(
    ErrorCategory::Transport,
    ErrorSeverity::Critical,
    "Custom error message".to_string(),
    Some(ErrorContext {
        dest: Some(dest),
        conn_type: Some(ConnType::WebRTC),
        debug_info: Some("Additional debug info".to_string()),
    }),
)?;
```

### 2. 在 Service Worker 端处理错误

#### 初始化错误处理器

```rust
use actr_runtime_sw::{init_global_error_handler, WirePool};
use std::sync::Arc;

// 在创建 WirePool 后初始化
let wire_pool = Arc::new(WirePool::new());
let error_handler = init_global_error_handler(wire_pool.clone());
```

#### 注册错误处理回调

```rust
use actr_runtime_sw::{get_global_error_handler, ErrorCallback};
use actr_web_common::{ErrorReport, ErrorSeverity};
use std::sync::Arc;

// 注册全局错误回调
if let Some(handler) = get_global_error_handler() {
    let callback: ErrorCallback = Arc::new(move |report: &ErrorReport| {
        // 处理错误
        match report.severity {
            ErrorSeverity::Warning => {
                log::warn!("收到警告: {}", report.message);
            }
            ErrorSeverity::Error => {
                log::error!("收到错误: {}", report.message);
                // 可能触发重连等恢复操作
            }
            ErrorSeverity::Critical | ErrorSeverity::Fatal => {
                log::error!("严重错误: {}", report.message);
                // 触发紧急恢复或通知用户
            }
        }
    });

    handler.register_callback(callback);
}
```

#### 在 Actor 中处理错误

```rust
use actr_framework::{Actor, Context, Message};
use actr_runtime_sw::get_global_error_handler;
use actr_web_common::ErrorReport;
use std::sync::Arc;

struct MyActor;

impl Actor for MyActor {
    async fn started(&mut self, ctx: &mut Context<Self>) {
        // 注册错误处理回调
        if let Some(handler) = get_global_error_handler() {
            let addr = ctx.address();
            let callback = Arc::new(move |report: &ErrorReport| {
                // 向自己发送错误消息
                addr.try_send(ErrorOccurred {
                    report: report.clone(),
                });
            });

            handler.register_callback(callback);
        }
    }
}

#[derive(Message)]
struct ErrorOccurred {
    report: ErrorReport,
}

impl Handler<ErrorOccurred> for MyActor {
    async fn handle(&mut self, msg: ErrorOccurred, ctx: &mut Context<Self>) {
        // 在 Actor 的消息处理中处理错误
        log::error!("Actor 收到错误: {:?}", msg.report);

        // 根据错误类型采取行动
        match msg.report.category {
            ErrorCategory::WebRTC => {
                // 处理 WebRTC 错误，例如触发重连
            }
            ErrorCategory::WebSocket => {
                // 处理 WebSocket 错误
            }
            _ => {
                // 处理其他错误
            }
        }
    }
}
```

### 3. 查询错误历史

```rust
use actr_runtime_sw::{get_global_error_handler, ErrorCategory, ErrorSeverity};

if let Some(handler) = get_global_error_handler() {
    // 获取最近 10 条错误
    let recent_errors = handler.get_error_history(10);

    // 获取特定类别的错误
    let webrtc_errors = handler.get_errors_by_category(ErrorCategory::WebRTC, 5);

    // 获取特定严重级别的错误
    let critical_errors = handler.get_errors_by_severity(ErrorSeverity::Critical, 10);

    // 获取错误统计
    let stats = handler.get_stats();
    log::info!("总错误数: {}", stats.total_errors);
    log::info!("WebRTC 错误数: {:?}", stats.by_category.get(&ErrorCategory::WebRTC));
    log::info!("严重错误数: {:?}", stats.by_severity.get(&ErrorSeverity::Critical));
}
```

## 自动恢复机制

### WirePool 状态更新

当收到 `Error`、`Critical` 或 `Fatal` 级别的连接错误时，`SwErrorHandler` 会自动：

1. 从 WirePool 中移除失效的连接
2. 触发连接重建（如果配置了 `WebRtcRecoveryManager`）

### 示例：集成恢复管理

```rust
use actr_runtime_sw::{
    init_global_error_handler,
    WebRtcRecoveryManager,
    get_global_error_handler,
    WirePool,
};
use std::sync::Arc;

// 初始化
let wire_pool = Arc::new(WirePool::new());
let error_handler = init_global_error_handler(wire_pool.clone());
let recovery_manager = WebRtcRecoveryManager::new(wire_pool.clone());

// 注册自动恢复回调
if let Some(handler) = get_global_error_handler() {
    let recovery = recovery_manager.clone();
    let callback = Arc::new(move |report: &ErrorReport| {
        if report.severity == ErrorSeverity::Critical
            && report.category == ErrorCategory::WebRTC {
            // 触发 WebRTC 恢复
            log::warn!("WebRTC 严重错误，触发恢复流程");
            // wasm_bindgen_futures::spawn_local(async move {
            //     if let Err(e) = recovery.handle_dom_restart(session_id).await {
            //         log::error!("恢复失败: {:?}", e);
            //     }
            // });
        }
    });

    handler.register_callback(callback);
}
```

## 错误历史限制

- 错误历史保留最近 100 条记录
- 超过 100 条时，最旧的记录会被自动丢弃
- 可以调用 `clear_history()` 清空历史记录

## 注意事项

1. **初始化顺序**：必须先创建 WirePool，再初始化 ErrorHandler
2. **线程安全**：所有错误处理器都是线程安全的（使用 `Arc` + `Mutex`）
3. **性能**：错误报告是异步的，不会阻塞正常业务流程
4. **全局单例**：`DomErrorReporter` 和 `SwErrorHandler` 都使用全局单例模式
5. **回调注册时机**：建议在 Actor 的 `started()` 生命周期方法中注册回调

## 示例项目

参见 `examples/error-handling/` 目录获取完整的错误处理示例。
