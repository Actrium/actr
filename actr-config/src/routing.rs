//! Message routing configuration structures

use crate::error::{ActrConfigError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Message routing rules configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoutingConfig {
    /// Map of message types to their routing rules
    #[serde(flatten)]
    pub rules: HashMap<String, RoutingRule>,
}

/// A single routing rule that defines how a message type should be routed
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RoutingRule {
    /// Call-based routing (request-response)
    Call {
        /// Target service to call
        call: String,
    },
    /// Tell-based routing (fire-and-forget)
    Tell {
        /// Target service to send message to
        tell: String,
    },
    /// Publish-subscribe routing
    Publish {
        /// Topic to publish message to
        publish: String,
    },
}

impl RoutingRule {
    /// Validate the routing rule
    pub fn validate(&self) -> Result<()> {
        match self {
            RoutingRule::Call { call } => {
                if call.is_empty() {
                    return Err(ActrConfigError::InvalidRoutingRule(
                        "Call target cannot be empty".to_string(),
                    ));
                }
                Ok(())
            }
            RoutingRule::Tell { tell } => {
                if tell.is_empty() {
                    return Err(ActrConfigError::InvalidRoutingRule(
                        "Tell target cannot be empty".to_string(),
                    ));
                }
                Ok(())
            }
            RoutingRule::Publish { publish } => {
                if publish.is_empty() {
                    return Err(ActrConfigError::InvalidRoutingRule(
                        "Publish topic cannot be empty".to_string(),
                    ));
                }
                Ok(())
            }
        }
    }

    /// Get the target of this routing rule
    pub fn target(&self) -> &str {
        match self {
            RoutingRule::Call { call } => call,
            RoutingRule::Tell { tell } => tell,
            RoutingRule::Publish { publish } => publish,
        }
    }

    /// Get the routing type as a string
    pub fn routing_type(&self) -> &'static str {
        match self {
            RoutingRule::Call { .. } => "call",
            RoutingRule::Tell { .. } => "tell",
            RoutingRule::Publish { .. } => "publish",
        }
    }

    /// Create a new Call routing rule
    pub fn call(target: impl Into<String>) -> Self {
        Self::Call {
            call: target.into(),
        }
    }

    /// Create a new Tell routing rule
    pub fn tell(target: impl Into<String>) -> Self {
        Self::Tell {
            tell: target.into(),
        }
    }

    /// Create a new Publish routing rule
    pub fn publish(topic: impl Into<String>) -> Self {
        Self::Publish {
            publish: topic.into(),
        }
    }
}

impl RoutingConfig {
    /// Create a new empty routing configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a routing rule for a message type
    pub fn add_rule(&mut self, message_type: impl Into<String>, rule: RoutingRule) {
        self.rules.insert(message_type.into(), rule);
    }

    /// Get a routing rule for a message type
    pub fn get_rule(&self, message_type: &str) -> Option<&RoutingRule> {
        self.rules.get(message_type)
    }

    /// Validate all routing rules
    pub fn validate(&self) -> Result<()> {
        for (message_type, rule) in &self.rules {
            rule.validate().map_err(|e| {
                ActrConfigError::InvalidRoutingRule(format!(
                    "Invalid rule for message type '{}': {}",
                    message_type, e
                ))
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_call_routing_rule() {
        let rule = RoutingRule::call("user.v1.UserService");
        assert!(rule.validate().is_ok());
        assert_eq!(rule.target(), "user.v1.UserService");
        assert_eq!(rule.routing_type(), "call");
    }

    #[test]
    fn test_tell_routing_rule() {
        let rule = RoutingRule::tell("email.v1.EmailService");
        assert!(rule.validate().is_ok());
        assert_eq!(rule.target(), "email.v1.EmailService");
        assert_eq!(rule.routing_type(), "tell");
    }

    #[test]
    fn test_publish_routing_rule() {
        let rule = RoutingRule::publish("user-events");
        assert!(rule.validate().is_ok());
        assert_eq!(rule.target(), "user-events");
        assert_eq!(rule.routing_type(), "publish");
    }

    #[test]
    fn test_empty_target_validation() {
        let rule = RoutingRule::call("");
        assert!(rule.validate().is_err());
    }

    #[test]
    fn test_routing_config() {
        let mut config = RoutingConfig::new();
        config.add_rule("user.v1.GetUserRequest", RoutingRule::call("user.v1.UserService"));
        config.add_rule("email.v1.SendEmailRequest", RoutingRule::tell("email.v1.EmailService"));

        assert!(config.validate().is_ok());
        assert!(config.get_rule("user.v1.GetUserRequest").is_some());
        assert!(config.get_rule("nonexistent.Message").is_none());
    }
}