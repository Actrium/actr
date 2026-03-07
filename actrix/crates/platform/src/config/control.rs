use serde::{Deserialize, Serialize};

/// Control 头类型。
///
/// - `admin_ui`: 提供本地管理 UI（HTTP）。
/// - `grpc_api`: 提供给集群 supervisor 的 gRPC API（复用主 HTTP 端口）。
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ControlHead {
    #[default]
    AdminUi,
    GrpcApi,
}

/// gRPC 头配置（仅当 `head = "grpc_api"` 时生效）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ControlGrpcApiConfig {
    /// 节点 ID（用于认证载荷）
    #[serde(default = "default_grpc_node_id")]
    pub node_id: String,

    /// 节点展示名（为空时回退到 node_id）
    #[serde(default = "default_grpc_node_name")]
    pub node_name: String,

    /// nonce-auth 共享密钥（hex, 至少 64 个字符）
    #[serde(default)]
    pub shared_secret: String,

    /// 允许的最大时钟偏差（秒）
    #[serde(default = "default_max_clock_skew_secs")]
    pub max_clock_skew_secs: u64,
}

/// Admin UI 配置（仅当 `head = "admin_ui"` 时生效）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AdminUiConfig {
    /// 登录密码（admin_ui 模式必填，≥8 字符）
    #[serde(default)]
    pub password: String,

    /// JWT 会话过期时间（秒），默认 86400（24 小时）
    #[serde(default = "default_session_expiry_secs")]
    pub session_expiry_secs: u64,
}

/// Control 常驻配置。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ControlConfig {
    /// 二选一头模式
    #[serde(default)]
    pub head: ControlHead,

    /// gRPC 头参数（仅 grpc_api 模式使用）
    #[serde(default)]
    pub grpc_api: ControlGrpcApiConfig,

    /// Admin UI 参数（仅 admin_ui 模式使用）
    #[serde(default)]
    pub admin_ui: AdminUiConfig,
}

fn default_grpc_node_id() -> String {
    "actrix-node".to_string()
}

fn default_grpc_node_name() -> String {
    "actrix-node".to_string()
}

fn default_max_clock_skew_secs() -> u64 {
    300
}

fn default_session_expiry_secs() -> u64 {
    86400
}

impl Default for AdminUiConfig {
    fn default() -> Self {
        Self {
            password: String::new(),
            session_expiry_secs: default_session_expiry_secs(),
        }
    }
}

impl Default for ControlGrpcApiConfig {
    fn default() -> Self {
        Self {
            node_id: default_grpc_node_id(),
            node_name: default_grpc_node_name(),
            shared_secret: String::new(),
            max_clock_skew_secs: default_max_clock_skew_secs(),
        }
    }
}

impl Default for ControlConfig {
    fn default() -> Self {
        Self {
            head: ControlHead::AdminUi,
            grpc_api: ControlGrpcApiConfig::default(),
            admin_ui: AdminUiConfig::default(),
        }
    }
}

impl ControlGrpcApiConfig {
    pub fn effective_node_name(&self) -> String {
        let trimmed = self.node_name.trim();
        if trimmed.is_empty() {
            self.node_id.trim().to_string()
        } else {
            trimmed.to_string()
        }
    }
}

impl ControlConfig {
    pub fn validate(&self) -> Result<(), String> {
        match self.head {
            ControlHead::AdminUi => self.validate_admin_ui(),
            ControlHead::GrpcApi => self.validate_grpc_api(),
        }
    }

    fn validate_admin_ui(&self) -> Result<(), String> {
        let cfg = &self.admin_ui;

        if cfg.password.is_empty() {
            return Err("control.admin_ui.password is required when head = admin_ui".to_string());
        }

        if cfg.password.len() < 8 {
            return Err("control.admin_ui.password must be at least 8 characters".to_string());
        }

        if cfg.session_expiry_secs == 0 {
            return Err("control.admin_ui.session_expiry_secs must be greater than 0".to_string());
        }

        Ok(())
    }

    fn validate_grpc_api(&self) -> Result<(), String> {
        let cfg = &self.grpc_api;

        if cfg.node_id.trim().is_empty() {
            return Err("control.grpc_api.node_id cannot be empty".to_string());
        }

        if cfg.shared_secret.trim().is_empty() {
            return Err(
                "control.grpc_api.shared_secret is required when head = grpc_api".to_string(),
            );
        }

        if cfg.shared_secret.len() < 64 {
            return Err(
                "control.grpc_api.shared_secret must be at least 64 hex characters (32 bytes)"
                    .to_string(),
            );
        }

        if hex::decode(&cfg.shared_secret).is_err() {
            return Err("control.grpc_api.shared_secret must be a valid hex string".to_string());
        }

        if cfg.max_clock_skew_secs == 0 {
            return Err("control.grpc_api.max_clock_skew_secs must be greater than 0".to_string());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_control_head_is_admin_ui() {
        let cfg = ControlConfig::default();
        assert_eq!(cfg.head, ControlHead::AdminUi);
    }

    #[test]
    fn admin_ui_requires_password() {
        let cfg = ControlConfig {
            head: ControlHead::AdminUi,
            admin_ui: AdminUiConfig {
                password: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(cfg.validate().is_err());
    }

    #[test]
    fn admin_ui_rejects_short_password() {
        let cfg = ControlConfig {
            head: ControlHead::AdminUi,
            admin_ui: AdminUiConfig {
                password: "short".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(cfg.validate().is_err());
    }

    #[test]
    fn admin_ui_accepts_valid_password() {
        let cfg = ControlConfig {
            head: ControlHead::AdminUi,
            admin_ui: AdminUiConfig {
                password: "changeme123".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn grpc_api_requires_shared_secret() {
        let cfg = ControlConfig {
            head: ControlHead::GrpcApi,
            grpc_api: ControlGrpcApiConfig {
                shared_secret: String::new(),
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(cfg.validate().is_err());
    }

    #[test]
    fn grpc_api_accepts_valid_secret() {
        let cfg = ControlConfig {
            head: ControlHead::GrpcApi,
            grpc_api: ControlGrpcApiConfig {
                shared_secret: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        assert!(cfg.validate().is_ok());
    }
}
