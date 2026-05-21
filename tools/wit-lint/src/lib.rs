// SPDX-License-Identifier: Apache-2.0

//! WIT <-> DynClib C ABI shape lint.
//!
//! Cross-checks the canonical WIT contract at
//! `core/framework/wit/actr-workload.wit` against the hand-rolled C ABI in
//! `core/framework/src/guest/dynclib_abi.rs`, surfacing field-, type-, variant- and
//! function-shape drift between the two.
//!
//! ## Why a lint and not codegen
//!
//! `wit-bindgen c` produces wasm-targeted glue, not portable C ABI, so the
//! DynClib backend keeps a hand-written ABI. This crate is the drift guard
//! that keeps the hand-written side in lock-step with the WIT source of
//! truth (CI gate at `.github/workflows/ci-rust.yml`).
//!
//! ## What is and is not checked
//!
//! The DynClib backend intentionally implements **only** the dispatch entry
//! plus the four guest->host operations. The 16 observation hooks and the
//! event payload records (`PeerEvent`, `ErrorEvent`, `CredentialEvent`,
//! `BackpressureEvent`, `ErrorCategory`) live exclusively on the WASM
//! Component Model path (wit-bindgen-generated). They are listed here as
//! `wit_only` items so an audit can prove the lint is aware of them.
//!
//! Conversely, several DynClib-specific items have no WIT counterpart
//! (`HostVTable`, ABI op codes, status codes, `InvocationContextV1`,
//! `InitPayloadV1`, `AbiFrame`, `AbiReply`). They are listed as
//! `dynclib_only` so future maintainers can see why the lint ignores them.
//!
//! For the items that *do* span both surfaces, the lint asserts:
//!
//! - WIT records match Rust struct field names and ABI-equivalent types.
//! - WIT variants match Rust enum variants and payload shapes.
//! - WIT functions in `interface host` and `interface workload` map to the
//!   declared Rust payload type (its prost fields capture the canonical
//!   parameter set) and have an ABI op code defined in `dynclib_abi::op`.

use std::path::Path;

pub(crate) mod mapping;
pub(crate) mod report;
pub(crate) mod rust_model;
pub(crate) mod wit_model;

pub use report::LintReport;

/// Run the default lint configuration: load the canonical mapping table and
/// compare the two source files against it.
pub fn run_default_lint(wit_path: &Path, abi_path: &Path) -> anyhow::Result<LintReport> {
    let wit = wit_model::load(wit_path)?;
    let abi = rust_model::load(abi_path)?;
    let drifts = mapping::default_mapping().check(&wit, &abi);
    Ok(LintReport::new(wit_path, abi_path, drifts))
}
