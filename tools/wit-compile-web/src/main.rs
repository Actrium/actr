// SPDX-License-Identifier: Apache-2.0

//! Binary entry point for the WIT -> wasm-bindgen ABI codegen.
//!
//! Two modes:
//!
//! - default: read `core/framework/wit/actr-workload.wit`, emit the
//!   three sources under `bindings/web/crates/actr-web-abi/src/`
//! - `--check`: regenerate in memory and compare against what is on
//!   disk, exit non-zero on drift (used by CI)

use std::path::PathBuf;
use std::process::ExitCode;

use actr_wit_compile_web::{
    CheckReport, Generated, check_outputs, default_out_dir, default_wit_path, generate_from_path,
    write_outputs,
};

/// Parsed command-line invocation. Kept minimal — the tool is meant
/// to be wired from Cargo / CI, not to grow a general-purpose CLI.
struct Args {
    check: bool,
    wit: PathBuf,
    out: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        check: false,
        wit: default_wit_path(),
        out: default_out_dir(),
    };
    let mut iter = std::env::args().skip(1);
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--check" => args.check = true,
            "--wit" => {
                args.wit = iter
                    .next()
                    .ok_or_else(|| "--wit expects a path".to_string())?
                    .into();
            }
            "--out" => {
                args.out = iter
                    .next()
                    .ok_or_else(|| "--out expects a path".to_string())?
                    .into();
            }
            "-h" | "--help" => {
                eprintln!(
                    "usage: wit-compile-web [--check] [--wit <PATH>] [--out <DIR>]\n\n\
                     Defaults:\n  \
                       --wit core/framework/wit/actr-workload.wit\n  \
                       --out bindings/web/crates/actr-web-abi/src\n\n\
                     --check regenerates in memory and compares against files\n\
                     on disk, exiting 1 on drift. Intended for CI drift gates."
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("wit-compile-web: {e}");
            return ExitCode::from(2);
        }
    };

    let generated: Generated = match generate_from_path(&args.wit) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("wit-compile-web: generation failed: {e:#}");
            return ExitCode::from(1);
        }
    };

    if args.check {
        let report: CheckReport = match check_outputs(&args.out, &generated) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("wit-compile-web: check failed: {e:#}");
                return ExitCode::from(1);
            }
        };
        if report.diffs.is_empty() {
            println!(
                "wit-compile-web: OK — generated sources in {} match WIT",
                args.out.display()
            );
            ExitCode::SUCCESS
        } else {
            eprintln!("wit-compile-web: DRIFT DETECTED in {}:", args.out.display());
            for entry in &report.diffs {
                eprintln!("  - {}: {}", entry.file, entry.reason);
            }
            eprintln!(
                "\nRegenerate with: cargo run -p actr-wit-compile-web -- --out {}",
                args.out.display()
            );
            ExitCode::from(1)
        }
    } else {
        if let Err(e) = write_outputs(&args.out, &generated) {
            eprintln!("wit-compile-web: write failed: {e:#}");
            return ExitCode::from(1);
        }
        println!(
            "wit-compile-web: wrote types.rs / guest.rs / host.rs to {}",
            args.out.display()
        );
        ExitCode::SUCCESS
    }
}
