//! Rust code generator

use crate::{
    GeneratedFile, ProtoField, ProtoMessage, ProtoMethod, ProtoService, config::WebCodegenConfig,
    error::Result,
};
use std::path::Path;

/// Parse proto files
pub fn parse_proto_files(config: &WebCodegenConfig) -> Result<Vec<ProtoService>> {
    use std::fs;

    let mut services = Vec::new();

    for proto_file in &config.proto_files {
        let content = fs::read_to_string(proto_file)?;
        let service = parse_proto_content(&content, proto_file)?;
        services.push(service);
    }

    Ok(services)
}

/// Parse proto file content
fn parse_proto_content(content: &str, path: &Path) -> Result<ProtoService> {
    use crate::error::CodegenError;

    // Extract package name
    let package = extract_package(content);

    // Extract service name
    let service_name = extract_service_name(content)
        .ok_or_else(|| CodegenError::InvalidProtoFile(path.to_path_buf()))?;

    // Extract methods
    let methods = extract_methods(content, &service_name);

    // Extract message types
    let messages = extract_messages(content);

    Ok(ProtoService {
        name: service_name,
        package,
        methods,
        messages,
    })
}

/// Extract package name
fn extract_package(content: &str) -> String {
    content
        .lines()
        .find(|line| line.trim().starts_with("package"))
        .and_then(|line| {
            line.split_whitespace()
                .nth(1)
                .map(|s| s.trim_end_matches(';').to_string())
        })
        .unwrap_or_else(|| "default".to_string())
}

/// Extract service name
fn extract_service_name(content: &str) -> Option<String> {
    content
        .lines()
        .find(|line| line.trim().starts_with("service"))
        .and_then(|line| line.split_whitespace().nth(1).map(String::from))
}

/// Extract methods from a service
fn extract_methods(content: &str, service_name: &str) -> Vec<ProtoMethod> {
    let mut methods = Vec::new();
    let mut in_service = false;
    let mut brace_count = 0;
    let mut service_started = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect service start
        if trimmed.starts_with("service") && trimmed.contains(service_name) {
            in_service = true;
        }

        if !in_service {
            continue;
        }

        // Track braces
        brace_count += trimmed.matches('{').count() as i32;
        brace_count -= trimmed.matches('}').count() as i32;

        // Mark service actually started (encountered first {)
        if brace_count > 0 {
            service_started = true;
        }

        // Service ended (encountered matching })
        if service_started && brace_count == 0 {
            break;
        }

        // Parse rpc method
        if trimmed.starts_with("rpc") {
            if let Some(method) = parse_rpc_method(trimmed) {
                methods.push(method);
            }
        }
    }

    methods
}

/// Parse a single rpc method definition
fn parse_rpc_method(line: &str) -> Option<ProtoMethod> {
    // rpc MethodName(RequestType) returns (ResponseType);
    // rpc StreamMethod(stream RequestType) returns (stream ResponseType);

    // Check for required keywords
    if !line.contains("returns") {
        return None;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        // Need at least: rpc, MethodName(...), returns, (...)
        return None;
    }

    // Extract method name (strip parentheses and everything after)
    let name = parts[1].split('(').next().unwrap_or("").to_string();

    // Extract input type
    let input_start = line.find('(')? + 1;
    let input_end = line.find(')')?;
    let input_part = line[input_start..input_end].trim();
    let (input_type, input_streaming) = if input_part.starts_with("stream") {
        (input_part.strip_prefix("stream")?.trim().to_string(), true)
    } else {
        (input_part.to_string(), false)
    };

    // Extract output type
    let output_start = line.rfind('(')? + 1;
    let output_end = line.rfind(')')?;
    let output_part = line[output_start..output_end].trim();
    let (output_type, output_streaming) = if output_part.starts_with("stream") {
        (output_part.strip_prefix("stream")?.trim().to_string(), true)
    } else {
        (output_part.to_string(), false)
    };

    Some(ProtoMethod {
        name,
        input_type,
        output_type,
        is_streaming: input_streaming || output_streaming,
    })
}

/// Extract message type definitions
fn extract_messages(content: &str) -> Vec<ProtoMessage> {
    let mut messages = Vec::new();
    let mut current_message: Option<(String, Vec<ProtoField>)> = None;
    let mut brace_count = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect message start
        if trimmed.starts_with("message") {
            let name = trimmed
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.strip_suffix('{').or(Some(s)))
                .map(String::from);

            if let Some(name) = name {
                current_message = Some((name, Vec::new()));
                brace_count = trimmed.matches('{').count() as i32;
            }
            continue;
        }

        if let Some((msg_name, fields)) = &mut current_message {
            // Track braces
            brace_count += trimmed.matches('{').count() as i32;
            brace_count -= trimmed.matches('}').count() as i32;

            // Message ended
            if brace_count == 0 {
                messages.push(ProtoMessage {
                    name: msg_name.clone(),
                    fields: fields.clone(),
                });
                current_message = None;
                continue;
            }

            // Parse field
            if let Some(field) = parse_message_field(trimmed) {
                fields.push(field);
            }
        }
    }

    messages
}

/// Parse a message field
fn parse_message_field(line: &str) -> Option<ProtoField> {
    // repeated string items = 1;
    // optional int32 count = 2;
    // string name = 3;

    // Skip empty lines, comments, and proto option directives
    if line.is_empty() || line.starts_with("//") {
        return None;
    }

    // Skip proto option directives (but not optional fields)
    if line.starts_with("option ") {
        return None;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }

    let (is_repeated, is_optional, type_idx, name_idx) = if parts[0] == "repeated" {
        (true, false, 1, 2)
    } else if parts[0] == "optional" {
        (false, true, 1, 2)
    } else {
        (false, false, 0, 1)
    };

    let field_type = parts[type_idx].to_string();
    let name = parts[name_idx].trim_end_matches('=').to_string();

    // Extract field number (format: name = N;)
    let number = line
        .split('=')
        .nth(1)
        .and_then(|s| s.trim().trim_end_matches(';').trim().parse::<u32>().ok())
        .unwrap_or(0);

    Some(ProtoField {
        name,
        field_type,
        number,
        is_repeated,
        is_optional,
    })
}

/// Generate Rust Actor code
pub fn generate_rust_actors(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();

    for service in services {
        let file = generate_rust_actor_for_service(config, service)?;
        files.push(file);
    }

    // Generate mod.rs
    let mod_file = generate_rust_mod_file(config, services)?;
    files.push(mod_file);

    Ok(files)
}

/// Generate Rust Actor code for a single service
fn generate_rust_actor_for_service(
    config: &WebCodegenConfig,
    service: &ProtoService,
) -> Result<GeneratedFile> {
    use heck::ToSnakeCase;

    let file_name = format!("{}.rs", service.name.to_snake_case());
    let file_path = config.rust_output_dir.join(&file_name);

    let mut content = format!(
        r#"//! Auto-generated Actor code
//! Service: {}
//! Package: {}
//!
//! DO NOT EDIT this file manually

use wasm_bindgen::prelude::*;
use serde::{{Serialize, Deserialize}};

"#,
        service.name, service.package
    );

    // Generate message type definitions
    for message in &service.messages {
        content.push_str(&generate_rust_message(message));
        content.push('\n');
    }

    // Generate Actor struct
    content.push_str(&format!(
        r#"/// {} Actor
#[wasm_bindgen]
pub struct {}Actor {{
    // Actor state
}}

#[wasm_bindgen]
impl {}Actor {{
    /// Create a new Actor instance
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {{
        Self {{}}
    }}

"#,
        service.name, service.name, service.name
    ));

    // Generate methods
    for method in &service.methods {
        content.push_str(&generate_rust_method(method));
        content.push('\n');
    }

    content.push_str("}\n");

    Ok(GeneratedFile::new(file_path, content))
}

/// Generate Rust message type
fn generate_rust_message(message: &ProtoMessage) -> String {
    let mut content = format!(
        r#"/// {} message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[wasm_bindgen]
pub struct {} {{
"#,
        message.name, message.name
    );

    for field in &message.fields {
        let rust_type = proto_type_to_rust(&field.field_type);
        let field_type = if field.is_repeated {
            format!("Vec<{}>", rust_type)
        } else if field.is_optional {
            format!("Option<{}>", rust_type)
        } else {
            rust_type
        };

        content.push_str(&format!("    pub {}: {},\n", field.name, field_type));
    }

    content.push_str("}\n");
    content
}

/// Generate Rust method
fn generate_rust_method(method: &ProtoMethod) -> String {
    use heck::ToSnakeCase;

    let method_name = method.name.to_snake_case();
    let input_type = &method.input_type;
    let output_type = &method.output_type;

    if method.is_streaming {
        // Streaming method
        format!(
            r#"    /// {} method (streaming)
    pub async fn {}(&self, request: {}) -> Result<JsValue, JsValue> {{
        // TODO: implement streaming method
        todo!("implement streaming method: {}")
    }}
"#,
            method.name, method_name, input_type, method.name
        )
    } else {
        // Regular RPC method
        format!(
            r#"    /// {} method
    pub async fn {}(&self, request: {}) -> Result<{}, JsValue> {{
        // TODO: implement method logic
        todo!("implement method: {}")
    }}
"#,
            method.name, method_name, input_type, output_type, method.name
        )
    }
}

/// Convert Proto type to Rust type
fn proto_type_to_rust(proto_type: &str) -> String {
    match proto_type {
        "string" => "String".to_string(),
        "bytes" => "Vec<u8>".to_string(),
        "int32" | "sint32" | "sfixed32" => "i32".to_string(),
        "int64" | "sint64" | "sfixed64" => "i64".to_string(),
        "uint32" | "fixed32" => "u32".to_string(),
        "uint64" | "fixed64" => "u64".to_string(),
        "bool" => "bool".to_string(),
        "float" => "f32".to_string(),
        "double" => "f64".to_string(),
        // Custom types remain as-is
        custom => custom.to_string(),
    }
}

/// Generate Rust mod.rs
fn generate_rust_mod_file(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<GeneratedFile> {
    use heck::ToSnakeCase;

    let file_path = config.rust_output_dir.join("mod.rs");

    let mut content = String::from(
        r#"//! Auto-generated module
//!
//! DO NOT EDIT this file manually

"#,
    );

    for service in services {
        let module_name = service.name.to_snake_case();
        content.push_str(&format!("pub mod {};\n", module_name));
    }

    content.push('\n');

    for service in services {
        let module_name = service.name.to_snake_case();
        content.push_str(&format!(
            "pub use {}::{}Actor;\n",
            module_name, service.name
        ));
    }

    Ok(GeneratedFile::new(file_path, content))
}
