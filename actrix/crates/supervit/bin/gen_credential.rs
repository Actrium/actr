//! Generate test credentials for SupervisedService
//!
//! This tool generates valid NonceCredential for testing the Supervisord gRPC service.
//!
//! # Usage
//!
//! Generate credential with human-readable output:
//! ```bash
//! cargo run --bin gen_credential -- --action node_info
//! cargo run --bin gen_credential -- --action get_realm --subject test-realm
//! ```
//!
//! Generate credential as JSON (for scripts):
//! ```bash
//! cargo run --bin gen_credential -- --action node_info --output json
//! ```
//!
//! Generate credential with custom node_id and shared_secret:
//! ```bash
//! cargo run --bin gen_credential -- \
//!   --action create_realm \
//!   --subject test-realm-01 \
//!   --node-id test-node-01 \
//!   --shared-secret 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
//! ```

use clap::Parser;
use nonce_auth::CredentialBuilder;

/// Default shared secret for testing (must match supervisord_server example)
const DEFAULT_SHARED_SECRET: &str =
    "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// Default node ID (must match supervisord_server example)
const DEFAULT_NODE_ID: &str = "example-node-01";

#[derive(Debug, Clone, Copy, PartialEq, clap::ValueEnum)]
enum OutputFormat {
    Human,
    Json,
}

fn parse_action(s: &str) -> Result<String, String> {
    let valid_actions = [
        "node_info",
        "get_realm",
        "create_realm",
        "update_realm",
        "delete_realm",
        "list_realms",
        "shutdown",
        "get_config",
        "update_config",
    ];
    if valid_actions.contains(&s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "Invalid action: {}. Valid actions: {}",
            s,
            valid_actions.join(", ")
        ))
    }
}

fn action_requires_subject(action: &str) -> bool {
    matches!(
        action,
        "get_realm"
            | "create_realm"
            | "update_realm"
            | "delete_realm"
            | "get_config"
            | "update_config"
    )
}

/// Generate test credentials for SupervisedService gRPC API
#[derive(Parser)]
#[command(name = "gen_credential")]
#[command(about = "Generate NonceCredential for testing Supervisord gRPC service")]
#[command(version)]
struct Args {
    /// Action to generate credential for
    #[arg(short, long, value_parser = parse_action, default_value = "node_info")]
    action: String,

    /// Subject (realm_id for realm operations, type:key for config operations)
    #[arg(short, long)]
    subject: Option<String>,

    /// Node ID
    #[arg(long, default_value = DEFAULT_NODE_ID)]
    node_id: String,

    /// Shared secret (hex encoded)
    #[arg(long, default_value = DEFAULT_SHARED_SECRET)]
    shared_secret: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "human")]
    output: OutputFormat,
}

fn main() {
    let args = Args::parse();

    // Validate subject requirement
    if action_requires_subject(&args.action) && args.subject.is_none() {
        eprintln!("Error: --subject is required for action '{}'", args.action);
        eprintln!("  For realm operations: --subject <realm-id>");
        eprintln!("  For config operations: --subject <type:key>");
        std::process::exit(1);
    }

    // Build payload based on action
    let payload = if action_requires_subject(&args.action) {
        let subject = args.subject.expect("subject required");
        format!("{}:{}:{}", args.action, args.node_id, subject)
    } else {
        format!("{}:{}", args.action, args.node_id)
    };

    // Decode shared secret
    let shared_secret = match hex::decode(&args.shared_secret) {
        Ok(secret) => secret,
        Err(e) => {
            eprintln!("Error: Invalid hex secret: {}", e);
            std::process::exit(1);
        }
    };

    // Generate credential
    let credential = match CredentialBuilder::new(&shared_secret).sign(payload.as_bytes()) {
        Ok(cred) => cred,
        Err(e) => {
            eprintln!("Error: Failed to generate credential: {}", e);
            std::process::exit(1);
        }
    };

    let timestamp = credential.timestamp;
    let nonce = &credential.nonce;
    let signature = &credential.signature;

    // Output based on format
    match args.output {
        OutputFormat::Json => {
            // Pure JSON output for script consumption
            println!(
                r#"{{"timestamp": {},"nonce": "{}","signature": "{}"}}"#,
                timestamp, nonce, signature
            );
        }
        OutputFormat::Human => {
            // Human-readable output with context
            println!("Action: {} (payload: {})", args.action, payload);
            println!();
            println!("Credential:");
            println!(
                r#"{{
  "timestamp": {},
  "nonce": "{}",
  "signature": "{}"
}}"#,
                timestamp, nonce, signature
            );
            println!();
            println!("Test with grpcurl script:");
            println!(
                "  ./crates/supervit/scripts/test_supervised.sh {}",
                args.action
            );
            println!();
            println!("Note: Credentials expire after ~5 minutes (max_clock_skew_secs).");
        }
    }
}
