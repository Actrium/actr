use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>, is_service: bool) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");

    // Cargo.toml
    let cargo_hbs = if is_service {
        fixtures_root.join("rust/Cargo.service.toml.hbs")
    } else {
        fixtures_root.join("rust/Cargo.toml.hbs")
    };
    ProjectTemplate::load_file(&cargo_hbs, files, "Cargo.toml")?;

    // Role-specific fixtures.
    if is_service {
        ProjectTemplate::load_file(
            &fixtures_root.join("rust/lib.rs.service.hbs"),
            files,
            "src/lib.rs",
        )?;
        ProjectTemplate::load_file(
            &fixtures_root.join("rust/build.rs.service.hbs"),
            files,
            "build.rs",
        )?;
        ProjectTemplate::load_file(
            &fixtures_root.join("rust/echo_service.rs.hbs"),
            files,
            "src/echo_service.rs",
        )?;
    } else {
        ProjectTemplate::load_file(
            &fixtures_root.join("rust/main.rs.hbs"),
            files,
            "src/main.rs",
        )?;
        ProjectTemplate::load_file(&fixtures_root.join("rust/lib.rs.hbs"), files, "src/lib.rs")?;
        ProjectTemplate::load_file(
            &fixtures_root.join("rust/echo/actr.toml.hbs"),
            files,
            "actr.toml",
        )?;
    }
    let manifest_toml_hbs = if is_service {
        fixtures_root.join("rust/echo/manifest.toml.service.hbs")
    } else {
        fixtures_root.join("rust/echo/manifest.toml.hbs")
    };
    ProjectTemplate::load_file(&manifest_toml_hbs, files, "manifest.toml")?;

    // README.md
    let readme_hbs = if is_service {
        fixtures_root.join("rust/echo/README.md.service.hbs")
    } else {
        fixtures_root.join("rust/echo/README.md.hbs")
    };
    ProjectTemplate::load_file(&readme_hbs, files, "README.md")?;

    // .gitignore
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/gitignore.hbs"),
        files,
        ".gitignore",
    )?;

    Ok(())
}
