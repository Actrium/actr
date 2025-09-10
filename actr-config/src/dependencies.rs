//! Proto dependency configuration structures

use crate::error::{ActrConfigError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::Url;

/// External proto dependencies configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtoDependencies {
    /// Map of dependency aliases to their sources
    #[serde(flatten)]
    pub dependencies: HashMap<String, ProtoDependency>,
}

impl Default for ProtoDependencies {
    fn default() -> Self {
        Self {
            dependencies: HashMap::new(),
        }
    }
}

/// A single proto dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProtoDependency {
    /// Git repository dependency
    Git {
        /// Git repository URL
        git: String,
        /// Path to the .proto file within the repository
        path: String,
        /// Git tag to use
        #[serde(skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
        /// Git branch to use
        #[serde(skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        /// Git revision (commit hash) to use
        #[serde(skip_serializing_if = "Option::is_none")]
        rev: Option<String>,
    },
    /// HTTP/HTTPS URL dependency
    Http {
        /// Direct URL to download the .proto file
        url: String,
    },
    /// Local file dependency
    Local {
        /// Local file path
        path: String,
    },
}

impl ProtoDependency {
    /// Validate the dependency configuration
    pub fn validate(&self) -> Result<()> {
        match self {
            ProtoDependency::Git { git, path, tag, branch, rev } => {
                // Validate git URL
                if git.is_empty() {
                    return Err(ActrConfigError::InvalidDependency(
                        "Git URL cannot be empty".to_string(),
                    ));
                }

                // Validate path
                if path.is_empty() {
                    return Err(ActrConfigError::InvalidDependency(
                        "Proto file path cannot be empty".to_string(),
                    ));
                }

                // Check that only one version specifier is provided
                let version_count = [tag.is_some(), branch.is_some(), rev.is_some()]
                    .iter()
                    .filter(|&&x| x)
                    .count();

                if version_count > 1 {
                    return Err(ActrConfigError::InvalidDependency(
                        "Only one of 'tag', 'branch', or 'rev' can be specified".to_string(),
                    ));
                }

                Ok(())
            }
            ProtoDependency::Http { url } => {
                // Validate URL format
                Url::parse(url).map_err(ActrConfigError::UrlError)?;
                Ok(())
            }
            ProtoDependency::Local { path } => {
                if path.is_empty() {
                    return Err(ActrConfigError::InvalidDependency(
                        "Local path cannot be empty".to_string(),
                    ));
                }
                Ok(())
            }
        }
    }

    /// Get a human-readable description of this dependency
    pub fn description(&self) -> String {
        match self {
            ProtoDependency::Git { git, tag, branch, rev, .. } => {
                let version = tag.as_ref()
                    .map(|t| format!("@{}", t))
                    .or_else(|| branch.as_ref().map(|b| format!("#{}", b)))
                    .or_else(|| rev.as_ref().map(|r| format!(":{}", &r[..8])))
                    .unwrap_or_default();
                format!("git:{}{}", git, version)
            }
            ProtoDependency::Http { url } => format!("http:{}", url),
            ProtoDependency::Local { path } => format!("local:{}", path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_dependency_validation() {
        let dep = ProtoDependency::Git {
            git: "https://github.com/example/protos.git".to_string(),
            path: "user/v1/user.proto".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
            rev: None,
        };
        assert!(dep.validate().is_ok());
    }

    #[test]
    fn test_invalid_git_dependency() {
        let dep = ProtoDependency::Git {
            git: "".to_string(),
            path: "user.proto".to_string(),
            tag: None,
            branch: None,
            rev: None,
        };
        assert!(dep.validate().is_err());
    }

    #[test]
    fn test_multiple_version_specifiers() {
        let dep = ProtoDependency::Git {
            git: "https://github.com/example/protos.git".to_string(),
            path: "user.proto".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: Some("main".to_string()),
            rev: None,
        };
        assert!(dep.validate().is_err());
    }

    #[test]
    fn test_http_dependency_validation() {
        let dep = ProtoDependency::Http {
            url: "https://example.com/schema/user.proto".to_string(),
        };
        assert!(dep.validate().is_ok());
    }

    #[test]
    fn test_local_dependency_validation() {
        let dep = ProtoDependency::Local {
            path: "../common/protos/user.proto".to_string(),
        };
        assert!(dep.validate().is_ok());
    }
}