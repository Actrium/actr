//! Rust 代码生成器

use crate::{
    GeneratedFile, ProtoField, ProtoMessage, ProtoMethod, ProtoService, config::WebCodegenConfig,
    error::Result,
};
use std::path::Path;

/// 解析 proto 文件
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

/// 解析 proto 文件内容
fn parse_proto_content(content: &str, path: &Path) -> Result<ProtoService> {
    use crate::error::CodegenError;

    // 提取 package 名称
    let package = extract_package(content);

    // 提取 service 名称
    let service_name = extract_service_name(content)
        .ok_or_else(|| CodegenError::InvalidProtoFile(path.to_path_buf()))?;

    // 提取方法
    let methods = extract_methods(content, &service_name);

    // 提取消息类型
    let messages = extract_messages(content);

    Ok(ProtoService {
        name: service_name,
        package,
        methods,
        messages,
    })
}

/// 提取 package 名称
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

/// 提取 service 名称
fn extract_service_name(content: &str) -> Option<String> {
    content
        .lines()
        .find(|line| line.trim().starts_with("service"))
        .and_then(|line| line.split_whitespace().nth(1).map(String::from))
}

/// 提取 service 中的方法
fn extract_methods(content: &str, service_name: &str) -> Vec<ProtoMethod> {
    let mut methods = Vec::new();
    let mut in_service = false;
    let mut brace_count = 0;
    let mut service_started = false;

    for line in content.lines() {
        let trimmed = line.trim();

        // 检测 service 开始
        if trimmed.starts_with("service") && trimmed.contains(service_name) {
            in_service = true;
        }

        if !in_service {
            continue;
        }

        // 追踪大括号
        brace_count += trimmed.matches('{').count() as i32;
        brace_count -= trimmed.matches('}').count() as i32;

        // 标记 service 真正开始（遇到第一个 {）
        if brace_count > 0 {
            service_started = true;
        }

        // service 结束（遇到匹配的 }）
        if service_started && brace_count == 0 {
            break;
        }

        // 解析 rpc 方法
        if trimmed.starts_with("rpc") {
            if let Some(method) = parse_rpc_method(trimmed) {
                methods.push(method);
            }
        }
    }

    methods
}

/// 解析单个 rpc 方法定义
fn parse_rpc_method(line: &str) -> Option<ProtoMethod> {
    // rpc MethodName(RequestType) returns (ResponseType);
    // rpc StreamMethod(stream RequestType) returns (stream ResponseType);

    // 检查是否包含必要的关键字
    if !line.contains("returns") {
        return None;
    }

    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        // 至少需要: rpc, MethodName(...), returns, (...)
        return None;
    }

    // 提取方法名（去掉括号及其后的内容）
    let name = parts[1].split('(').next().unwrap_or("").to_string();

    // 提取输入类型
    let input_start = line.find('(')? + 1;
    let input_end = line.find(')')?;
    let input_part = line[input_start..input_end].trim();
    let (input_type, input_streaming) = if input_part.starts_with("stream") {
        (input_part.strip_prefix("stream")?.trim().to_string(), true)
    } else {
        (input_part.to_string(), false)
    };

    // 提取输出类型
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

/// 提取消息类型定义
fn extract_messages(content: &str) -> Vec<ProtoMessage> {
    let mut messages = Vec::new();
    let mut current_message: Option<(String, Vec<ProtoField>)> = None;
    let mut brace_count = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // 检测 message 开始
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
            // 追踪大括号
            brace_count += trimmed.matches('{').count() as i32;
            brace_count -= trimmed.matches('}').count() as i32;

            // message 结束
            if brace_count == 0 {
                messages.push(ProtoMessage {
                    name: msg_name.clone(),
                    fields: fields.clone(),
                });
                current_message = None;
                continue;
            }

            // 解析字段
            if let Some(field) = parse_message_field(trimmed) {
                fields.push(field);
            }
        }
    }

    messages
}

/// 解析消息字段
fn parse_message_field(line: &str) -> Option<ProtoField> {
    // repeated string items = 1;
    // optional int32 count = 2;
    // string name = 3;

    // 跳过空行、注释和 proto option 配置
    if line.is_empty() || line.starts_with("//") {
        return None;
    }

    // 跳过 proto option 配置（但不跳过 optional 字段）
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

    // 提取字段编号（格式: name = N;）
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

/// 生成 Rust Actor 代码
pub fn generate_rust_actors(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();

    for service in services {
        let file = generate_rust_actor_for_service(config, service)?;
        files.push(file);
    }

    // 生成 mod.rs
    let mod_file = generate_rust_mod_file(config, services)?;
    files.push(mod_file);

    Ok(files)
}

/// 为单个服务生成 Rust Actor 代码
fn generate_rust_actor_for_service(
    config: &WebCodegenConfig,
    service: &ProtoService,
) -> Result<GeneratedFile> {
    use heck::ToSnakeCase;

    let file_name = format!("{}.rs", service.name.to_snake_case());
    let file_path = config.rust_output_dir.join(&file_name);

    let mut content = format!(
        r#"//! 自动生成的 Actor 代码
//! 服务: {}
//! 包: {}
//!
//! ⚠️  请勿手动编辑此文件

use wasm_bindgen::prelude::*;
use serde::{{Serialize, Deserialize}};

"#,
        service.name, service.package
    );

    // 生成消息类型定义
    for message in &service.messages {
        content.push_str(&generate_rust_message(message));
        content.push('\n');
    }

    // 生成 Actor 结构
    content.push_str(&format!(
        r#"/// {} Actor
#[wasm_bindgen]
pub struct {}Actor {{
    // Actor 状态
}}

#[wasm_bindgen]
impl {}Actor {{
    /// 创建新的 Actor 实例
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {{
        Self {{}}
    }}

"#,
        service.name, service.name, service.name
    ));

    // 生成方法
    for method in &service.methods {
        content.push_str(&generate_rust_method(method));
        content.push('\n');
    }

    content.push_str("}\n");

    Ok(GeneratedFile::new(file_path, content))
}

/// 生成 Rust 消息类型
fn generate_rust_message(message: &ProtoMessage) -> String {
    let mut content = format!(
        r#"/// {} 消息
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

/// 生成 Rust 方法
fn generate_rust_method(method: &ProtoMethod) -> String {
    use heck::ToSnakeCase;

    let method_name = method.name.to_snake_case();
    let input_type = &method.input_type;
    let output_type = &method.output_type;

    if method.is_streaming {
        // 流式方法
        format!(
            r#"    /// {} 方法（流式）
    pub async fn {}(&self, request: {}) -> Result<JsValue, JsValue> {{
        // TODO: 实现流式方法
        todo!("实现流式方法: {}")
    }}
"#,
            method.name, method_name, input_type, method.name
        )
    } else {
        // 普通 RPC 方法
        format!(
            r#"    /// {} 方法
    pub async fn {}(&self, request: {}) -> Result<{}, JsValue> {{
        // TODO: 实现方法逻辑
        todo!("实现方法: {}")
    }}
"#,
            method.name, method_name, input_type, output_type, method.name
        )
    }
}

/// 将 Proto 类型转换为 Rust 类型
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
        // 自定义类型保持原样
        custom => custom.to_string(),
    }
}

/// 生成 Rust mod.rs
fn generate_rust_mod_file(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<GeneratedFile> {
    use heck::ToSnakeCase;

    let file_path = config.rust_output_dir.join("mod.rs");

    let mut content = String::from(
        r#"//! 自动生成的模块
//!
//! ⚠️  请勿手动编辑此文件

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
#[cfg(test)]
#[path = "generator_test.rs"]
mod generator_test;
