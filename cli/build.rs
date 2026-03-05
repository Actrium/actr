use std::process::Command;

fn main() {
    // Only run if we are in a git repository
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output();

    let git_hash = match output {
        Ok(output) if output.status.success() => {
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        }
        _ => "unknown".to_string(),
    };

    // Get git commit date
    let output_date = Command::new("git")
        .args(["log", "-1", "--format=%cs"])
        .output();

    let git_date = match output_date {
        Ok(output) if output.status.success() => {
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        }
        _ => "unknown".to_string(),
    };

    println!("cargo:rustc-env=ACTR_GIT_HASH={}", git_hash);
    println!("cargo:rustc-env=ACTR_GIT_DATE={}", git_date);
    // Re-run build script if git head changes (monorepo: .git is in parent)
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs");
}
