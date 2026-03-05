//! Unified Error Handling
//!
//! Defines unified error types and handling strategies for the CLI tool

use thiserror::Error;

/// CLI Unified Error Type
#[derive(Debug, Error)]
pub enum ActrCliError {
    #[error("Config error: {message}")]
    Config { message: String },

    #[error("Invalid project: {message}")]
    InvalidProject { message: String },

    #[error("Invalid argument: {message}")]
    InvalidArgument { message: String },

    #[error("Network error: {message}")]
    Network { message: String },

    #[error("Dependency error: {message}")]
    Dependency { message: String },

    #[error("Dependency conflict: {message}")]
    DependencyConflict { message: String },

    #[error("Service not found: {name}")]
    ServiceNotFound { name: String },

    #[error("Service discovery error: {message}")]
    ServiceDiscovery { message: String },

    #[error("Fingerprint validation error: {message}")]
    FingerprintValidation { message: String },

    #[error("Fingerprint mismatch: expected {expected}, got {actual}")]
    FingerprintMismatch { expected: String, actual: String },

    #[error("Compatibility conflict: {message}")]
    CompatibilityConflict { message: String },

    #[error("Code generation error: {message}")]
    CodeGeneration { message: String },

    #[error("Cache error: {message}")]
    Cache { message: String },

    #[error("User interface error: {message}")]
    UserInterface { message: String },

    #[error("Command execution error: {message}")]
    Command { message: String },

    #[error("Validation failed: {details}")]
    ValidationFailed { details: String },

    #[error("Install failed: {reason}")]
    InstallFailed { reason: String },

    #[error("Component not registered: {component}")]
    ComponentNotRegistered { component: String },

    #[error("Operation cancelled")]
    OperationCancelled,

    #[error("IO error")]
    Io(#[from] std::io::Error),

    #[error("Serialization error")]
    Serialization(#[from] toml::de::Error),

    #[error("HTTP error")]
    Http(#[from] reqwest::Error),

    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Install Error
#[derive(Debug, Error)]
pub enum InstallError {
    #[error("Dependency resolution failed: {dependency}")]
    DependencyResolutionFailed { dependency: String },

    #[error("Service unavailable: {service}")]
    ServiceUnavailable { service: String },

    #[error("Network connection failed")]
    NetworkConnectionFailed,

    #[error("Fingerprint mismatch: {service} - expected: {expected}, actual: {actual}")]
    FingerprintMismatch {
        service: String,
        expected: String,
        actual: String,
    },

    #[error("Version conflict: {details}")]
    VersionConflict { details: String },

    #[error("Cache operation failed: {operation}")]
    CacheOperationFailed { operation: String },

    #[error("Config update failed: {reason}")]
    ConfigUpdateFailed { reason: String },

    #[error("Pre-check failed: {failures:?}")]
    PreCheckFailed { failures: Vec<String> },
}

/// Validation Error
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Config file syntax error: {file}")]
    ConfigSyntaxError { file: String },

    #[error("Dependency not found: {dependency}")]
    DependencyNotFound { dependency: String },

    #[error("Network unreachable")]
    NetworkUnreachable,

    #[error("Fingerprint mismatch: {service}")]
    FingerprintMismatch { service: String },

    #[error("Circular dependency: {cycle}")]
    CircularDependency { cycle: String },

    #[error("Insufficient permissions: {resource}")]
    InsufficientPermissions { resource: String },
}

/// User-friendly Error Display
impl ActrCliError {
    /// Get user-friendly error message
    pub fn user_message(&self) -> String {
        match self {
            ActrCliError::Config { message } => {
                format!(
                    "⚠️  Config file error: {message}\n💡 Hint: Check Actr.toml syntax and content"
                )
            }
            ActrCliError::Network { message } => {
                format!(
                    "🌐 Network connection error: {message}\n💡 Hint: Check network connection and service address"
                )
            }
            ActrCliError::Dependency { message } => {
                format!(
                    "📦 Dependency error: {message}\n💡 Hint: Run 'actr check' to check dependencies"
                )
            }
            ActrCliError::ValidationFailed { details } => {
                format!(
                    "❌ Validation failed: {details}\n💡 Hint: Fix the issues above and try again"
                )
            }
            ActrCliError::InstallFailed { reason } => {
                format!(
                    "📥 Install failed: {reason}\n💡 Hint: Run 'actr check' to check environment"
                )
            }
            _ => self.to_string(),
        }
    }

    /// Get possible solutions
    pub fn suggested_actions(&self) -> Vec<String> {
        match self {
            ActrCliError::Config { .. } => vec![
                "Check Actr.toml file syntax".to_string(),
                "Run 'actr config test' to validate config".to_string(),
                "Refer to config examples in documentation".to_string(),
            ],
            ActrCliError::Network { .. } => vec![
                "Check network connection".to_string(),
                "Verify service address is correct".to_string(),
                "Check firewall settings".to_string(),
                "Run 'actr check --verbose' for details".to_string(),
            ],
            ActrCliError::Dependency { .. } => vec![
                "Run 'actr check' to check dependency status".to_string(),
                "Run 'actr install' to install missing dependencies".to_string(),
                "Run 'actr discovery' to find available services".to_string(),
            ],
            ActrCliError::ValidationFailed { .. } => vec![
                "Check and fix reported issues".to_string(),
                "Run 'actr check --verbose' for detailed diagnostics".to_string(),
                "Ensure all dependency services are available".to_string(),
            ],
            ActrCliError::InstallFailed { .. } => vec![
                "Check disk space".to_string(),
                "Check network connection".to_string(),
                "Run 'actr check' to validate environment".to_string(),
                "Try clearing cache and retry".to_string(),
            ],
            _ => vec!["View detailed error information".to_string()],
        }
    }

    /// Get related documentation links
    pub fn documentation_links(&self) -> Vec<(&str, &str)> {
        match self {
            ActrCliError::Config { .. } => vec![
                ("Config Docs", "https://docs.actor-rtc.com/config"),
                (
                    "Actr.toml Reference",
                    "https://docs.actor-rtc.com/actr-toml",
                ),
            ],
            ActrCliError::Dependency { .. } => vec![
                (
                    "Dependency Management",
                    "https://docs.actor-rtc.com/dependencies",
                ),
                (
                    "Troubleshooting",
                    "https://docs.actor-rtc.com/troubleshooting",
                ),
            ],
            _ => vec![("User Guide", "https://docs.actor-rtc.com/guide")],
        }
    }
}

/// Convert validation report to error
impl From<super::components::ValidationReport> for ActrCliError {
    fn from(report: super::components::ValidationReport) -> Self {
        let mut details = Vec::new();

        if !report.config_validation.is_valid {
            details.extend(
                report
                    .config_validation
                    .errors
                    .iter()
                    .map(|e| format!("Config error: {e}")),
            );
        }

        for dep in &report.dependency_validation {
            if !dep.is_available {
                details.push(format!(
                    "Dependency unavailable: {} - {}",
                    dep.dependency,
                    dep.error.as_deref().unwrap_or("unknown error")
                ));
            }
        }

        for net in &report.network_validation {
            if !net.is_reachable {
                details.push(format!(
                    "Network unreachable: {}",
                    net.error.as_deref().unwrap_or("connection failed")
                ));
            }
        }

        for fp in &report.fingerprint_validation {
            if !fp.is_valid {
                details.push(format!(
                    "Fingerprint validation failed: {} - {}",
                    fp.dependency,
                    fp.error.as_deref().unwrap_or("fingerprint mismatch")
                ));
            }
        }

        for conflict in &report.conflicts {
            details.push(format!("Dependency conflict: {}", conflict.description));
        }

        ActrCliError::ValidationFailed {
            details: details.join("; "),
        }
    }
}

/// Error Report Formatter
pub struct ErrorReporter;

impl ErrorReporter {
    /// Format error report
    pub fn format_error(error: &ActrCliError) -> String {
        let mut output = Vec::new();

        // Main error message
        output.push(error.user_message());
        output.push(String::new());

        // Suggested solutions
        let actions = error.suggested_actions();
        if !actions.is_empty() {
            output.push("🔧 Suggested solutions:".to_string());
            for (i, action) in actions.iter().enumerate() {
                output.push(format!("   {}. {}", i + 1, action));
            }
            output.push(String::new());
        }

        // Documentation links
        let docs = error.documentation_links();
        if !docs.is_empty() {
            output.push("📚 Related documentation:".to_string());
            for (title, url) in docs {
                output.push(format!("   • {title}: {url}"));
            }
            output.push(String::new());
        }

        output.join("\n")
    }

    /// Format validation report
    pub fn format_validation_report(report: &super::components::ValidationReport) -> String {
        let mut output = vec![
            "🔍 Dependency Validation Report".to_string(),
            "=".repeat(50),
            String::new(),
            "📋 Config file validation:".to_string(),
        ];

        // Config validation
        if report.config_validation.is_valid {
            output.push("   ✅ Passed".to_string());
        } else {
            output.push("   ❌ Failed".to_string());
            for error in &report.config_validation.errors {
                output.push(format!("      • {error}"));
            }
        }
        output.push(String::new());

        // Dependency validation
        output.push("📦 Dependency availability:".to_string());
        for dep in &report.dependency_validation {
            if dep.is_available {
                output.push(format!("   ✅ {} - available", dep.dependency));
            } else {
                output.push(format!(
                    "   ❌ {} - {}",
                    dep.dependency,
                    dep.error.as_deref().unwrap_or("unavailable")
                ));
            }
        }
        output.push(String::new());

        // Network validation
        output.push("🌐 Network connectivity:".to_string());
        for net in &report.network_validation {
            if net.is_reachable {
                let latency = net
                    .latency_ms
                    .map(|ms| format!(" ({ms}ms)"))
                    .unwrap_or_default();
                output.push(format!("   ✅ Connected{}", latency));
            } else {
                output.push(format!(
                    "   ❌ Connection failed - {}",
                    net.error.as_deref().unwrap_or("unreachable")
                ));
            }
        }
        output.push(String::new());

        // Fingerprint validation
        if !report.fingerprint_validation.is_empty() {
            output.push("🔐 Fingerprint validation:".to_string());
            for fp in &report.fingerprint_validation {
                if fp.is_valid {
                    output.push(format!("   ✅ {} - passed", fp.dependency));
                } else {
                    output.push(format!(
                        "   ❌ {} - {}",
                        fp.dependency,
                        fp.error.as_deref().unwrap_or("validation failed")
                    ));
                }
            }
            output.push(String::new());
        }

        // Conflict report
        if !report.conflicts.is_empty() {
            output.push("⚠️ Dependency conflicts:".to_string());
            for conflict in &report.conflicts {
                output.push(format!(
                    "   • {} vs {}: {}",
                    conflict.dependency_a, conflict.dependency_b, conflict.description
                ));
            }
            output.push(String::new());
        }

        // Summary
        if report.is_success() {
            output.push("✨ Overall: All validations passed".to_string());
        } else {
            output.push("❌ Overall: Issues need to be resolved".to_string());
        }

        output.join("\n")
    }
}
