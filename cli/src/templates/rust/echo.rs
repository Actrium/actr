use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>, is_service: bool) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");

    // Cargo.toml
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/Cargo.toml.hbs"),
        files,
        "Cargo.toml",
    )?;

    // Role-specific fixtures: main.rs, actr.toml, README.md
    let (main_hbs, actr_toml_hbs, readme_hbs) = if is_service {
        (
            fixtures_root.join("rust/main.rs.service.hbs"),
            fixtures_root.join("rust/echo/actr.toml.service.hbs"),
            fixtures_root.join("rust/echo/README.md.service.hbs"),
        )
    } else {
        (
            fixtures_root.join("rust/main.rs.hbs"),
            fixtures_root.join("rust/echo/actr.toml.hbs"),
            fixtures_root.join("rust/echo/README.md.hbs"),
        )
    };

    ProjectTemplate::load_file(&main_hbs, files, "src/main.rs")?;
    if is_service {
        ProjectTemplate::load_file(
            &fixtures_root.join("rust/echo_service.rs.hbs"),
            files,
            "src/echo_service.rs",
        )?;
    }
    ProjectTemplate::load_file(&actr_toml_hbs, files, "actr.toml")?;

    // README.md
    ProjectTemplate::load_file(&readme_hbs, files, "README.md")?;

    // .gitignore
    ProjectTemplate::load_file(
        &fixtures_root.join("rust/gitignore.hbs"),
        files,
        ".gitignore",
    )?;

    Ok(())
}
