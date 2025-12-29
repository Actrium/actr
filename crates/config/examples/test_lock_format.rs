//! Example showing lock file format with embedded proto content

use actr_config::lock::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a lock file with embedded proto content
    let mut lock_file = LockFile::new();

    let spec_meta = ServiceSpecMeta {
        description: Some("User management service".to_string()),
        fingerprint: "service_semantic:a1b2c3d4e5f6".to_string(),
        protobufs: vec![
            ProtoFileWithContent {
                name: "user.v1".to_string(),
                fingerprint: "semantic:xyz123".to_string(),
                content: r#"syntax = "proto3";

package user.v1;

message User {
  uint64 id = 1;
  string name = 2;
  string email = 3;
}

service UserService {
  rpc GetUser(GetUserRequest) returns (GetUserResponse);
  rpc CreateUser(CreateUserRequest) returns (CreateUserResponse);
}
"#
                .to_string(),
            },
            ProtoFileWithContent {
                name: "common.v1".to_string(),
                fingerprint: "semantic:abc789".to_string(),
                content: "syntax = \"proto3\";\n\npackage common.v1;\n\nmessage Empty {}"
                    .to_string(),
            },
        ],
        published_at: Some(1705315800),
        tags: vec!["latest".to_string(), "stable".to_string()],
    };

    let dep = LockedDependency::new(
        "user-service".to_string(),
        "acme+user-service".to_string(),
        spec_meta,
    );

    lock_file.add_dependency(dep);

    // Serialize to TOML
    println!("=== Generated actr.lock.toml ===\n");
    let toml_str = toml::to_string_pretty(&lock_file)?;
    println!("{toml_str}");

    // Verify round-trip
    println!("\n=== Verifying round-trip ===");
    let restored: LockFile = toml::from_str(&toml_str)?;
    println!("✓ Successfully parsed back");
    println!("✓ Dependencies: {}", restored.dependencies.len());
    println!(
        "✓ Proto packages: {}",
        restored.dependencies[0].spec.protobufs.len()
    );

    // Check content
    let user_proto = &restored.dependencies[0].spec.protobufs[0];
    println!("✓ First package name: {}", user_proto.name);
    println!("✓ Content length: {} bytes", user_proto.content.len());
    println!(
        "✓ Content preserved: {}",
        user_proto.content.contains("message User")
    );

    Ok(())
}
