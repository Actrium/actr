// SPDX-License-Identifier: Apache-2.0

//! Binary entry point — invokes the lint and exits non-zero on drift.

use std::process::ExitCode;

fn main() -> ExitCode {
    let cwd = std::env::current_dir().expect("current_dir");
    let wit_path = cwd.join("core/framework/wit/actr-workload.wit");
    let abi_path = cwd.join("core/framework/src/guest/dynclib_abi.rs");

    match wit_lint::run_default_lint(&wit_path, &abi_path) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("wit-lint: {err:#}");
            ExitCode::from(1)
        }
    }
}
