use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>, is_service: bool) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let ts_fixtures = fixtures_root.join("typescript/echo");

    ProjectTemplate::load_file(&ts_fixtures.join("package.json.hbs"), files, "package.json")?;
    ProjectTemplate::load_file(
        &ts_fixtures.join("tsconfig.json.hbs"),
        files,
        "tsconfig.json",
    )?;
    ProjectTemplate::load_file(&ts_fixtures.join("gitignore.hbs"), files, ".gitignore")?;
    ProjectTemplate::load_file(&ts_fixtures.join("README.md.hbs"), files, "README.md")?;

    if is_service {
        ProjectTemplate::load_file(
            &ts_fixtures.join("actr.toml.service.hbs"),
            files,
            "actr.toml",
        )?;
        ProjectTemplate::load_file(
            &ts_fixtures.join("index.service.ts.hbs"),
            files,
            "src/actr_service.ts",
        )?;
    } else {
        ProjectTemplate::load_file(&ts_fixtures.join("actr.toml.hbs"), files, "actr.toml")?;
        ProjectTemplate::load_file(
            &ts_fixtures.join("index.ts.hbs"),
            files,
            "src/actr_service.ts",
        )?;
    }

    Ok(())
}
