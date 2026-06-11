//! Log callback for forwarding tracing events to foreign-language hosts.
//!
//! This module exposes a UniFFI `callback_interface` that allows Swift/Kotlin
//! hosts to receive all tracing events produced by the actr runtime.
//! The callback must be registered via `set_log_callback()` **before** the
//! actr node is created, because `init_observability` locks the subscriber.

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use tracing::Subscriber;
use tracing_subscriber::Layer;

/// Callback interface for forwarding tracing log events to the host.
///
/// Register via `set_log_callback()` before starting the actr node.
/// Once set, every tracing event emitted by the runtime will be
/// forwarded through this callback.
#[uniffi::export(callback_interface)]
pub trait LogCallback: Send + Sync + 'static {
    /// Called for every tracing event emitted by the actr runtime.
    ///
    /// Parameters:
    /// - `level`: tracing level (TRACE, DEBUG, INFO, WARN, ERROR).
    /// - `target`: module path of the log source.
    /// - `message`: field values formatted as `key=value` pairs.
    /// - `timestamp_ms`: wall-clock milliseconds since UNIX epoch.
    fn on_log(&self, level: String, target: String, message: String, timestamp_ms: i64);
}

// ---------------------------------------------------------------------------
// Global registry
// ---------------------------------------------------------------------------

static LOG_CALLBACK: OnceLock<Arc<dyn LogCallback>> = OnceLock::new();

/// Set or clear the global log callback.
///
/// Must be called **before** the actr node is created. The tracing subscriber
/// is locked during node initialization; calls after that point are ignored.
/// Pass `None` to disable forwarding.
#[uniffi::export]
pub fn set_log_callback(callback: Option<Box<dyn LogCallback>>) {
    if let Some(cb) = callback {
        let _ = LOG_CALLBACK.set(Arc::from(cb));
    }
}

// ---------------------------------------------------------------------------
// Layer factory — called from `logger::init_observability`
// ---------------------------------------------------------------------------

/// Shorthand for a boxed dynamic tracing layer.
pub(crate) type DynLayer = Box<dyn Layer<tracing_subscriber::Registry> + Send + Sync + 'static>;

/// Produce a [`LogCallbackLayer`] if a callback has been registered.
/// Returns `None` when no callback is set.
pub(crate) fn make_layer() -> Option<DynLayer> {
    LOG_CALLBACK.get().map(|cb| {
        let layer = LogCallbackLayer {
            callback: cb.clone(),
        };
        Box::new(layer) as DynLayer
    })
}

// ---------------------------------------------------------------------------
// Layer implementation
// ---------------------------------------------------------------------------

/// A tracing [`Layer`] that forwards every event to the global [`LogCallback`].
struct LogCallbackLayer {
    callback: Arc<dyn LogCallback>,
}

impl<S: Subscriber> Layer<S> for LogCallbackLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let metadata = event.metadata();
        let mut visitor = LogMessageVisitor::default();
        event.record(&mut visitor);

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        self.callback.on_log(
            metadata.level().to_string(),
            metadata.target().to_string(),
            visitor.message,
            timestamp_ms,
        );
    }
}

// ---------------------------------------------------------------------------
// Field visitor
// ---------------------------------------------------------------------------

/// Collects structured fields from a tracing event into a `key=value` string.
#[derive(Default)]
struct LogMessageVisitor {
    message: String,
}

impl tracing::field::Visit for LogMessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if self.message.is_empty() {
            let _ = write!(self.message, "{}={:?}", field.name(), value);
        } else {
            let _ = write!(self.message, " {}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        use std::fmt::Write;
        if self.message.is_empty() {
            let _ = write!(self.message, "{}={}", field.name(), value);
        } else {
            let _ = write!(self.message, " {}={}", field.name(), value);
        }
    }
}
