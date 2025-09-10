//! Example showing how to parse an actr.toml configuration file

use actr_config::{ActrConfig, ProtoDependency, RoutingRule};
use std::env;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get config path from command line or use default
    let config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "../actr.toml".to_string());

    println!("🔧 Loading configuration from: {}", config_path);
    
    // Load and parse the configuration
    let config = ActrConfig::from_file(&config_path)?;
    
    println!("\n📦 Package Information:");
    println!("  Name: {}", config.package.name);
    println!("  Version: {}", config.package.version);
    println!("  Edition: {}", config.package.edition);
    if let Some(ref description) = config.package.description {
        println!("  Description: {}", description);
    }
    if let Some(ref authors) = config.package.authors {
        println!("  Authors: {}", authors.join(", "));
    }
    if let Some(ref license) = config.package.license {
        println!("  License: {}", license);
    }

    println!("\n📚 Proto Dependencies:");
    if config.dependencies.protos.dependencies.is_empty() {
        println!("  (none)");
    } else {
        for (name, dep) in &config.dependencies.protos.dependencies {
            println!("  {}: {}", name, dep.description());
            match dep {
                ProtoDependency::Git { git, path, tag, branch, rev } => {
                    println!("    Repository: {}", git);
                    println!("    Proto path: {}", path);
                    if let Some(tag) = tag {
                        println!("    Tag: {}", tag);
                    }
                    if let Some(branch) = branch {
                        println!("    Branch: {}", branch);
                    }
                    if let Some(rev) = rev {
                        println!("    Revision: {}", rev);
                    }
                }
                ProtoDependency::Http { url } => {
                    println!("    URL: {}", url);
                }
                ProtoDependency::Local { path } => {
                    println!("    Local path: {}", path);
                }
            }
        }
    }

    println!("\n🚦 Routing Rules:");
    if config.routing.rules.is_empty() {
        println!("  (none)");
    } else {
        for (message_type, rule) in &config.routing.rules {
            match rule {
                RoutingRule::Call { call } => {
                    println!("  {} → call → {}", message_type, call);
                }
                RoutingRule::Tell { tell } => {
                    println!("  {} → tell → {}", message_type, tell);
                }
                RoutingRule::Publish { publish } => {
                    println!("  {} → publish → {}", message_type, publish);
                }
            }
        }
    }

    println!("\n🎯 Configuration Mode:");
    if config.is_auto_runner_mode() {
        println!("  Auto-runner mode (no main.rs)");
    } else {
        println!("  Library mode (main.rs exists)");
    }

    println!("\n✅ Configuration successfully parsed and validated!");

    Ok(())
}