use super::*;
use actr_config::ObservabilityConfig;

fn cfg() -> ObservabilityConfig {
    ObservabilityConfig {
        filter_level: "info".into(),
        tracing_enabled: false,
        tracing_endpoint: String::new(),
        tracing_service_name: "actr-test".into(),
    }
}

#[test]
fn init_observability_returns_guard() {
    // Global subscriber init is best-effort (try_init errors are swallowed),
    // so this returns Ok whether or not a subscriber was already installed.
    let guard = init_observability(&cfg()).unwrap();
    let _ = guard; // drop runs without panic
}

#[test]
fn init_with_default_layer() {
    let _guard =
        init_observability_with_layer(&cfg(), None::<BoxedLayer<tracing_subscriber::Registry>>)
            .unwrap();
}

#[test]
fn init_with_custom_platform_layer() {
    // A plain fmt layer stands in for a platform-specific layer (android/oslog).
    let layer = fmt::layer();
    let _guard = init_observability_with_layer(&cfg(), Some(layer)).unwrap();
}

#[test]
fn default_fmt_layer_builds_without_panic() {
    // Exercises the cfg!(unix/mobile) branch and writer configuration.
    let _layer = create_default_fmt_layer::<tracing_subscriber::Registry>();
}

#[test]
fn observability_guard_default_constructs() {
    // Without the opentelemetry feature the guard holds no provider; drop is a no-op.
    let guard = ObservabilityGuard::default();
    drop(guard);
}

#[test]
fn env_filter_falls_back_on_invalid_level() {
    // An unparseable configured level must not panic — EnvFilter falls back to "info".
    let bad = ObservabilityConfig {
        filter_level: "not-a-real-level!!!".into(),
        ..cfg()
    };
    let _guard = init_observability(&bad).unwrap();
}
