use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

use proc_macro2::TokenStream;
use quote::ToTokens;
use syn::{Attribute, Fields, Type};

fn main() {
    println!("cargo:rerun-if-changed=src/config");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let config_dir = Path::new(&manifest_dir).join("src/config");
    let template_output_path =
        Path::new(&manifest_dir).join("../../deploy/tpl/config.template.toml");

    // Ensure output directory exists
    if let Some(parent) = template_output_path.parent()
        && !parent.exists()
    {
        fs::create_dir_all(parent).expect("Failed to create template directory");
    }

    match generate_config_template(&config_dir, &template_output_path) {
        Ok(_) => println!("Configuration template generated successfully"),
        Err(e) => {
            eprintln!("Failed to generate configuration template: {e}");
            std::process::exit(1);
        }
    }
}

fn generate_config_template(
    config_dir: &Path,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Parse all submodule files and build a combined structs map
    let structs = parse_all_config_modules(config_dir)?;

    let mut template_content = String::new();

    // Add file header comments
    template_content.push_str("# Actor-RTC 辅助服务配置文件模板\n");
    template_content.push_str("# 此文件由 config crate 的 build.rs 自动生成\n");
    template_content.push_str("# 请根据实际部署环境修改相应的配置值\n\n");

    // Find ActrixConfig struct and generate template
    if let Some(actrix_config) = structs.get("ActrixConfig") {
        generate_struct_template(&mut template_content, actrix_config, &structs, "", 0)?;
    }

    fs::write(output_path, template_content)?;
    Ok(())
}

/// Parse all config modules and their submodules recursively
fn parse_all_config_modules(
    config_dir: &Path,
) -> Result<HashMap<String, syn::ItemStruct>, Box<dyn std::error::Error>> {
    let mut structs = HashMap::new();

    // Parse main mod.rs
    let mod_path = config_dir.join("mod.rs");
    if mod_path.exists() {
        parse_file_structs(&mod_path, &mut structs)?;
    }

    // Parse other .rs files in config directory
    for entry in fs::read_dir(config_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            // Skip mod.rs since we already parsed it
            if path.file_name().is_some_and(|name| name != "mod.rs") {
                parse_file_structs(&path, &mut structs)?;
            }
        } else if path.is_dir() {
            // Parse submodule directories (like bind/)
            parse_submodule_dir(&path, &mut structs)?;
        }
    }

    Ok(structs)
}

/// Parse struct definitions from a single .rs file
fn parse_file_structs(
    path: &Path,
    structs: &mut HashMap<String, syn::ItemStruct>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    if let Ok(syntax_tree) = syn::parse_str::<syn::File>(&content) {
        for item in syntax_tree.items {
            if let syn::Item::Struct(struct_item) = item {
                structs.insert(struct_item.ident.to_string(), struct_item);
            }
        }
    }
    Ok(())
}

/// Parse submodule directory (like bind/)
fn parse_submodule_dir(
    dir: &Path,
    structs: &mut HashMap<String, syn::ItemStruct>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            parse_file_structs(&path, structs)?;
        }
    }
    Ok(())
}

fn generate_struct_template(
    template: &mut String,
    struct_item: &syn::ItemStruct,
    structs: &HashMap<String, syn::ItemStruct>,
    section_prefix: &str,
    _indent_level: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Fields::Named(fields) = &struct_item.fields {
        for field in &fields.named {
            let field_name = field.ident.as_ref().unwrap().to_string();

            // Add field doc comments
            add_doc_comments(template, &field.attrs);

            // Generate config based on field type
            match &field.ty {
                // Handle Option<T> types
                Type::Path(type_path) if is_option_type(type_path) => {
                    let inner_type = extract_option_inner_type(type_path);
                    if structs.contains_key(&inner_type) {
                        // Option<StructType> - generate commented-out struct config
                        template.push_str(&format!("# [{section_prefix}{field_name}]\n"));
                        if let Some(inner_struct) = structs.get(&inner_type) {
                            generate_optional_struct_fields(
                                template,
                                inner_struct,
                                structs,
                                &format!("{section_prefix}{field_name}."),
                            )?;
                        }
                    } else {
                        // Option<primitive type> - generate commented-out field
                        let default_value =
                            get_default_value_for_field_with_context(&field_name, section_prefix);
                        template.push_str(&format!("# {field_name} = {default_value}\n"));
                    }
                }
                // Handle custom struct types
                Type::Path(type_path) if structs.contains_key(&type_path_to_string(type_path)) => {
                    let struct_type = type_path_to_string(type_path);
                    template.push_str(&format!("\n[{section_prefix}{field_name}]\n"));
                    if let Some(inner_struct) = structs.get(&struct_type) {
                        generate_struct_template(
                            template,
                            inner_struct,
                            structs,
                            &format!("{section_prefix}{field_name}."),
                            0,
                        )?;
                    }
                }
                // Handle primitive types
                _ => {
                    let default_value =
                        get_default_value_for_field_with_context(&field_name, section_prefix);
                    template.push_str(&format!("{field_name} = {default_value}\n"));
                }
            }

            template.push('\n');
        }
    }

    Ok(())
}

fn generate_optional_struct_fields(
    template: &mut String,
    struct_item: &syn::ItemStruct,
    structs: &HashMap<String, syn::ItemStruct>,
    section_prefix: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Fields::Named(fields) = &struct_item.fields {
        for field in &fields.named {
            let field_name = field.ident.as_ref().unwrap().to_string();

            // Add field doc comments (commented out)
            add_doc_comments_commented(template, &field.attrs);

            // Generate commented-out field config
            match &field.ty {
                Type::Path(type_path) if structs.contains_key(&type_path_to_string(type_path)) => {
                    // Nested struct - generate sub-section
                    let struct_type = type_path_to_string(type_path);
                    template.push_str(&format!("# [{section_prefix}{field_name}]\n"));
                    if let Some(inner_struct) = structs.get(&struct_type) {
                        generate_optional_struct_fields(
                            template,
                            inner_struct,
                            structs,
                            &format!("{section_prefix}{field_name}."),
                        )?;
                    }
                }
                _ => {
                    let default_value =
                        get_default_value_for_field_with_context(&field_name, section_prefix);
                    template.push_str(&format!("# {field_name} = {default_value}\n"));
                }
            }
        }
    }

    Ok(())
}

fn add_doc_comments(template: &mut String, attrs: &[Attribute]) {
    for attr in attrs {
        if attr.path().is_ident("doc")
            && let Ok(doc_string) = attr.value()
        {
            let doc_content = doc_string.to_token_stream().to_string();
            // Remove quotes and clean content
            let cleaned = doc_content.trim_matches('"').trim();
            if !cleaned.is_empty() {
                template.push_str(&format!("#{cleaned}\n"));
            }
        }
    }
}

fn add_doc_comments_commented(template: &mut String, attrs: &[Attribute]) {
    for attr in attrs {
        if attr.path().is_ident("doc")
            && let Ok(doc_string) = attr.value()
        {
            let doc_content = doc_string.to_token_stream().to_string();
            // Remove quotes and clean content
            let cleaned = doc_content.trim_matches('"').trim();
            if !cleaned.is_empty() {
                template.push_str(&format!("# #{cleaned}\n"));
            }
        }
    }
}

fn is_option_type(type_path: &syn::TypePath) -> bool {
    if let Some(segment) = type_path.path.segments.first() {
        segment.ident == "Option"
    } else {
        false
    }
}

fn extract_option_inner_type(type_path: &syn::TypePath) -> String {
    if let Some(segment) = type_path.path.segments.first()
        && let syn::PathArguments::AngleBracketed(args) = &segment.arguments
        && let Some(syn::GenericArgument::Type(inner_type)) = args.args.first()
    {
        return type_to_string(inner_type);
    }
    "Unknown".to_string()
}

fn type_path_to_string(type_path: &syn::TypePath) -> String {
    type_path.path.segments.last().unwrap().ident.to_string()
}

fn type_to_string(ty: &Type) -> String {
    match ty {
        Type::Path(type_path) => type_path_to_string(type_path),
        _ => "Unknown".to_string(),
    }
}

fn get_default_value_for_field_with_context(field_name: &str, section_prefix: &str) -> String {
    match field_name {
        // Main config fields
        "enable" => {
            // Differentiate between top-level enable and tracing.enable
            if section_prefix.contains("tracing") {
                "false".to_string()
            } else {
                "15".to_string()
            }
        }
        "name" => "\"actrix-default\"".to_string(),
        "env" => "\"dev\"".to_string(),
        "user" => "\"actrix\"".to_string(),
        "group" => "\"actrix\"".to_string(),
        "pid" => "\"logs/actrix.pid\"".to_string(),
        "location_tag" => "\"default-location\"".to_string(),
        // Bug 3 fix: changed from "sqlite" to "sqlite_path"
        "sqlite_path" => "\"database\"".to_string(),
        "actrix_shared_key" => "\"CHANGE_ME_32_CHAR_RANDOM_KEY_HERE\"".to_string(),

        // Observability config fields
        "filter_level" => "\"info\"".to_string(),
        "output" => "\"console\"".to_string(),
        "rotate" => "false".to_string(),
        "path" => "\"logs/\"".to_string(),

        // Tracing config fields
        "service_name" => "\"actrix\"".to_string(),
        "endpoint" => {
            if section_prefix.contains("supervisor") {
                "\"http://localhost:50051\"".to_string()
            } else {
                "\"http://127.0.0.1:4317\"".to_string()
            }
        }

        // Network config fields
        "domain_name" => "\"localhost\"".to_string(),
        "advertised_ip" => "\"127.0.0.1\"".to_string(),
        "ip" => "\"0.0.0.0\"".to_string(),

        // Port config - differentiate by context
        "port" => {
            if section_prefix.contains("ice") {
                "3478".to_string() // ICE (STUN/TURN) port
            } else if section_prefix.contains("https") {
                "8443".to_string() // HTTPS port
            } else {
                "8080".to_string() // HTTP default port
            }
        }

        // TURN config
        "advertised_port" => "3478".to_string(),
        "cert" => "\"certificates/server.crt\"".to_string(),
        "key" => "\"certificates/server.key\"".to_string(),
        "relay_port_range" => "\"49152-65535\"".to_string(),
        "realm" => "\"actor-rtc.local\"".to_string(),

        // Supervisor config
        "node_id" => "\"\"".to_string(),
        "secret" => "\"\"".to_string(),
        "shared_secret" => {
            "\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"".to_string()
        }
        "node_name" => "\"actrix-node\"".to_string(),

        // Default fallback
        _ => "\"\"".to_string(),
    }
}

#[allow(dead_code)]
fn get_default_value_for_field(field_name: &str) -> String {
    get_default_value_for_field_with_context(field_name, "")
}

trait AttributeValue {
    fn value(&self) -> Result<TokenStream, syn::Error>;
}

impl AttributeValue for Attribute {
    fn value(&self) -> Result<TokenStream, syn::Error> {
        self.parse_args()
    }
}
