use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let python_fixtures = fixtures_root.join("python/echo");

    // Note: proto files are no longer created during init, they will be pulled via actr install
    ProjectTemplate::load_file(
        &python_fixtures.join("Actr.server.toml.jinja2"),
        files,
        "server/Actr.toml",
    )?;
    ProjectTemplate::load_file(
        &python_fixtures.join("Actr.client.toml.jinja2"),
        files,
        "client/Actr.toml",
    )?;
    ProjectTemplate::load_file(
        &python_fixtures.join("server.py.jinja2"),
        files,
        "server/server.py",
    )?;
    ProjectTemplate::load_file(
        &python_fixtures.join("client.py.jinja2"),
        files,
        "client/client.py",
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

    // Load proto templates
    let proto_fixtures = fixtures_root.join("protos");

    // Server: echo service definition
    // Note: Use PROJECT_NAME_SNAKE in filename so protoc generates matching pb2 filename
    ProjectTemplate::load_file(
        &proto_fixtures.join("echo_service.hbs"),
        files,
        "server/protos/local/{{PROJECT_NAME_SNAKE}}.proto",
    )?;

    // Client: empty local.proto
    ProjectTemplate::load_file(
        &proto_fixtures.join("local.echo.hbs"),
        files,
        "client/protos/local/local.proto",
    )?;

    Ok(())
}
