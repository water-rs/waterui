use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use super::template;
use crate::util;

pub fn create_android_project(
    project_dir: &Path,
    display_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
) -> Result<()> {
    let android_dir = project_dir.join("android");
    util::ensure_directory(&android_dir)?;

    let mut context = HashMap::new();
    context.insert("APP_NAME", display_name.to_string());
    context.insert("CRATE_NAME", crate_name.to_string());
    context.insert("BUNDLE_IDENTIFIER", bundle_identifier.to_string());

    let templates = &template::TEMPLATES_DIR;
    let android_tpl_dir = templates.get_dir("android").expect("android template directory should exist");

    // Process root-level templates
    template::process_template_file(
        android_tpl_dir.get_file("build.gradle.kts.tpl").expect("build.gradle.kts.tpl should exist"),
        &android_dir.join("build.gradle.kts"),
        &context,
    )?;
    template::process_template_file(
        android_tpl_dir.get_file("settings.gradle.kts.tpl").expect("settings.gradle.kts.tpl should exist"),
        &android_dir.join("settings.gradle.kts"),
        &context,
    )?;

    // Process app-level templates
    let app_dir = android_dir.join("app");
    template::process_template_file(
        android_tpl_dir
            .get_file("app/build.gradle.kts.tpl")
            .expect("app/build.gradle.kts.tpl should exist"),
        &app_dir.join("build.gradle.kts"),
        &context,
    )?;

    let main_dir = app_dir.join("src/main");
    template::process_template_file(
        android_tpl_dir
            .get_file("app/src/main/AndroidManifest.xml.tpl")
            .expect("app/src/main/AndroidManifest.xml.tpl should exist"),
        &main_dir.join("AndroidManifest.xml"),
        &context,
    )?;

    // Process res templates
    let values_dir = main_dir.join("res/values");
    template::process_template_file(
        android_tpl_dir
            .get_file("app/src/main/res/values/strings.xml.tpl")
            .expect("app/src/main/res/values/strings.xml.tpl should exist"),
        &values_dir.join("strings.xml"),
        &context,
    )?;
    template::process_template_file(
        android_tpl_dir
            .get_file("app/src/main/res/values/themes.xml.tpl")
            .expect("app/src/main/res/values/themes.xml.tpl should exist"),
        &values_dir.join("themes.xml"),
        &context,
    )?;

    // Process Java/Kotlin source with dynamic path
    let package_path = bundle_identifier.replace('.', "/");
    let java_dir = main_dir.join(format!("java/{}", package_path));
    template::process_template_file(
        android_tpl_dir
            .get_file("app/src/main/java/MainActivity.kt.tpl")
            .expect("app/src/main/java/MainActivity.kt.tpl should exist"),
        &java_dir.join("MainActivity.kt"),
        &context,
    )?;

    // Process root build script
    template::process_template_file(
        android_tpl_dir.get_file("build-rust.sh.tpl").expect("build-rust.sh.tpl should exist"),
        &project_dir.join("build-rust.sh"),
        &context,
    )?;

    // Make it executable
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(project_dir.join("build-rust.sh"))
        .status()?;

    Ok(())
}
