//! Monitoring and alerting
//!
//! Reserved scaffolding for future observability work. The module is
//! compiled but no runtime consumer currently invokes it; the public
//! items are crate-private and tagged `allow(dead_code)`.

#![allow(dead_code)]

use actr_protocol::{ActorResult, ActrError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Alert severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum AlertSeverity {
    /// Informational
    Info = 1,
    /// Warning
    Warning = 2,
    /// Error
    Error = 3,
    /// Critical
    Critical = 4,
}

impl AlertSeverity {
    /// Human-readable severity label
    pub fn description(&self) -> &'static str {
        match self {
            AlertSeverity::Info => "info",
            AlertSeverity::Warning => "warning",
            AlertSeverity::Error => "error",
            AlertSeverity::Critical => "critical",
        }
    }
}

/// Alert information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Alert ID
    pub id: Uuid,

    /// Alert title
    pub title: String,

    /// Alert description
    pub description: String,

    /// Severity level
    pub severity: AlertSeverity,

    /// Alert source
    pub source: String,

    /// Occurrence time
    pub timestamp: DateTime<Utc>,

    /// Whether the alert has been acknowledged
    pub acknowledged: bool,

    /// Whether the alert has been resolved
    pub resolved: bool,

    /// Labels/tags
    pub labels: HashMap<String, String>,

    /// Metric value that triggered the alert
    pub metric_value: Option<f64>,

    /// Threshold that was crossed
    pub threshold: Option<f64>,
}

impl Alert {
    /// Create a new alert
    pub fn new(
        title: String,
        description: String,
        severity: AlertSeverity,
        source: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            title,
            description,
            severity,
            source,
            timestamp: Utc::now(),
            acknowledged: false,
            resolved: false,
            labels: HashMap::new(),
            metric_value: None,
            threshold: None,
        }
    }

    /// Add a label/tag
    pub fn with_label(mut self, key: String, value: String) -> Self {
        self.labels.insert(key, value);
        self
    }

    /// Set metric value and threshold
    pub fn with_metric(mut self, value: f64, threshold: f64) -> Self {
        self.metric_value = Some(value);
        self.threshold = Some(threshold);
        self
    }

    /// Acknowledge the alert
    pub fn acknowledge(&mut self) {
        self.acknowledged = true;
    }

    /// Resolve the alert
    pub fn resolve(&mut self) {
        self.resolved = true;
    }
}

/// Alert configuration
#[derive(Debug, Clone)]
pub struct AlertConfig {
    /// Whether alerts are enabled
    pub enabled: bool,

    /// CPU usage alert thresholds
    pub cpu_warning_threshold: f64,
    pub cpu_critical_threshold: f64,

    /// Memory usage alert thresholds
    pub memory_warning_threshold: f64,
    pub memory_critical_threshold: f64,

    /// Error rate alert thresholds
    pub error_rate_warning_threshold: f64,
    pub error_rate_critical_threshold: f64,

    /// Response-time alert thresholds (milliseconds)
    pub response_time_warning_threshold_ms: f64,
    pub response_time_critical_threshold_ms: f64,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cpu_warning_threshold: 0.8,
            cpu_critical_threshold: 0.95,
            memory_warning_threshold: 0.8,
            memory_critical_threshold: 0.95,
            error_rate_warning_threshold: 0.05,
            error_rate_critical_threshold: 0.1,
            response_time_warning_threshold_ms: 1000.0,
            response_time_critical_threshold_ms: 5000.0,
        }
    }
}

/// Monitoring configuration
#[derive(Debug, Clone)]
pub struct MonitoringConfig {
    /// Whether monitoring is enabled
    pub enabled: bool,

    /// Monitoring interval (seconds)
    pub monitoring_interval_seconds: u64,

    /// Metrics retention duration (seconds)
    pub metrics_retention_seconds: u64,

    /// Alert configuration
    pub alert_config: AlertConfig,
}

impl Default for MonitoringConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            monitoring_interval_seconds: 30,
            metrics_retention_seconds: 7 * 24 * 3600, // 7 days
            alert_config: AlertConfig::default(),
        }
    }
}

/// Monitoring metric
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metric {
    /// Metric name
    pub name: String,

    /// Metric value
    pub value: f64,

    /// Timestamp
    pub timestamp: DateTime<Utc>,

    /// Labels/tags
    pub labels: HashMap<String, String>,

    /// Unit
    pub unit: Option<String>,
}

/// Monitor interface
pub trait Monitor: Send + Sync {
    /// Record a metric sample
    fn record_metric(&mut self, metric: Metric) -> ActorResult<()>;

    /// Get recent metrics
    fn get_metrics(&self, name: &str, duration_seconds: u64) -> ActorResult<Vec<Metric>>;

    /// Evaluate alert conditions and emit new alerts
    fn check_alerts(&mut self) -> ActorResult<Vec<Alert>>;

    /// Get currently active (unresolved) alerts
    fn get_active_alerts(&self) -> Vec<&Alert>;

    /// Acknowledge an alert
    fn acknowledge_alert(&mut self, alert_id: Uuid) -> ActorResult<()>;

    /// Resolve an alert
    fn resolve_alert(&mut self, alert_id: Uuid) -> ActorResult<()>;
}

/// Basic monitor implementation
pub struct BasicMonitor {
    config: MonitoringConfig,
    metrics: Vec<Metric>,
    alerts: Vec<Alert>,
}

impl BasicMonitor {
    /// Create a new monitor
    pub fn new(config: MonitoringConfig) -> Self {
        Self {
            config,
            metrics: Vec::new(),
            alerts: Vec::new(),
        }
    }

    /// Check CPU usage against warning/critical thresholds
    fn check_cpu_alerts(&mut self, cpu_usage: f64) -> ActorResult<Option<Alert>> {
        if !self.config.alert_config.enabled {
            return Ok(None);
        }

        if cpu_usage >= self.config.alert_config.cpu_critical_threshold {
            let alert = Alert::new(
                "CPU usage critical".to_string(),
                format!("CPU usage reached {:.1}%", cpu_usage * 100.0),
                AlertSeverity::Critical,
                "system".to_string(),
            )
            .with_metric(cpu_usage, self.config.alert_config.cpu_critical_threshold);

            Ok(Some(alert))
        } else if cpu_usage >= self.config.alert_config.cpu_warning_threshold {
            let alert = Alert::new(
                "CPU usage warning".to_string(),
                format!("CPU usage reached {:.1}%", cpu_usage * 100.0),
                AlertSeverity::Warning,
                "system".to_string(),
            )
            .with_metric(cpu_usage, self.config.alert_config.cpu_warning_threshold);

            Ok(Some(alert))
        } else {
            Ok(None)
        }
    }
}

impl Monitor for BasicMonitor {
    fn record_metric(&mut self, metric: Metric) -> ActorResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        self.metrics.push(metric);

        // Clean up expired metrics
        let cutoff =
            Utc::now() - chrono::Duration::seconds(self.config.metrics_retention_seconds as i64);
        self.metrics.retain(|m| m.timestamp > cutoff);

        Ok(())
    }

    fn get_metrics(&self, name: &str, duration_seconds: u64) -> ActorResult<Vec<Metric>> {
        let cutoff = Utc::now() - chrono::Duration::seconds(duration_seconds as i64);

        let metrics: Vec<Metric> = self
            .metrics
            .iter()
            .filter(|m| m.name == name && m.timestamp > cutoff)
            .cloned()
            .collect();

        Ok(metrics)
    }

    fn check_alerts(&mut self) -> ActorResult<Vec<Alert>> {
        if !self.config.alert_config.enabled {
            return Ok(Vec::new());
        }

        let mut new_alerts = Vec::new();

        // Check CPU usage
        if let Ok(cpu_metrics) = self.get_metrics("cpu_usage", 300) {
            if let Some(latest) = cpu_metrics.last() {
                if let Some(alert) = self.check_cpu_alerts(latest.value)? {
                    new_alerts.push(alert);
                }
            }
        }

        // Push newly emitted alerts onto the active list
        for alert in &new_alerts {
            self.alerts.push(alert.clone());
        }

        Ok(new_alerts)
    }

    fn get_active_alerts(&self) -> Vec<&Alert> {
        self.alerts.iter().filter(|alert| !alert.resolved).collect()
    }

    fn acknowledge_alert(&mut self, alert_id: Uuid) -> ActorResult<()> {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.id == alert_id) {
            alert.acknowledge();
            Ok(())
        } else {
            Err(ActrError::NotFound("Alert not found".to_string()))
        }
    }

    fn resolve_alert(&mut self, alert_id: Uuid) -> ActorResult<()> {
        if let Some(alert) = self.alerts.iter_mut().find(|a| a.id == alert_id) {
            alert.resolve();
            Ok(())
        } else {
            Err(ActrError::NotFound("Alert not found".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
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
        let mut cfg = MonitoringConfig::default();
        cfg.enabled = false;
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
}
