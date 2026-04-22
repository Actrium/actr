// SPDX-License-Identifier: Apache-2.0

//! Flattened view of the WIT contract used for drift checks.
//!
//! `wit-parser` returns a fully resolved `Resolve` graph that is optimal for
//! codegen but awkward to random-access by name. This module walks the
//! resolved graph once and re-indexes every record / variant / function the
//! lint cares about under its WIT name, so the mapping layer can look up
//! items directly.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, anyhow};
use wit_parser::{Resolve, Type, TypeDefKind};

/// Structural projection of WIT items we compare against Rust.
#[derive(Debug, Clone, Default)]
pub struct WitModel {
    /// `record` types keyed by WIT name (kebab-case).
    pub records: HashMap<String, WitRecord>,
    /// `variant` types keyed by WIT name.
    pub variants: HashMap<String, WitVariant>,
    /// Fully-qualified function keys `"<interface>.<func>"` (e.g.
    /// `"host.call"`, `"workload.dispatch"`).
    pub functions: HashMap<String, WitFunction>,
}

#[derive(Debug, Clone)]
pub struct WitRecord {
    pub name: String,
    pub fields: Vec<WitField>,
}

#[derive(Debug, Clone)]
pub struct WitField {
    pub name: String,
    pub ty: WitTypeRef,
}

#[derive(Debug, Clone)]
pub struct WitVariant {
    pub name: String,
    pub cases: Vec<WitVariantCase>,
}

#[derive(Debug, Clone)]
pub struct WitVariantCase {
    pub name: String,
    /// `None` for unit (no-payload) cases.
    pub payload: Option<WitTypeRef>,
}

#[derive(Debug, Clone)]
pub struct WitFunction {
    pub interface_name: String,
    pub name: String,
    pub params: Vec<WitField>,
    /// `None` if the function has no result (WIT `func() -> ()`-equivalent).
    pub result: Option<WitTypeRef>,
}

/// Lightweight reference-capable type descriptor.
///
/// We keep this stringly for two reasons:
/// 1. The Rust AST side is also string-based (we normalise `Vec<u8>` ->
///    `list<u8>`), so comparison is symmetric.
/// 2. We don't want the lint to transitively re-interpret every nested
///    record; the mapping table declares which records to descend into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitTypeRef {
    /// Primitive such as `u32`, `string`, `bool`.
    Primitive(String),
    /// Reference to a named type (record / variant / enum / resource).
    Named(String),
    /// `list<T>` — inner ref is the flattened element type.
    List(Box<WitTypeRef>),
    /// `option<T>`.
    Option(Box<WitTypeRef>),
    /// `tuple<T1, T2, ...>`.
    Tuple(Vec<WitTypeRef>),
    /// `result<T, E>` — either arm may be `None` (unit).
    Result {
        ok: Option<Box<WitTypeRef>>,
        err: Option<Box<WitTypeRef>>,
    },
}

impl WitTypeRef {
    /// Canonical, human-friendly stringified form used in drift messages.
    pub fn display(&self) -> String {
        match self {
            Self::Primitive(s) | Self::Named(s) => s.clone(),
            Self::List(inner) => format!("list<{}>", inner.display()),
            Self::Option(inner) => format!("option<{}>", inner.display()),
            Self::Tuple(items) => {
                let mut out = String::from("tuple<");
                for (idx, item) in items.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&item.display());
                }
                out.push('>');
                out
            }
            Self::Result { ok, err } => {
                let mut out = String::from("result<");
                out.push_str(
                    &ok.as_ref()
                        .map(|t| t.display())
                        .unwrap_or_else(|| "_".into()),
                );
                out.push_str(", ");
                out.push_str(
                    &err.as_ref()
                        .map(|t| t.display())
                        .unwrap_or_else(|| "_".into()),
                );
                out.push('>');
                out
            }
        }
    }
}

/// Load `path` as a WIT package and flatten it into a [`WitModel`].
pub fn load(path: &Path) -> anyhow::Result<WitModel> {
    let mut resolve = Resolve::new();
    resolve
        .push_file(path)
        .with_context(|| format!("failed to parse WIT at {}", path.display()))?;
    flatten(&resolve)
}

/// Parse WIT source text directly (used by the unit tests).
pub fn load_str(source: &str) -> anyhow::Result<WitModel> {
    let mut resolve = Resolve::new();
    resolve
        .push_str("in-memory.wit", source)
        .context("failed to parse in-memory WIT source")?;
    flatten(&resolve)
}

fn flatten(resolve: &Resolve) -> anyhow::Result<WitModel> {
    let mut out = WitModel::default();

    // Index all named type defs (records / variants) first so function
    // parameters can reference them later.
    for (_, ty) in resolve.types.iter() {
        let Some(name) = ty.name.clone() else {
            continue;
        };
        match &ty.kind {
            TypeDefKind::Record(rec) => {
                let fields = rec
                    .fields
                    .iter()
                    .map(|f| WitField {
                        name: f.name.clone(),
                        ty: type_ref(resolve, &f.ty),
                    })
                    .collect();
                out.records.insert(name.clone(), WitRecord { name, fields });
            }
            TypeDefKind::Variant(variant) => {
                let cases = variant
                    .cases
                    .iter()
                    .map(|c| WitVariantCase {
                        name: c.name.clone(),
                        payload: c.ty.as_ref().map(|t| type_ref(resolve, t)),
                    })
                    .collect();
                out.variants
                    .insert(name.clone(), WitVariant { name, cases });
            }
            // Enums and aliases are not currently referenced by the mapping
            // table; skip silently. They are still surfaced via `Named` on
            // field type references.
            _ => {}
        }
    }

    // Walk interfaces and index their functions under `"<iface>.<fn>"`.
    for (_id, iface) in resolve.interfaces.iter() {
        let Some(iface_name) = iface.name.clone() else {
            continue;
        };
        for (fn_name, func) in &iface.functions {
            let params = func
                .params
                .iter()
                .map(|p| WitField {
                    name: p.name.clone(),
                    ty: type_ref(resolve, &p.ty),
                })
                .collect();
            let result = func.result.as_ref().map(|t| type_ref(resolve, t));
            let key = format!("{iface_name}.{fn_name}");
            out.functions.insert(
                key,
                WitFunction {
                    interface_name: iface_name.clone(),
                    name: fn_name.clone(),
                    params,
                    result,
                },
            );
        }
    }

    if out.records.is_empty() && out.variants.is_empty() && out.functions.is_empty() {
        return Err(anyhow!(
            "WIT resolve produced no records/variants/functions — parser likely misread the file"
        ));
    }
    Ok(out)
}

fn type_ref(resolve: &Resolve, ty: &Type) -> WitTypeRef {
    match ty {
        Type::Bool => WitTypeRef::Primitive("bool".into()),
        Type::U8 => WitTypeRef::Primitive("u8".into()),
        Type::U16 => WitTypeRef::Primitive("u16".into()),
        Type::U32 => WitTypeRef::Primitive("u32".into()),
        Type::U64 => WitTypeRef::Primitive("u64".into()),
        Type::S8 => WitTypeRef::Primitive("s8".into()),
        Type::S16 => WitTypeRef::Primitive("s16".into()),
        Type::S32 => WitTypeRef::Primitive("s32".into()),
        Type::S64 => WitTypeRef::Primitive("s64".into()),
        Type::F32 => WitTypeRef::Primitive("f32".into()),
        Type::F64 => WitTypeRef::Primitive("f64".into()),
        Type::Char => WitTypeRef::Primitive("char".into()),
        Type::String => WitTypeRef::Primitive("string".into()),
        Type::ErrorContext => WitTypeRef::Primitive("error-context".into()),
        Type::Id(id) => {
            let def = &resolve.types[*id];
            if let Some(name) = &def.name {
                return WitTypeRef::Named(name.clone());
            }
            match &def.kind {
                TypeDefKind::List(inner) => WitTypeRef::List(Box::new(type_ref(resolve, inner))),
                TypeDefKind::Option(inner) => {
                    WitTypeRef::Option(Box::new(type_ref(resolve, inner)))
                }
                TypeDefKind::Tuple(t) => {
                    WitTypeRef::Tuple(t.types.iter().map(|t| type_ref(resolve, t)).collect())
                }
                TypeDefKind::Result(r) => WitTypeRef::Result {
                    ok: r.ok.as_ref().map(|t| Box::new(type_ref(resolve, t))),
                    err: r.err.as_ref().map(|t| Box::new(type_ref(resolve, t))),
                },
                TypeDefKind::Type(inner) => type_ref(resolve, inner),
                other => WitTypeRef::Primitive(format!("<unsupported:{}>", other.as_str())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINI_WIT: &str = r#"
        package test:mini@0.1.0;

        interface types {
            record sample {
                field-a: string,
                count: u32,
            }

            variant colour {
                red,
                green(string),
            }
        }

        interface host {
            use types.{sample};
            ping: func(arg: sample) -> result<u32, string>;
        }

        world w {
            import host;
        }
    "#;

    #[test]
    fn flattens_records_variants_and_functions() {
        let model = load_str(MINI_WIT).expect("parse");
        let rec = model.records.get("sample").expect("sample record");
        assert_eq!(rec.fields.len(), 2);
        assert_eq!(rec.fields[0].name, "field-a");
        assert_eq!(rec.fields[1].ty, WitTypeRef::Primitive("u32".into()));

        let var = model.variants.get("colour").expect("colour variant");
        assert_eq!(var.cases.len(), 2);
        assert!(var.cases[0].payload.is_none());
        assert_eq!(
            var.cases[1].payload.as_ref().map(|t| t.display()),
            Some("string".into())
        );

        let func = model.functions.get("host.ping").expect("host.ping");
        assert_eq!(func.params[0].ty, WitTypeRef::Named("sample".into()));
        assert_eq!(
            func.result.as_ref().map(|t| t.display()).unwrap(),
            "result<u32, string>"
        );
    }
}
