//! Platform-specific observability initialization for libactr.
//!
//! This module adapts the core observability features from `actr-runtime`
//! to mobile platforms (Android Logcat, iOS os_log).
//!
//! When a [`LogCallback`](crate::log_callback::LogCallback) has been
//! registered via `set_log_callback()` before initialization, a forwarding
//! layer is composed alongside the platform layer so that the host can
//! receive tracing events.

use actr_config::ObservabilityConfig;
use actr_hyper::observability::{ObservabilityGuard, init_observability_with_layer};
use std::sync::OnceLock;

static GUARD: OnceLock<ObservabilityGuard> = OnceLock::new();

pub(crate) fn init_observability(config: ObservabilityConfig) {
    if GUARD.get().is_some() {
        return;
    }

    let guard: Option<ObservabilityGuard> = {
        #[cfg(target_os = "android")]
        {
            init_android(&config)
        }

        #[cfg(target_os = "macos")]
        {
            init_macos(&config)
        }

        #[cfg(not(any(target_os = "android", target_os = "macos")))]
        {
            init_default(&config)
        }
    };

    if let Some(g) = guard {
        let _ = GUARD.set(g);
    }
}

#[cfg(target_os = "android")]
fn init_android(config: &ObservabilityConfig) -> Option<ObservabilityGuard> {
    let callback_layer = crate::log_callback::make_layer();

    match tracing_android::layer("actr") {
        Ok(platform_layer) => {
            compose_and_init(config, Some(Box::new(platform_layer)), callback_layer)
        }
        Err(_) => compose_and_init(config, None, callback_layer),
    }
}

#[cfg(target_os = "macos")]
fn init_macos(config: &ObservabilityConfig) -> Option<ObservabilityGuard> {
    let callback_layer = crate::log_callback::make_layer();

    #[cfg(feature = "macos-oslog")]
    let platform_layer: Option<crate::log_callback::DynLayer> = {
        let layer = tracing_oslog::OsLogger::new("io.actrium.actr", "core");
        Some(Box::new(layer))
    };
    #[cfg(not(feature = "macos-oslog"))]
    let platform_layer: Option<crate::log_callback::DynLayer> = None;

    compose_and_init(config, platform_layer, callback_layer)
}

#[cfg(not(any(target_os = "android", target_os = "macos")))]
fn init_default(config: &ObservabilityConfig) -> Option<ObservabilityGuard> {
    let callback_layer = crate::log_callback::make_layer();
    compose_and_init(config, None, callback_layer)
}

fn compose_and_init(
    config: &ObservabilityConfig,
    platform: Option<crate::log_callback::DynLayer>,
    callback: Option<crate::log_callback::DynLayer>,
) -> Option<ObservabilityGuard> {
    let composed: Option<crate::log_callback::DynLayer> = match (platform, callback) {
        (Some(p), Some(c)) => Some(Box::new(CompositeLayer(p, c))),
        (Some(p), None) => Some(p),
        (None, Some(c)) => Some(c),
        (None, None) => None,
    };

    init_observability_with_layer(config, composed).ok()
}

struct CompositeLayer(crate::log_callback::DynLayer, crate::log_callback::DynLayer);

impl tracing_subscriber::Layer<tracing_subscriber::Registry> for CompositeLayer {
    fn on_layer(&mut self, subscriber: &mut tracing_subscriber::Registry) {
        self.0.on_layer(subscriber);
        self.1.on_layer(subscriber);
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        ctx: tracing_subscriber::layer::Context<'_, tracing_subscriber::Registry>,
    ) {
        self.0.on_event(event, ctx.clone());
        self.1.on_event(event, ctx);
    }

    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        id: &tracing::span::Id,
        ctx: tracing_subscriber::layer::Context<'_, tracing_subscriber::Registry>,
    ) {
        self.0.on_new_span(attrs, id, ctx.clone());
        self.1.on_new_span(attrs, id, ctx);
    }
}
