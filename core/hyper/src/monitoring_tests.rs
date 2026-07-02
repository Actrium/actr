use super::*;

#[test]
fn alert_severity_descriptions_and_order() {
    assert_eq!(AlertSeverity::Info.description(), "info");
    assert_eq!(AlertSeverity::Warning.description(), "warning");
    assert_eq!(AlertSeverity::Error.description(), "error");
    assert_eq!(AlertSeverity::Critical.description(), "critical");

    // Discriminants are ordered so severity comparison works.
    assert!(AlertSeverity::Critical > AlertSeverity::Error);
    assert!(AlertSeverity::Error > AlertSeverity::Warning);
    assert!(AlertSeverity::Warning > AlertSeverity::Info);
}

#[test]
fn alert_new_sets_defaults() {
    let a = Alert::new(
        "title".into(),
        "desc".into(),
        AlertSeverity::Warning,
        "src".into(),
    );
    assert_eq!(a.title, "title");
    assert_eq!(a.description, "desc");
    assert_eq!(a.severity, AlertSeverity::Warning);
    assert_eq!(a.source, "src");
    assert!(!a.acknowledged);
    assert!(!a.resolved);
    assert!(a.labels.is_empty());
    assert!(a.metric_value.is_none());
    assert!(a.threshold.is_none());
}

#[test]
fn alert_builders_and_state_transitions() {
    let a = Alert::new("t".into(), "d".into(), AlertSeverity::Critical, "s".into())
        .with_label("env".into(), "prod".into())
        .with_metric(0.97, 0.95);
    assert_eq!(a.labels.get("env").unwrap(), "prod");
    assert_eq!(a.metric_value, Some(0.97));
    assert_eq!(a.threshold, Some(0.95));

    let mut a = a;
    a.acknowledge();
    assert!(a.acknowledged);
    assert!(!a.resolved);
    a.resolve();
    assert!(a.resolved);
}

#[test]
fn alert_config_default_thresholds() {
    let c = AlertConfig::default();
    assert!(c.enabled);
    assert_eq!(c.cpu_warning_threshold, 0.8);
    assert_eq!(c.cpu_critical_threshold, 0.95);
    assert_eq!(c.error_rate_warning_threshold, 0.05);
    assert_eq!(c.response_time_critical_threshold_ms, 5000.0);
}

#[test]
fn monitoring_config_default() {
    let c = MonitoringConfig::default();
    assert!(c.enabled);
    assert_eq!(c.monitoring_interval_seconds, 30);
    assert_eq!(c.metrics_retention_seconds, 7 * 24 * 3600);
    assert!(c.alert_config.enabled);
}

fn cpu_metric(value: f64) -> Metric {
    Metric {
        name: "cpu_usage".into(),
        value,
        timestamp: Utc::now(),
        labels: HashMap::new(),
        unit: None,
    }
}

#[test]
fn record_metric_disabled_is_noop() {
    let cfg = MonitoringConfig {
        enabled: false,
        ..MonitoringConfig::default()
    };
    let mut m = BasicMonitor::new(cfg);
    m.record_metric(cpu_metric(0.5)).unwrap();
    assert!(m.get_metrics("cpu_usage", 60).unwrap().is_empty());
}

#[test]
fn record_and_retrieve_metric() {
    let mut m = BasicMonitor::new(MonitoringConfig::default());
    m.record_metric(cpu_metric(0.5)).unwrap();
    let got = m.get_metrics("cpu_usage", 60).unwrap();
    assert_eq!(got.len(), 1);
    // Unknown name returns nothing.
    assert!(m.get_metrics("memory", 60).unwrap().is_empty());
}

#[test]
fn get_metrics_filters_old_samples() {
    let mut m = BasicMonitor::new(MonitoringConfig::default());
    m.record_metric(cpu_metric(0.5)).unwrap();
    // Window of 0 seconds ago excludes the just-recorded sample (uses strict >).
    let got = m.get_metrics("cpu_usage", 0).unwrap();
    assert!(got.is_empty());
}

#[test]
fn check_alerts_emits_warning_then_critical_and_stores() {
    let mut m = BasicMonitor::new(MonitoringConfig::default());

    // Below warning threshold → no alert.
    m.record_metric(cpu_metric(0.5)).unwrap();
    assert!(m.check_alerts().unwrap().is_empty());

    // Warning band.
    m.record_metric(cpu_metric(0.85)).unwrap();
    let emitted = m.check_alerts().unwrap();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].severity, AlertSeverity::Warning);
    assert_eq!(m.get_active_alerts().len(), 1);

    // Critical band.
    m.record_metric(cpu_metric(0.99)).unwrap();
    let emitted = m.check_alerts().unwrap();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].severity, AlertSeverity::Critical);
    assert_eq!(m.get_active_alerts().len(), 2);
}

#[test]
fn check_alerts_disabled_returns_empty() {
    let mut cfg = MonitoringConfig::default();
    cfg.alert_config.enabled = false;
    let mut m = BasicMonitor::new(cfg);
    m.record_metric(cpu_metric(0.99)).unwrap();
    assert!(m.check_alerts().unwrap().is_empty());
}

#[test]
fn active_alerts_excludes_resolved() {
    let mut m = BasicMonitor::new(MonitoringConfig::default());
    m.record_metric(cpu_metric(0.99)).unwrap();
    let emitted = m.check_alerts().unwrap();
    let id = emitted[0].id;
    assert_eq!(m.get_active_alerts().len(), 1);

    m.resolve_alert(id).unwrap();
    assert!(m.get_active_alerts().is_empty());
}

#[test]
fn acknowledge_and_resolve_unknown_alert_errors() {
    let mut m = BasicMonitor::new(MonitoringConfig::default());
    let bogus = Uuid::new_v4();
    assert!(matches!(
        m.acknowledge_alert(bogus),
        Err(ActrError::NotFound(_))
    ));
    assert!(matches!(
        m.resolve_alert(bogus),
        Err(ActrError::NotFound(_))
    ));
}

#[test]
fn acknowledge_marks_alert() {
    let mut m = BasicMonitor::new(MonitoringConfig::default());
    m.record_metric(cpu_metric(0.99)).unwrap();
    let id = m.check_alerts().unwrap()[0].id;
    m.acknowledge_alert(id).unwrap();
    let active = m.get_active_alerts();
    assert!(active.iter().any(|a| a.id == id && a.acknowledged));
}
