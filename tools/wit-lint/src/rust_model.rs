// SPDX-License-Identifier: Apache-2.0

//! Flattened view of the DynClib ABI Rust source file.
//!
//! `syn` parses the file into a token-faithful AST, but 99% of the lint logic
//! only needs (name, field/variant list, type label). This module reduces the
//! AST to that slice and keeps prost tag attributes alongside each field so
//! the mapping layer can detect silent renames that preserve the wire
//! ordering.
//!
//! Some projected fields (e.g. `RustStruct::name`, `RustConst::{name, ty}`)
//! are recorded for a structurally complete model even when the current
//! mapping layer does not read them; dead-code is suppressed at module scope
//! so the representation stays faithful to the source AST.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, anyhow};
use syn::{Fields, Item};

/// Structural projection of the Rust ABI file.
#[derive(Debug, Clone, Default)]
pub struct RustModel {
    /// Named structs keyed by ident.
    pub structs: HashMap<String, RustStruct>,
    /// Named enums keyed by ident.
    pub enums: HashMap<String, RustEnum>,
    /// Modules containing `pub const` definitions, keyed by module ident.
    /// Values map constant ident -> string-literalized type.
    pub const_modules: HashMap<String, HashMap<String, RustConst>>,
}

#[derive(Debug, Clone)]
pub struct RustStruct {
    pub name: String,
    pub fields: Vec<RustField>,
}

#[derive(Debug, Clone)]
pub struct RustField {
    pub name: String,
    /// Display form of the type (whitespace-normalized).
    pub ty: String,
    /// Optional prost tag number if present on the field (for ordering checks).
    pub prost_tag: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct RustEnum {
    pub name: String,
    pub variants: Vec<RustVariant>,
}

#[derive(Debug, Clone)]
pub struct RustVariant {
    pub name: String,
    /// `None` for unit variants, else a display form of the payload type(s).
    ///
    /// Tuple variants collapse to `"(T1, T2)"`; struct variants collapse to
    /// `"{ f1: T1, f2: T2 }"`. This is deliberately coarse — the mapping
    /// layer decides what level of detail to enforce.
    pub payload: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RustConst {
    pub name: String,
    pub ty: String,
    pub value: String,
}

/// Load and flatten the Rust file at `path` into a [`RustModel`].
pub fn load(path: &Path) -> anyhow::Result<RustModel> {
    let src = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read Rust source at {}", path.display()))?;
    load_str(&src)
}

/// Parse Rust source text directly (used by unit tests).
pub fn load_str(source: &str) -> anyhow::Result<RustModel> {
    let file = syn::parse_file(source).context("failed to parse Rust source")?;
    let mut model = RustModel::default();
    collect(&file.items, &mut model);
    if model.structs.is_empty() && model.enums.is_empty() && model.const_modules.is_empty() {
        return Err(anyhow!(
            "Rust source contained no structs/enums/modules the lint could index"
        ));
    }
    Ok(model)
}

fn collect(items: &[Item], out: &mut RustModel) {
    for item in items {
        match item {
            Item::Struct(s) => {
                let fields = match &s.fields {
                    Fields::Named(named) => named
                        .named
                        .iter()
                        .map(|f| RustField {
                            name: f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default(),
                            ty: normalize_type(&f.ty),
                            prost_tag: extract_prost_tag(&f.attrs),
                        })
                        .collect(),
                    Fields::Unnamed(unnamed) => unnamed
                        .unnamed
                        .iter()
                        .enumerate()
                        .map(|(idx, f)| RustField {
                            name: format!("{idx}"),
                            ty: normalize_type(&f.ty),
                            prost_tag: extract_prost_tag(&f.attrs),
                        })
                        .collect(),
                    Fields::Unit => Vec::new(),
                };
                out.structs.insert(
                    s.ident.to_string(),
                    RustStruct {
                        name: s.ident.to_string(),
                        fields,
                    },
                );
            }
            Item::Enum(e) => {
                let variants = e
                    .variants
                    .iter()
                    .map(|v| RustVariant {
                        name: v.ident.to_string(),
                        payload: render_variant_fields(&v.fields),
                    })
                    .collect();
                out.enums.insert(
                    e.ident.to_string(),
                    RustEnum {
                        name: e.ident.to_string(),
                        variants,
                    },
                );
            }
            Item::Mod(m) => {
                if let Some((_, items)) = &m.content {
                    let mut constants = HashMap::new();
                    for sub in items {
                        if let Item::Const(c) = sub {
                            constants.insert(
                                c.ident.to_string(),
                                RustConst {
                                    name: c.ident.to_string(),
                                    ty: normalize_type(&c.ty),
                                    value: quote_tokens(&c.expr),
                                },
                            );
                        }
                    }
                    if !constants.is_empty() {
                        out.const_modules.insert(m.ident.to_string(), constants);
                    }
                    // Recurse to catch nested modules if ever introduced.
                    collect(items, out);
                }
            }
            _ => {}
        }
    }
}

fn render_variant_fields(fields: &Fields) -> Option<String> {
    match fields {
        Fields::Unit => None,
        Fields::Unnamed(unnamed) => {
            let rendered: Vec<String> = unnamed
                .unnamed
                .iter()
                .map(|f| normalize_type(&f.ty))
                .collect();
            Some(format!("({})", rendered.join(", ")))
        }
        Fields::Named(named) => {
            let rendered: Vec<String> = named
                .named
                .iter()
                .map(|f| {
                    format!(
                        "{}: {}",
                        f.ident.as_ref().map(|i| i.to_string()).unwrap_or_default(),
                        normalize_type(&f.ty)
                    )
                })
                .collect();
            Some(format!("{{ {} }}", rendered.join(", ")))
        }
    }
}

fn normalize_type(ty: &syn::Type) -> String {
    // `quote` would pull in a heavier dep. Use syn's Display via to_string()
    // on the tokenstream instead — guaranteed to be available on syn 2.x.
    let tokens = ty_tokens(ty);
    normalize_whitespace(&tokens)
}

fn ty_tokens(ty: &syn::Type) -> String {
    use std::fmt::Write;
    use syn::__private::ToTokens;
    let mut buf = String::new();
    let mut tokens = proc_macro2::TokenStream::new();
    ty.to_tokens(&mut tokens);
    write!(&mut buf, "{tokens}").ok();
    buf
}

fn quote_tokens<T: syn::__private::ToTokens>(node: &T) -> String {
    let mut tokens = proc_macro2::TokenStream::new();
    node.to_tokens(&mut tokens);
    tokens.to_string()
}

fn normalize_whitespace(s: &str) -> String {
    // Collapse any run of whitespace to a single space and trim edges.
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn extract_prost_tag(attrs: &[syn::Attribute]) -> Option<u32> {
    for attr in attrs {
        if !attr.path().is_ident("prost") {
            continue;
        }
        let Ok(list) = attr.meta.require_list() else {
            continue;
        };
        let tokens = list.tokens.to_string();
        // Look for `tag = "N"` or `tag="N"` within the comma-separated list.
        if let Some(idx) = tokens.find("tag") {
            let rest = &tokens[idx..];
            if let Some(start) = rest.find('"') {
                if let Some(end_rel) = rest[start + 1..].find('"') {
                    let num = &rest[start + 1..start + 1 + end_rel];
                    if let Ok(n) = num.parse::<u32>() {
                        return Some(n);
                    }
                }
            }
            // Or bare form like `tag = 5`
            if let Some(eq_rel) = rest.find('=') {
                let tail = rest[eq_rel + 1..].trim_start();
                let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
                if !digits.is_empty() {
                    if let Ok(n) = digits.parse::<u32>() {
                        return Some(n);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI_RS: &str = r#"
        pub mod op {
            pub const ALPHA: u32 = 1;
            pub const BETA: u32 = 2;
        }

        pub struct Sample {
            #[prost(string, tag = "1")]
            pub field_a: String,
            #[prost(uint32, tag = "2")]
            pub count: u32,
        }

        pub enum Colour {
            Red,
            Green(String),
        }
    "#;

    #[test]
    fn flattens_struct_enum_module() {
        let model = load_str(MINI_RS).expect("parse");
        let s = model.structs.get("Sample").expect("Sample struct");
        assert_eq!(s.fields.len(), 2);
        assert_eq!(s.fields[0].name, "field_a");
        assert_eq!(s.fields[0].prost_tag, Some(1));
        assert_eq!(s.fields[0].ty, "String");

        let e = model.enums.get("Colour").expect("Colour enum");
        assert_eq!(e.variants.len(), 2);
        assert!(e.variants[0].payload.is_none());
        assert_eq!(e.variants[1].payload.as_deref(), Some("(String)"));

        let ops = model.const_modules.get("op").expect("op module");
        assert_eq!(ops.len(), 2);
        assert_eq!(ops.get("ALPHA").unwrap().value, "1");
    }
}
