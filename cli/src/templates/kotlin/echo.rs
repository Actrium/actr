use crate::error::Result;
use crate::templates::ProjectTemplate;
use std::collections::HashMap;
use std::path::Path;

pub fn load(files: &mut HashMap<String, String>) -> Result<()> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");

    // Load root project files
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/build.gradle.kts"),
        files,
        "build.gradle.kts",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/settings.gradle.kts"),
        files,
        "settings.gradle.kts",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/gradle.properties"),
        files,
        "gradle.properties",
    )?;
    ProjectTemplate::load_file(&fixtures_root.join("kotlin/gitignore"), files, ".gitignore")?;
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/echo/Actr.toml"),
        files,
        "Actr.toml",
    )?;

    // Load app module files
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/app/build.gradle.kts"),
        files,
        "app/build.gradle.kts",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/app/src/main/AndroidManifest.xml"),
        files,
        "app/src/main/AndroidManifest.xml",
    )?;

    // Load main source files
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/echo/MainActivity.kt"),
        files,
        "app/src/main/java/MainActivity.kt",
    )?;

    // Load resource files
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/app/src/main/res/layout/activity_main.xml"),
        files,
        "app/src/main/res/layout/activity_main.xml",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/app/src/main/res/values/strings.xml"),
        files,
        "app/src/main/res/values/strings.xml",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/app/src/main/res/values/colors.xml"),
        files,
        "app/src/main/res/values/colors.xml",
    )?;
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/app/src/main/res/values/themes.xml"),
        files,
        "app/src/main/res/values/themes.xml",
    )?;

    // Load test files
    ProjectTemplate::load_file(
        &fixtures_root.join("kotlin/echo/EchoIntegrationTest.kt"),
        files,
        "app/src/androidTest/java/EchoIntegrationTest.kt",
    )?;

    Ok(())
}
