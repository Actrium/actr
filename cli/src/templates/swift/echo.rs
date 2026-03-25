use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>, is_service: bool) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");

    ProjectTemplate::load_file(
        &fixtures_root.join("swift/project.yml.hbs"),
        files,
        "project.yml",
    )?;
    if is_service {
        ProjectTemplate::load_file(
            &fixtures_root.join("swift/echo/actr.toml.service.hbs"),
            files,
            "actr.toml",
        )?;
    } else {
        ProjectTemplate::load_file(
            &fixtures_root.join("swift/echo/actr.toml.hbs"),
            files,
            "actr.toml",
        )?;
    }
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/Actr.lock.toml.hbs"),
        files,
        "Actr.lock.toml",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/gitignore.hbs"),
        files,
        ".gitignore",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/dist.keep.hbs"),
        files,
        "dist/.keep",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/echo/README.md.hbs"),
        files,
        "README.md",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/echo/README.zh.md.hbs"),
        files,
        "README.zh.md",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/Info.plist.hbs"),
        files,
        "{{PROJECT_NAME_PASCAL}}/Info.plist",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/App.swift.hbs"),
        files,
        "{{PROJECT_NAME_PASCAL}}/{{PROJECT_NAME_PASCAL}}.swift",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/echo/ContentView.swift.hbs"),
        files,
        "{{PROJECT_NAME_PASCAL}}/ContentView.swift",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/echo/ActrService.swift.hbs"),
        files,
        "{{PROJECT_NAME_PASCAL}}/ActrService.swift",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/Assets.xcassets/Contents.json"),
        files,
        "{{PROJECT_NAME_PASCAL}}/Assets.xcassets/Contents.json",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/Assets.xcassets/AccentColor.colorset/Contents.json"),
        files,
        "{{PROJECT_NAME_PASCAL}}/Assets.xcassets/AccentColor.colorset/Contents.json",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/Assets.xcassets/AppIcon.appiconset/Contents.json"),
        files,
        "{{PROJECT_NAME_PASCAL}}/Assets.xcassets/AppIcon.appiconset/Contents.json",
    )?;

    Ok(())
}
