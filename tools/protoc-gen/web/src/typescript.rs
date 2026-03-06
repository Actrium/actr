//! TypeScript 代码生成器

use crate::{GeneratedFile, ProtoService, config::WebCodegenConfig, error::Result};

/// 生成 TypeScript 类型定义
pub fn generate_types(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();

    for service in services {
        let file = generate_types_for_service(config, service)?;
        files.push(file);
    }

    // 生成 index.ts
    let index_file = generate_ts_index_file(config, services)?;
    files.push(index_file);

    Ok(files)
}

/// 为单个服务生成 TypeScript 类型
fn generate_types_for_service(
    config: &WebCodegenConfig,
    service: &ProtoService,
) -> Result<GeneratedFile> {
    use heck::ToKebabCase;

    let file_name = format!("{}.types.ts", service.name.to_kebab_case());
    let file_path = config.ts_output_dir.join(&file_name);

    let mut content = format!(
        r#"/**
 * 自动生成的类型定义
 * 服务: {}
 * 包: {}
 *
 * ⚠️  请勿手动编辑此文件
 */

"#,
        service.name, service.package
    );

    // 添加 protobuf 工具函数
    content.push_str(generate_ts_protobuf_utils());

    // 生成消息类型和编解码函数
    for message in &service.messages {
        content.push_str(&generate_ts_message_type(message));
        content.push('\n');
        content.push_str(&generate_ts_encode_function(message));
        content.push('\n');
        content.push_str(&generate_ts_decode_function(message));
        content.push('\n');
    }

    Ok(GeneratedFile::new(file_path, content))
}

/// 生成 TypeScript 消息类型
fn generate_ts_message_type(message: &crate::ProtoMessage) -> String {
    let mut content = format!(
        r#"/**
 * {} 消息
 */
export interface {} {{
"#,
        message.name, message.name
    );

    for field in &message.fields {
        let ts_type = proto_type_to_typescript(&field.field_type);
        let field_type = if field.is_repeated {
            format!("{}[]", ts_type)
        } else {
            ts_type
        };

        let optional_marker = if field.is_optional { "?" } else { "" };

        content.push_str(&format!(
            "  {}{}: {};\n",
            field.name, optional_marker, field_type
        ));
    }

    content.push_str("}\n");
    content
}

/// 生成编码函数
fn generate_ts_encode_function(message: &crate::ProtoMessage) -> String {
    let mut content = format!(
        r#"/**
 * 将 {} 编码为 Uint8Array (Protobuf wire format)
 */
export function encode{}(msg: {}): Uint8Array {{
  const parts: number[] = [];
"#,
        message.name, message.name, message.name
    );

    for field in &message.fields {
        let field_number = field.number;
        let wire_type = get_wire_type(&field.field_type);

        match field.field_type.as_str() {
            "string" => {
                content.push_str(&format!(
                    r#"
  // Field {}: {} (string)
  if (msg.{} !== undefined && msg.{} !== '') {{
    const text = new TextEncoder().encode(msg.{});
    parts.push({} << 3 | 2); // field tag
    pushVarint(parts, text.length);
    parts.push(...Array.from(text));
  }}
"#,
                    field_number, field.name, field.name, field.name, field.name, field_number
                ));
            }
            "bytes" => {
                content.push_str(&format!(
                    r#"
  // Field {}: {} (bytes)
  if (msg.{} !== undefined && msg.{}.length > 0) {{
    parts.push({} << 3 | 2); // field tag
    pushVarint(parts, msg.{}.length);
    parts.push(...Array.from(msg.{}));
  }}
"#,
                    field_number,
                    field.name,
                    field.name,
                    field.name,
                    field_number,
                    field.name,
                    field.name
                ));
            }
            "bool" => {
                content.push_str(&format!(
                    r#"
  // Field {}: {} (bool)
  if (msg.{} !== undefined) {{
    parts.push({} << 3 | 0); // field tag
    parts.push(msg.{} ? 1 : 0);
  }}
"#,
                    field_number, field.name, field.name, field_number, field.name
                ));
            }
            "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" => {
                content.push_str(&format!(
                    r#"
  // Field {}: {} ({})
  if (msg.{} !== undefined && msg.{} !== 0) {{
    parts.push({} << 3 | 0); // field tag
    pushVarint(parts, msg.{});
  }}
"#,
                    field_number,
                    field.name,
                    field.field_type,
                    field.name,
                    field.name,
                    field_number,
                    field.name
                ));
            }
            "float" | "double" => {
                let byte_size = if field.field_type == "float" { 4 } else { 8 };
                let wire = if field.field_type == "float" { 5 } else { 1 };
                content.push_str(&format!(
                    r#"
  // Field {}: {} ({})
  if (msg.{} !== undefined && msg.{} !== 0) {{
    parts.push({} << 3 | {}); // field tag
    const buf = new ArrayBuffer({});
    const view = new DataView(buf);
    view.set{}(0, msg.{}, true);
    parts.push(...Array.from(new Uint8Array(buf)));
  }}
"#,
                    field_number,
                    field.name,
                    field.field_type,
                    field.name,
                    field.name,
                    field_number,
                    wire,
                    byte_size,
                    if field.field_type == "float" {
                        "Float32"
                    } else {
                        "Float64"
                    },
                    field.name
                ));
            }
            "fixed32" | "sfixed32" => {
                content.push_str(&format!(
                    r#"
  // Field {}: {} ({})
  if (msg.{} !== undefined && msg.{} !== 0) {{
    parts.push({} << 3 | 5); // field tag (wire type 5 = 32-bit)
    const buf = new ArrayBuffer(4);
    const view = new DataView(buf);
    view.set{}(0, msg.{}, true);
    parts.push(...Array.from(new Uint8Array(buf)));
  }}
"#,
                    field_number,
                    field.name,
                    field.field_type,
                    field.name,
                    field.name,
                    field_number,
                    if field.field_type == "sfixed32" {
                        "Int32"
                    } else {
                        "Uint32"
                    },
                    field.name
                ));
            }
            "fixed64" | "sfixed64" => {
                content.push_str(&format!(
                    r#"
  // Field {}: {} ({})
  if (msg.{} !== undefined && msg.{} !== 0) {{
    parts.push({} << 3 | 1); // field tag (wire type 1 = 64-bit)
    const buf = new ArrayBuffer(8);
    const view = new DataView(buf);
    view.setBigInt64(0, BigInt(msg.{}), true);
    parts.push(...Array.from(new Uint8Array(buf)));
  }}
"#,
                    field_number,
                    field.name,
                    field.field_type,
                    field.name,
                    field.name,
                    field_number,
                    field.name
                ));
            }
            _ => {
                // 默认作为 varint 处理
                content.push_str(&format!(
                    r#"
  // Field {}: {} (default varint)
  if (msg.{} !== undefined) {{
    parts.push({} << 3 | {}); // field tag
    pushVarint(parts, msg.{});
  }}
"#,
                    field_number, field.name, field.name, field_number, wire_type, field.name
                ));
            }
        }
    }

    content.push_str(
        r#"
  return new Uint8Array(parts);
}
"#,
    );

    content
}

/// 生成解码函数
fn generate_ts_decode_function(message: &crate::ProtoMessage) -> String {
    let mut content = format!(
        r#"/**
 * 从 Uint8Array (Protobuf wire format) 解码 {}
 */
export function decode{}(bytes: Uint8Array): {} {{
  const result: {} = {{{}}};
  let offset = 0;

  while (offset < bytes.length) {{
    const tagInfo = readVarint(bytes, offset);
    const tag = tagInfo.value;
    offset = tagInfo.offset;

    const fieldNumber = tag >> 3;
    const wireType = tag & 0x7;

    switch (fieldNumber) {{
"#,
        message.name,
        message.name,
        message.name,
        message.name,
        // 生成默认值
        message
            .fields
            .iter()
            .map(|f| format!(
                "{}: {}",
                f.name,
                get_default_value(&f.field_type, f.is_repeated)
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );

    for field in &message.fields {
        let field_number = field.number;

        match field.field_type.as_str() {
            "string" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 2) throw new Error('Expected wire type 2 for string');
        const lenInfo = readVarint(bytes, offset);
        offset = lenInfo.offset;
        const data = bytes.slice(offset, offset + lenInfo.value);
        result.{} = new TextDecoder().decode(data);
        offset += lenInfo.value;
        break;
      }}
"#,
                    field_number, field.name, field.name
                ));
            }
            "bytes" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 2) throw new Error('Expected wire type 2 for bytes');
        const lenInfo = readVarint(bytes, offset);
        offset = lenInfo.offset;
        result.{} = bytes.slice(offset, offset + lenInfo.value);
        offset += lenInfo.value;
        break;
      }}
"#,
                    field_number, field.name, field.name
                ));
            }
            "bool" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 0) throw new Error('Expected wire type 0 for bool');
        const valInfo = readVarint(bytes, offset);
        result.{} = valInfo.value !== 0;
        offset = valInfo.offset;
        break;
      }}
"#,
                    field_number, field.name, field.name
                ));
            }
            "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 0) throw new Error('Expected wire type 0 for {}');
        const valInfo = readVarint(bytes, offset);
        result.{} = valInfo.value;
        offset = valInfo.offset;
        break;
      }}
"#,
                    field_number, field.name, field.field_type, field.name
                ));
            }
            "float" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 5) throw new Error('Expected wire type 5 for float');
        const view = new DataView(bytes.buffer, bytes.byteOffset + offset, 4);
        result.{} = view.getFloat32(0, true);
        offset += 4;
        break;
      }}
"#,
                    field_number, field.name, field.name
                ));
            }
            "double" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 1) throw new Error('Expected wire type 1 for double');
        const view = new DataView(bytes.buffer, bytes.byteOffset + offset, 8);
        result.{} = view.getFloat64(0, true);
        offset += 8;
        break;
      }}
"#,
                    field_number, field.name, field.name
                ));
            }
            "fixed32" | "sfixed32" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 5) throw new Error('Expected wire type 5 for {}');
        const view = new DataView(bytes.buffer, bytes.byteOffset + offset, 4);
        result.{} = view.get{}(0, true);
        offset += 4;
        break;
      }}
"#,
                    field_number,
                    field.name,
                    field.field_type,
                    field.name,
                    if field.field_type == "sfixed32" {
                        "Int32"
                    } else {
                        "Uint32"
                    }
                ));
            }
            "fixed64" | "sfixed64" => {
                content.push_str(&format!(
                    r#"      case {}: {{ // {}
        if (wireType !== 1) throw new Error('Expected wire type 1 for {}');
        const view = new DataView(bytes.buffer, bytes.byteOffset + offset, 8);
        result.{} = Number(view.getBigInt64(0, true));
        offset += 8;
        break;
      }}
"#,
                    field_number, field.name, field.field_type, field.name
                ));
            }
            _ => {
                // 默认跳过未知字段
                content.push_str(&format!(
                    r#"      case {}: {{ // {} (unknown type: {})
        offset = skipField(bytes, offset, wireType);
        break;
      }}
"#,
                    field_number, field.name, field.field_type
                ));
            }
        }
    }

    content.push_str(
        r#"      default:
        // Skip unknown field
        offset = skipField(bytes, offset, wireType);
    }
  }

  return result;
}
"#,
    );

    content
}

/// 将 Proto 类型转换为 TypeScript 类型
fn proto_type_to_typescript(proto_type: &str) -> String {
    match proto_type {
        "string" => "string".to_string(),
        "bytes" => "Uint8Array".to_string(),
        "int32" | "sint32" | "sfixed32" | "int64" | "sint64" | "sfixed64" | "uint32"
        | "fixed32" | "uint64" | "fixed64" | "float" | "double" => "number".to_string(),
        "bool" => "boolean".to_string(),
        // 自定义类型保持原样
        custom => custom.to_string(),
    }
}

/// 获取 protobuf wire type
fn get_wire_type(proto_type: &str) -> u8 {
    match proto_type {
        "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" | "bool" => 0, // Varint
        "fixed64" | "sfixed64" | "double" => 1,                                      // 64-bit
        "string" | "bytes" => 2,               // Length-delimited
        "fixed32" | "sfixed32" | "float" => 5, // 32-bit
        _ => 0,                                // Default to varint
    }
}

/// 获取字段的默认值
fn get_default_value(proto_type: &str, is_repeated: bool) -> &'static str {
    if is_repeated {
        return "[]";
    }
    match proto_type {
        "string" => "''",
        "bytes" => "new Uint8Array()",
        "bool" => "false",
        "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" | "float" | "double"
        | "fixed32" | "sfixed32" | "fixed64" | "sfixed64" => "0",
        _ => "undefined as any",
    }
}

/// 生成 TypeScript 工具函数（用于 protobuf 编解码）
fn generate_ts_protobuf_utils() -> &'static str {
    r#"// ========== Protobuf 编解码工具函数 ==========

/**
 * 将 varint 写入 number 数组
 */
function pushVarint(arr: number[], value: number): void {
  value = value >>> 0; // 转为无符号整数
  while (value > 127) {
    arr.push((value & 0x7f) | 0x80);
    value = value >>> 7;
  }
  arr.push(value);
}

/**
 * 从字节数组读取 varint
 */
function readVarint(bytes: Uint8Array, offset: number): { value: number; offset: number } {
  let result = 0;
  let shift = 0;
  let byte: number;
  do {
    byte = bytes[offset++];
    result |= (byte & 0x7f) << shift;
    shift += 7;
  } while (byte >= 0x80);
  return { value: result >>> 0, offset };
}

/**
 * 跳过未知字段
 */
function skipField(bytes: Uint8Array, offset: number, wireType: number): number {
  switch (wireType) {
    case 0: // Varint
      while (bytes[offset++] >= 0x80) {}
      return offset;
    case 1: // 64-bit
      return offset + 8;
    case 2: // Length-delimited
      const lenInfo = readVarint(bytes, offset);
      return lenInfo.offset + lenInfo.value;
    case 5: // 32-bit
      return offset + 4;
    default:
      throw new Error(`Unknown wire type: ${wireType}`);
  }
}

"#
}

/// 生成 ActorRef 包装类
pub fn generate_actor_refs(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();

    for service in services {
        let file = generate_actor_ref_for_service(config, service)?;
        files.push(file);
    }

    Ok(files)
}

/// 为单个服务生成 ActorRef
fn generate_actor_ref_for_service(
    config: &WebCodegenConfig,
    service: &ProtoService,
) -> Result<GeneratedFile> {
    use heck::{ToKebabCase, ToPascalCase};

    let file_name = format!("{}.actor-ref.ts", service.name.to_kebab_case());
    let file_path = config.ts_output_dir.join(&file_name);
    let class_name = format!("{}ActorRef", service.name.to_pascal_case());

    // 收集所有消息类型和编解码函数
    let mut type_imports = std::collections::HashSet::new();
    let mut encode_imports = std::collections::HashSet::new();
    let mut decode_imports = std::collections::HashSet::new();
    for method in &service.methods {
        type_imports.insert(method.input_type.clone());
        type_imports.insert(method.output_type.clone());
        encode_imports.insert(format!("encode{}", method.input_type));
        decode_imports.insert(format!("decode{}", method.output_type));
    }

    let mut content = format!(
        r#"/**
 * 自动生成的 ActorRef 包装
 * 服务: {}
 *
 * ⚠️  请勿手动编辑此文件
 */

import {{ ActorRef }} from '@actr/web';
import type {{ {} }} from './{}.types';
import {{ {} }} from './{}.types';

/**
 * {} Actor 引用
 */
export class {} extends ActorRef {{
  /**
   * 创建新的 ActorRef 实例
   */
  constructor(actorId: string) {{
    super(actorId);
  }}

"#,
        service.name,
        type_imports
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        service.name.to_kebab_case(),
        encode_imports
            .iter()
            .chain(decode_imports.iter())
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        service.name.to_kebab_case(),
        service.name,
        class_name
    );

    // 生成方法
    for method in &service.methods {
        content.push_str(&generate_ts_actor_ref_method(method, &service.name));
        content.push('\n');
    }

    content.push_str("}\n");

    Ok(GeneratedFile::new(file_path, content))
}

/// 生成 ActorRef 方法
fn generate_ts_actor_ref_method(method: &crate::ProtoMethod, service_name: &str) -> String {
    use heck::ToLowerCamelCase;

    let method_name = method.name.to_lower_camel_case();
    let input_type = &method.input_type;
    let output_type = &method.output_type;
    let route_key = format!("{}:{}", service_name, method.name);

    if method.is_streaming {
        // 流式方法 - 使用 subscribe
        format!(
            r#"  /**
   * {} 方法（流式）
   */
  subscribe{}(callback: (data: {}) => void): () => void {{
    return this.subscribe('{}', (bytes: Uint8Array) => {{
      callback(decode{}(bytes));
    }});
  }}
"#,
            method.name, method.name, output_type, route_key, output_type
        )
    } else {
        // 普通 RPC 方法 - 使用 callRaw
        format!(
            r#"  /**
   * {} 方法
   */
  async {}(request: {}): Promise<{}> {{
    const requestBytes = encode{}(request);
    const responseBytes = await this.callRaw('{}', requestBytes);
    return decode{}(responseBytes);
  }}
"#,
            method.name, method_name, input_type, output_type, input_type, route_key, output_type
        )
    }
}

/// 生成 React Hooks
pub fn generate_react_hooks(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<Vec<GeneratedFile>> {
    let mut files = Vec::new();

    for service in services {
        let file = generate_react_hook_for_service(config, service)?;
        files.push(file);
    }

    Ok(files)
}

/// 为单个服务生成 React Hook
fn generate_react_hook_for_service(
    config: &WebCodegenConfig,
    service: &ProtoService,
) -> Result<GeneratedFile> {
    use heck::{ToKebabCase, ToPascalCase};

    let file_name = format!("use-{}.ts", service.name.to_kebab_case());
    let file_path = config.ts_output_dir.join(&file_name);
    let hook_name = format!("use{}", service.name.to_pascal_case());
    let class_name = format!("{}ActorRef", service.name.to_pascal_case());

    let mut content = format!(
        r#"/**
 * 自动生成的 React Hook
 * 服务: {}
 *
 * ⚠️  请勿手动编辑此文件
 */

import {{ useState, useEffect, useCallback }} from 'react';
import {{ {} }} from './{}.actor-ref';

/**
 * {} React Hook
 */
export function {}(actorId: string) {{
  const [actorRef] = useState(() => new {}(actorId));
  const [isConnected, setIsConnected] = useState(false);

  useEffect(() => {{
    // 监听连接状态
    const unlisten = actorRef.on('connection-state-changed', (state) => {{
      setIsConnected(state === 'connected');
    }});

    return () => {{
      unlisten();
    }};
  }}, [actorRef]);

"#,
        service.name,
        class_name,
        service.name.to_kebab_case(),
        service.name,
        hook_name,
        class_name
    );

    // 为每个方法生成便捷的 hook 函数
    for method in &service.methods {
        if !method.is_streaming {
            content.push_str(&generate_react_hook_method(method));
        }
    }

    content.push_str(
        r#"
  return {
    actorRef,
    isConnected,
  };
}
"#,
    );

    Ok(GeneratedFile::new(file_path, content))
}

/// 生成 React Hook 方法
fn generate_react_hook_method(method: &crate::ProtoMethod) -> String {
    use heck::ToLowerCamelCase;

    let method_name = method.name.to_lower_camel_case();
    let input_type = &method.input_type;
    let _output_type = &method.output_type;

    format!(
        r#"  /**
   * {} 方法的便捷调用
   */
  const {} = useCallback(
    async (request: {}) => {{
      return actorRef.{}(request);
    }},
    [actorRef]
  );

"#,
        method.name, method_name, input_type, method_name
    )
}

/// 生成 TypeScript index.ts
fn generate_ts_index_file(
    config: &WebCodegenConfig,
    services: &[ProtoService],
) -> Result<GeneratedFile> {
    use heck::ToKebabCase;

    let file_path = config.ts_output_dir.join("index.ts");

    let mut content = String::from(
        r#"/**
 * 自动生成的导出文件
 *
 * ⚠️  请勿手动编辑此文件
 */

"#,
    );

    // 导出类型
    content.push_str("// 类型定义\n");
    for service in services {
        let file_name = service.name.to_kebab_case();
        content.push_str(&format!("export * from './{}.types';\n", file_name));
    }

    content.push('\n');

    // 导出 ActorRef
    content.push_str("// ActorRef 类\n");
    for service in services {
        let file_name = service.name.to_kebab_case();
        content.push_str(&format!("export * from './{}.actor-ref';\n", file_name));
    }

    // 如果启用了 React Hooks
    if config.generate_react_hooks {
        content.push('\n');
        content.push_str("// React Hooks\n");
        for service in services {
            let file_name = service.name.to_kebab_case();
            content.push_str(&format!("export * from './use-{}';\n", file_name));
        }
    }

    Ok(GeneratedFile::new(file_path, content))
}
