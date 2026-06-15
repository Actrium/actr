use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");

    ProjectTemplate::load_file(
        &fixtures_root.join("rust/Cargo.service.toml.hbs"),
        files,
        "Cargo.toml",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/build.rs.service.hbs"),
        files,
        "build.rs",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/empty/lib.rs.hbs"),
        files,
        "src/lib.rs",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/empty/manifest.toml.hbs"),
        files,
        "manifest.toml",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/empty/README.md.hbs"),
        files,
        "README.md",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/gitignore.hbs"),
        files,
        ".gitignore",
    )?;

    Ok(())
}
