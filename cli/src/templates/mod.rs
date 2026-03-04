//! Project template system

use crate::error::{ActrCliError, Result};
use handlebars::Handlebars;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct TemplateContext {
    pub project_name: String,
    pub project_name_snake: String,
    pub project_name_pascal: String,
}

impl TemplateContext {
    pub fn new(project_name: &str) -> Self {
        Self {
            project_name: project_name.to_string(),
            project_name_snake: to_snake_case(project_name),
            project_name_pascal: to_pascal_case(project_name),
        }
    }
}

pub struct ProjectTemplate {
    #[allow(dead_code)]
    name: String,
    files: HashMap<String, String>,
}

impl ProjectTemplate {
    pub fn load(template_name: &str) -> Result<Self> {
        match template_name {
            "basic" => Ok(Self::basic_template()),
            "echo" => Ok(Self::echo_template()),
            _ => Err(ActrCliError::InvalidProject(format!(
                "Unknown template: {template_name}"
            ))),
        }
    }

    pub fn generate(&self, project_path: &Path, context: &TemplateContext) -> Result<()> {
        let handlebars = Handlebars::new();

        for (file_path, content) in &self.files {
            let rendered_path = handlebars.render_template(file_path, context)?;
            let rendered_content = handlebars.render_template(content, context)?;

            let full_path = project_path.join(&rendered_path);

            // Create parent directories if they don't exist
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            std::fs::write(full_path, rendered_content)?;
        }

        Ok(())
    }

    fn basic_template() -> Self {
        let mut files = HashMap::new();

        // Cargo.toml
        files.insert(
            "Cargo.toml".to_string(),
            r#"[package]
name = "{{project_name}}"
version = "0.1.0"
edition = "2021"

[dependencies]
# Actor-RTC framework
actor-rtc-framework = { path = "../../actor-rtc-framework" }  # Adjust path as needed

# Async runtime
tokio = { version = "1.0", features = ["full"] }
async-trait = "0.1"

# Protocol definitions
tonic = "0.10"
prost = "0.12"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"

# Error handling
anyhow = "1.0"

[build-dependencies]
tonic-build = "0.10"
"#
            .to_string(),
        );

        // src/lib.rs for auto-runner mode
        files.insert(
            "src/lib.rs".to_string(),
            r#"//! {{project_name}} - Actor-RTC service implementation

use actor_rtc_framework::prelude::*;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::info;

// Include generated proto code
pub mod greeter {
    tonic::include_proto!("greeter");
}

// Include generated actor code
include!(concat!(env!("OUT_DIR"), "/greeter_service_actor.rs"));

use greeter::{GreetRequest, GreetResponse};

/// Main actor implementation
#[derive(Default)]
pub struct {{project_name_pascal}}Actor {
    greeting_count: std::sync::atomic::AtomicU64,
}

#[async_trait]
impl IGreeterService for {{project_name_pascal}}Actor {
    async fn greet(
        &self, 
        request: GreetRequest,
        _context: Arc<Context>
    ) -> Result<GreetResponse, tonic::Status> {
        let count = self.greeting_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        
        info!("Received greeting request #{}: Hello {}", count, request.name);
        
        let message = format!("Hello, {}! This is greeting #{}", request.name, count);
        
        Ok(GreetResponse { message })
    }
}

#[async_trait]
impl ILifecycle for {{project_name_pascal}}Actor {
    async fn on_start(&self, _ctx: Arc<Context>) {
        info!("{{project_name_pascal}}Actor started successfully");
    }

    async fn on_stop(&self, _ctx: Arc<Context>) {
        info!("{{project_name_pascal}}Actor shutting down");
    }

    async fn on_actor_discovered(&self, _ctx: Arc<Context>, _actor_id: &ActorId) -> bool {
        // Accept connections from any actor
        true
    }
}
"#
            .to_string(),
        );

        // proto/greeter.proto
        files.insert(
            "protos/greeter.proto".to_string(),
            r#"syntax = "proto3";

package greeter;

// Greeting request message
message GreetRequest {
    string name = 1;
}

// Greeting response message
message GreetResponse {
    string message = 1;
}

// Greeter service definition
service GreeterService {
    rpc Greet(GreetRequest) returns (GreetResponse);
}
"#
            .to_string(),
        );

        // build.rs
        files.insert(
            "build.rs".to_string(),
            r#"fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto_files = ["protos/greeter.proto"];

    // Build with protoc-gen-actrframework plugin if available
    let plugin_path = std::env::current_dir()?
        .parent()
        .unwrap()
        .join("target/debug/protoc-gen-actrframework");
    
    let mut config = tonic_build::configure()
        .build_server(false)  // We generate our own server-side traits
        .build_client(true);

    if plugin_path.exists() {
        config = config
            .protoc_arg(format!("--plugin=protoc-gen-actrframework={}", plugin_path.display()))
            .protoc_arg("--actrframework_out=.");
        println!("Using protoc-gen-actrframework plugin");
    } else {
        println!("Warning: protoc-gen-actrframework plugin not found, using standard tonic generation only");
    }

    config.compile(&proto_files, &["protos/"])?;

    // Re-run if proto files change
    for proto_file in &proto_files {
        println!("cargo:rerun-if-changed={}", proto_file);
    }

    Ok(())
}
"#.to_string(),
        );

        // README.md
        files.insert(
            "README.md".to_string(),
            r#"# {{project_name}}

An Actor-RTC service implementation.

## Building

```bash
actr gen --input proto --output src/generated
```

## Running

```bash
actr run
```

## Development

This project uses the Actor-RTC framework's auto-runner mode. The main logic is implemented in `src/lib.rs`, and the framework automatically generates the necessary startup code.

The service definition is in `protos/greeter.proto`, and the implementation is in the `{{project_name_pascal}}Actor` struct.
"#.to_string(),
        );

        Self {
            name: "basic".to_string(),
            files,
        }
    }

    fn echo_template() -> Self {
        // Similar to basic but with echo-specific content
        // For now, just return the basic template
        Self::basic_template()
    }
}

fn to_snake_case(s: &str) -> String {
    let mut result = String::new();
    let chars = s.chars().peekable();

    for c in chars {
        if c.is_uppercase() {
            if !result.is_empty() {
                result.push('_');
            }
            result.push(c.to_lowercase().next().unwrap());
        } else if c.is_alphanumeric() {
            result.push(c);
        } else {
            result.push('_');
        }
    }

    result
}

fn to_pascal_case(s: &str) -> String {
    // Handle already PascalCase or camelCase strings by detecting uppercase transitions
    let mut result = String::new();
    let chars = s.chars().peekable();
    let mut start_of_word = true;

    for c in chars {
        if !c.is_alphanumeric() {
            // Separator - next char starts a new word
            start_of_word = true;
            continue;
        }

        if c.is_uppercase() {
            // Uppercase marks start of a word
            result.push(c);
            start_of_word = false;
        } else if start_of_word {
            // First char of a word - capitalize it
            result.push(c.to_uppercase().next().unwrap());
            start_of_word = false;
        } else {
            // Middle of a word - keep as lowercase
            result.push(c.to_lowercase().next().unwrap());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_snake_case() {
        assert_eq!(to_snake_case("MyProject"), "my_project");
        assert_eq!(to_snake_case("my-project"), "my_project");
        assert_eq!(to_snake_case("my_project"), "my_project");
    }

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("my-project"), "MyProject");
        assert_eq!(to_pascal_case("my_project"), "MyProject");
        assert_eq!(to_pascal_case("MyProject"), "MyProject");
    }
}
