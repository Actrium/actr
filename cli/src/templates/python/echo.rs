use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let python_fixtures = fixtures_root.join("python/echo");

    ProjectTemplate::load_file(
        &python_fixtures.join("manifest.toml.jinja2"),
        files,
        "manifest.toml",
    )?;
    ProjectTemplate::load_file(
        &python_fixtures.join("workload.py.jinja2"),
        files,
        "workload.py",
    )?;
    ProjectTemplate::load_file(&python_fixtures.join("build.sh.jinja2"), files, "build.sh")?;
    ProjectTemplate::load_file(
        &python_fixtures.join("requirements.txt.jinja2"),
        files,
        "requirements.txt",
    )?;
    ProjectTemplate::load_file(
        &python_fixtures.join("README.md.jinja2"),
        files,
        "README.md",
    )?;
    ProjectTemplate::load_file(
        &python_fixtures.join("gitignore.jinja2"),
        files,
        ".gitignore",
    )?;

    let proto_fixtures = fixtures_root.join("protos");

    ProjectTemplate::load_file(
        &proto_fixtures.join("echo_service.hbs"),
        files,
        "protos/local/{{PROJECT_NAME_SNAKE}}.proto",
    )?;

    Ok(())
}
