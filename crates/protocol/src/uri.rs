//! # Actor-RTC URI 解析库
//!
//! 提供 actr:// 协议的标准 URI 解析功能，不包含业务逻辑。

use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use thiserror::Error;

/// Actor-RTC URI 解析错误
#[derive(Error, Debug)]
pub enum ActrUriError {
    #[error("Invalid URI scheme, expected 'actr' but got '{0}'")]
    InvalidScheme(String),

    #[error("Missing actor type in URI")]
    MissingActorType,

    #[error("URI parse error: {0}")]
    ParseError(String),
}

/// Actor-RTC URI 结构
/// 格式: actr://<actor-type>/<path>?<query-params>
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActrUri {
    /// Actor 类型（如 "user-service"）
    pub actor_type: String,

    /// 路径（可选，如 "user.proto"、"api/v1" 等）
    pub path: Option<String>,

    /// 查询参数
    pub query_params: std::collections::HashMap<String, String>,
}

impl ActrUri {
    /// 创建新的 Actor-RTC URI
    pub fn new(actor_type: String) -> Self {
        Self {
            actor_type,
            path: None,
            query_params: std::collections::HashMap::new(),
        }
    }

    /// 设置路径
    pub fn with_path(mut self, path: String) -> Self {
        self.path = Some(path);
        self
    }

    /// 添加查询参数
    pub fn with_query_param(mut self, key: String, value: String) -> Self {
        self.query_params.insert(key, value);
        self
    }

    /// 获取 scheme 信息
    pub fn scheme(&self) -> &'static str {
        "actr"
    }
}

impl Display for ActrUri {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut uri = format!("actr://{}", self.actor_type);

        if let Some(ref path) = self.path {
            if !path.starts_with('/') {
                uri.push('/');
            }
            uri.push_str(path);
        } else {
            uri.push('/');
        }

        if !self.query_params.is_empty() {
            uri.push('?');
            let params: Vec<String> = self
                .query_params
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            uri.push_str(&params.join("&"));
        }

        write!(f, "{uri}")
    }
}

impl FromStr for ActrUri {
    type Err = ActrUriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if !s.starts_with("actr://") {
            return Err(ActrUriError::InvalidScheme(
                s.split(':').next().unwrap_or("").to_string(),
            ));
        }

        let without_scheme = &s[7..];

        let (base_part, query_part) = if let Some(idx) = without_scheme.find('?') {
            (&without_scheme[..idx], Some(&without_scheme[idx + 1..]))
        } else {
            (without_scheme, None)
        };

        let (actor_type, path) = if let Some(idx) = base_part.find('/') {
            let actor_type = &base_part[..idx];
            let path_part = &base_part[idx + 1..];

            if actor_type.is_empty() {
                return Err(ActrUriError::MissingActorType);
            }

            let path = if path_part.is_empty() {
                None
            } else {
                Some(path_part.to_string())
            };

            (actor_type.to_string(), path)
        } else {
            if base_part.is_empty() {
                return Err(ActrUriError::MissingActorType);
            }
            (base_part.to_string(), None)
        };

        let mut query_params = std::collections::HashMap::new();
        if let Some(query) = query_part {
            for param in query.split('&') {
                if let Some(idx) = param.find('=') {
                    let key = param[..idx].to_string();
                    let value = param[idx + 1..].to_string();
                    query_params.insert(key, value);
                } else if !param.is_empty() {
                    query_params.insert(param.to_string(), String::new());
                }
            }
        }

        Ok(ActrUri {
            actor_type,
            path,
            query_params,
        })
    }
}

/// Actor-RTC URI 构建器
#[derive(Debug, Default)]
pub struct ActrUriBuilder {
    actor_type: Option<String>,
    path: Option<String>,
    query_params: std::collections::HashMap<String, String>,
}

impl ActrUriBuilder {
    /// 创建新的构建器
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置 Actor 类型
    pub fn actor_type<S: Into<String>>(mut self, actor_type: S) -> Self {
        self.actor_type = Some(actor_type.into());
        self
    }

    /// 设置路径
    pub fn path<S: Into<String>>(mut self, path: S) -> Self {
        self.path = Some(path.into());
        self
    }

    /// 添加查询参数
    pub fn query<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.query_params.insert(key.into(), value.into());
        self
    }

    /// 构建 URI
    pub fn build(self) -> Result<ActrUri, ActrUriError> {
        let actor_type = self.actor_type.ok_or(ActrUriError::MissingActorType)?;

        Ok(ActrUri {
            actor_type,
            path: self.path,
            query_params: self.query_params,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_uri_parsing() {
        let uri = "actr://user-service/".parse::<ActrUri>().unwrap();
        assert_eq!(uri.actor_type, "user-service");
        assert_eq!(uri.path, None);
        assert!(uri.query_params.is_empty());
    }

    #[test]
    fn test_uri_with_path() {
        let uri = "actr://user-service/api/v1".parse::<ActrUri>().unwrap();
        assert_eq!(uri.actor_type, "user-service");
        assert_eq!(uri.path, Some("api/v1".to_string()));
    }

    #[test]
    fn test_uri_with_query_params() {
        let uri = "actr://notification-service/?param1=value1&param2=value2"
            .parse::<ActrUri>()
            .unwrap();
        assert_eq!(uri.actor_type, "notification-service");
        assert_eq!(uri.path, None);
        assert_eq!(uri.query_params.get("param1"), Some(&"value1".to_string()));
        assert_eq!(uri.query_params.get("param2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_uri_without_trailing_slash() {
        let uri = "actr://payment-service".parse::<ActrUri>().unwrap();
        assert_eq!(uri.actor_type, "payment-service");
        assert_eq!(uri.path, None);
    }

    #[test]
    fn test_uri_builder() {
        let uri = ActrUriBuilder::new()
            .actor_type("order-service")
            .path("orders/create")
            .query("timeout", "30s")
            .build()
            .unwrap();

        assert_eq!(uri.actor_type, "order-service");
        assert_eq!(uri.path, Some("orders/create".to_string()));
        assert_eq!(uri.query_params.get("timeout"), Some(&"30s".to_string()));
    }

    #[test]
    fn test_uri_to_string() {
        let uri = ActrUri::new("user-service".to_string())
            .with_path("users/profile".to_string())
            .with_query_param("format".to_string(), "json".to_string());

        let uri_string = uri.to_string();
        assert!(uri_string.starts_with("actr://user-service"));
        assert!(uri_string.contains("users/profile"));
        assert!(uri_string.contains("format=json"));
    }

    #[test]
    fn test_invalid_scheme() {
        let result = "http://user-service/".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::InvalidScheme(_))));
    }

    #[test]
    fn test_missing_actor_type() {
        let result = "actr:///".parse::<ActrUri>();
        assert!(matches!(result, Err(ActrUriError::MissingActorType)));
    }

    #[test]
    fn test_empty_query_param() {
        let uri = "actr://service/?flag".parse::<ActrUri>().unwrap();
        assert_eq!(uri.query_params.get("flag"), Some(&"".to_string()));
    }

    #[test]
    fn test_complex_path() {
        let uri = "actr://api-gateway/v2/users/123/profile"
            .parse::<ActrUri>()
            .unwrap();
        assert_eq!(uri.actor_type, "api-gateway");
        assert_eq!(uri.path, Some("v2/users/123/profile".to_string()));
    }
}
