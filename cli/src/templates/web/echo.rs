use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>, is_service: bool) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let web_fixtures = fixtures_root.join("web/echo");

    ProjectTemplate::load_file(
        &web_fixtures.join("package.json.hbs"),
        files,
        "package.json",
    )?;
    ProjectTemplate::load_file(
        &web_fixtures.join("tsconfig.json.hbs"),
        files,
        "tsconfig.json",
    )?;
    ProjectTemplate::load_file(
        &web_fixtures.join("vite.config.ts.hbs"),
        files,
        "vite.config.ts",
    )?;
    ProjectTemplate::load_file(&web_fixtures.join("gitignore.hbs"), files, ".gitignore")?;
    ProjectTemplate::load_file(&web_fixtures.join("README.md.hbs"), files, "README.md")?;

    if is_service {
        ProjectTemplate::load_file(
            &web_fixtures.join("Actr.toml.service.hbs"),
            files,
            "Actr.toml",
        )?;
        ProjectTemplate::load_file(
            &web_fixtures.join("main.service.ts.hbs"),
            files,
            "src/main.ts",
        )?;
        ProjectTemplate::load_file(
            &web_fixtures.join("index.service.html.hbs"),
            files,
            "index.html",
        )?;
    } else {
        ProjectTemplate::load_file(&web_fixtures.join("Actr.toml.hbs"), files, "Actr.toml")?;
        ProjectTemplate::load_file(&web_fixtures.join("main.ts.hbs"), files, "src/main.ts")?;
        ProjectTemplate::load_file(&web_fixtures.join("index.html.hbs"), files, "index.html")?;
    }

    Ok(())
}
