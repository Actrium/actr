use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");

    // Load template files from disk with placeholders in path
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/project.yml.hbs"),
        files,
        "project.yml",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/data-stream/manifest.toml.hbs"),
        files,
        "manifest.toml",
    )?;
    // Load empty manifest.lock.toml template
    // This is REQUIRED for Swift projects because:
    // 1. project.yml references it as a resource file
    // 2. xcodegen requires all referenced files to exist during project generation
    // 3. The lock file will be populated later by `actr deps install`
    // See: fixtures/swift/project.yml.hbs:30-32 for the resource reference
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/manifest.lock.toml.hbs"),
        files,
        "manifest.lock.toml",
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
        &fixtures_root.join("swift/data-stream/ContentView.swift.hbs"),
        files,
        "{{PROJECT_NAME_PASCAL}}/ContentView.swift",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("swift/data-stream/ActrService.swift.hbs"),
        files,
        "{{PROJECT_NAME_PASCAL}}/ActrService.swift",
    )?;
    // Load fixture files (no placeholders, fixed paths)
    // Note: proto files are no longer created during init, they will be pulled via actr deps install
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
