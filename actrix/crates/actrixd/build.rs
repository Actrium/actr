fn main() {
    // Rebuild when admin UI source changes
    println!("cargo:rerun-if-changed=admin/web/src");
    println!("cargo:rerun-if-changed=admin/web/index.html");
    println!("cargo:rerun-if-changed=admin/web/package.json");

    #[cfg(feature = "admin-ui")]
    {
        let dist_path = std::path::Path::new("admin/web/dist");
        if !dist_path.exists() {
            // Try to build the Admin UI when a JS toolchain is available
            // (developer convenience). In a pure-Rust environment (e.g. CI
            // runners without node), fall back to an empty dist directory so
            // the rust-embed build still compiles. The production Admin UI is
            // produced by the dedicated Admin UI CI / release pipeline, which
            // runs whenever admin/web changes.
            if !try_build_admin_ui() {
                println!(
                    "cargo:warning=Admin UI not built (no JS toolchain or build failed); \
                     embedding empty dist. Run `pnpm build` in admin/web for a UI-enabled binary."
                );
                let _ = std::fs::create_dir_all(dist_path);
            }
        }
    }
}

/// Attempt to build the Admin UI with pnpm (preferred) or npm.
/// Returns `true` only if a `dist` was successfully produced; any missing
/// toolchain or build failure returns `false` so the caller can degrade
/// gracefully instead of failing the whole Rust build.
#[cfg(feature = "admin-ui")]
fn try_build_admin_ui() -> bool {
    fn available(cmd: &str) -> bool {
        std::process::Command::new(cmd)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    let package_manager = if available("pnpm") {
        "pnpm"
    } else if available("npm") {
        "npm"
    } else {
        return false;
    };

    println!("cargo:warning=Admin UI dist not found, building with {package_manager}...");

    let node_modules = std::path::Path::new("admin/web/node_modules");
    if !node_modules.exists() {
        println!("cargo:warning=Installing Admin UI dependencies...");
        let installed = std::process::Command::new(package_manager)
            .args(["install"])
            .current_dir("admin/web")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !installed {
            return false;
        }
    }

    println!("cargo:warning=Building Admin UI...");
    let built = std::process::Command::new(package_manager)
        .args(["run", "build"])
        .current_dir("admin/web")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if built {
        println!("cargo:warning=Admin UI built successfully");
    }
    built
}
