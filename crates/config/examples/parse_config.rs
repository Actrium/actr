//! Example showing how to parse an Actr.toml configuration file

use actr_config::ConfigParser;
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get config path from command line or use default
    let config_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "../Actr.toml".to_string());

    println!("🔧 Loading configuration from: {config_path}");

    // Load and parse the configuration
    let config = ConfigParser::from_file(&config_path)?;

    println!("\n📦 Package Information:");
    println!("  Name: {}", config.package.name);
    println!("  Manufacturer: {}", config.package.actr_type.manufacturer);
    println!("  Type: {}", config.package.actr_type.name);
    if let Some(ref description) = config.package.description {
        println!("  Description: {description}");
    }
    if !config.package.authors.is_empty() {
        println!("  Authors: {}", config.package.authors.join(", "));
    }
    if let Some(ref license) = config.package.license {
        println!("  License: {license}");
    }

    println!("\n📦 Exports:");
    if config.exports.is_empty() {
        println!("  (none)");
    } else {
        for proto in &config.exports {
            println!("  {}", proto.path.display());
        }
    }

    println!("\n📚 Dependencies:");
    if config.dependencies.is_empty() {
        println!("  (none)");
    } else {
        for dep in &config.dependencies {
            let fingerprint = dep.fingerprint.as_deref().unwrap_or("*");
            println!(
                "  {} ({}:{}) @ {}",
                dep.alias, dep.actr_type.manufacturer, dep.actr_type.name, fingerprint
            );
        }
    }

    println!("\n📜 Scripts:");
    if config.scripts.is_empty() {
        println!("  (none)");
    } else {
        for (name, command) in &config.scripts {
            println!("  {name}: {command}");
        }
    }

    println!("\n🌐 System Configuration:");
    println!("  Signaling URL: {}", config.signaling_url);
    println!("  Realm ID: {}", config.realm.realm_id);
    println!("  Visible in discovery: {}", config.visible_in_discovery);

    println!("\n✅ Configuration successfully parsed and validated!");

    Ok(())
}
