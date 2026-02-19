//! Historical compatibility aliases.
//!
//! Keep all legacy `supervit` naming in one place to avoid leaking
//! compatibility concerns across the SDK surface.

/// Compatibility alias for historical `supervit` naming.
pub type SupervitClient = admin::AdminClient;

/// Compatibility alias for historical `supervit` naming.
pub type SupervitConfig = admin::AdminConfig;

/// Compatibility alias for historical `supervit` naming.
pub type Supervisord = admin::AdminApiService;

/// Compatibility alias for historical `supervit` naming.
pub type SupervitError = admin::AdminError;

/// Compatibility alias for historical `supervit` naming.
pub type Result<T> = admin::AdminResult<T>;
