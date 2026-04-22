// SPDX-License-Identifier: Apache-2.0

//! Drift report types printed to the console.

use std::fmt;
use std::path::{Path, PathBuf};

/// One piece of shape drift between WIT and Rust.
#[derive(Debug, Clone)]
pub struct ShapeDrift {
    pub kind: ShapeDriftKind,
    /// Human-readable "where": e.g. `"record peer-event"`, `"host.call"`,
    /// `"op::HOST_CALL"`.
    pub location: String,
    /// What disagrees.
    pub message: String,
}

/// Drift taxonomy, useful for sorting and future metrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeDriftKind {
    /// A record in WIT has no matching Rust struct.
    MissingRustStruct,
    /// A variant in WIT has no matching Rust enum.
    MissingRustEnum,
    /// A WIT function declared in the mapping has no matching Rust payload type.
    MissingRustPayload,
    /// A Rust struct field differs in name, order, or type.
    FieldMismatch,
    /// A Rust enum variant differs in name or payload shape.
    VariantMismatch,
    /// A declared ABI op constant is missing from `dynclib_abi::op`.
    MissingOpConstant,
    /// Rust const ABI value differs from the mapping expectation.
    OpConstantValueMismatch,
    /// A mapping row cannot be verified because the WIT side is absent.
    MissingWitItem,
}

impl fmt::Display for ShapeDriftKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::MissingRustStruct => "missing rust struct",
            Self::MissingRustEnum => "missing rust enum",
            Self::MissingRustPayload => "missing rust payload",
            Self::FieldMismatch => "field mismatch",
            Self::VariantMismatch => "variant mismatch",
            Self::MissingOpConstant => "missing op constant",
            Self::OpConstantValueMismatch => "op constant value mismatch",
            Self::MissingWitItem => "missing wit item",
        };
        f.write_str(s)
    }
}

impl fmt::Display for ShapeDrift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.kind, self.location, self.message)
    }
}

/// Aggregated lint result.
#[derive(Debug, Clone)]
pub struct LintReport {
    pub wit_path: PathBuf,
    pub abi_path: PathBuf,
    pub drifts: Vec<ShapeDrift>,
}

impl LintReport {
    pub fn new(wit: &Path, abi: &Path, drifts: Vec<ShapeDrift>) -> Self {
        Self {
            wit_path: wit.to_path_buf(),
            abi_path: abi.to_path_buf(),
            drifts,
        }
    }

    pub fn is_clean(&self) -> bool {
        self.drifts.is_empty()
    }
}

impl fmt::Display for LintReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "wit-lint: checked {} against {}",
            self.wit_path.display(),
            self.abi_path.display(),
        )?;
        if self.drifts.is_empty() {
            writeln!(f, "wit-lint: OK, no drift")?;
        } else {
            writeln!(f, "wit-lint: {} drift(s) detected", self.drifts.len())?;
            for drift in &self.drifts {
                writeln!(f, "  - {drift}")?;
            }
        }
        Ok(())
    }
}
