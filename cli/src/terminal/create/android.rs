use color_eyre::eyre::{Context, Result, bail, eyre};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::template;
use crate::util;

pub fn create_android_project(
    project_dir: &Path,
    display_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
    use_dev_backend: bool,
) -> Result<()> {
    let android_dir = project_dir.join("android");
    util::ensure_directory(&android_dir)?;

    copy_android_backend(project_dir, use_dev_backend)?;

    let android_package = crate::android::sanitize_package_name(bundle_identifier);

    let mut context = HashMap::new();
    context.insert("APP_NAME", display_name.to_string());
    context.insert("CRATE_NAME", crate_name.to_string());
    context.insert("CRATE_NAME_SANITIZED", crate_name.replace('-', "_"));
    context.insert("BUNDLE_IDENTIFIER", android_package.clone());

    let templates = &template::TEMPLATES_DIR;

    // Process root-level templates
    template::process_template_file(
        templates.get_file("android/build.gradle.kts.tpl").unwrap(),
        &android_dir.join("build.gradle.kts"),
        &context,
    )?;
    template::process_template_file(
        templates.get_file("android/gradle.properties.tpl").unwrap(),
        &android_dir.join("gradle.properties"),
        &context,
    )?;
    template::process_template_file(
        templates
            .get_file("android/settings.gradle.kts.tpl")
            .unwrap(),
        &android_dir.join("settings.gradle.kts"),
        &context,
    )?;

    // Process app-level templates
    let app_dir = android_dir.join("app");
    template::process_template_file(
        templates
            .get_file("android/app/build.gradle.kts.tpl")
            .unwrap(),
        &app_dir.join("build.gradle.kts"),
        &context,
    )?;
    fs::write(
        app_dir.join("proguard-rules.pro"),
        templates
            .get_file("android/app/proguard-rules.pro")
            .unwrap()
            .contents(),
    )?;

    let main_dir = app_dir.join("src/main");
    template::process_template_file(
        templates
            .get_file("android/app/src/main/AndroidManifest.xml.tpl")
            .unwrap(),
        &main_dir.join("AndroidManifest.xml"),
        &context,
    )?;

    // Process res templates
    let values_dir = main_dir.join("res/values");
    template::process_template_file(
        templates
            .get_file("android/app/src/main/res/values/strings.xml.tpl")
            .unwrap(),
        &values_dir.join("strings.xml"),
        &context,
    )?;
    template::process_template_file(
        templates
            .get_file("android/app/src/main/res/values/themes.xml.tpl")
            .unwrap(),
        &values_dir.join("themes.xml"),
        &context,
    )?;
    template::process_template_file(
        templates
            .get_file("android/app/src/main/res/values/colors.xml.tpl")
            .unwrap(),
        &values_dir.join("colors.xml"),
        &context,
    )?;

    let drawable_dir = main_dir.join("res/drawable");
    template::process_template_file(
        templates
            .get_file("android/app/src/main/res/drawable/ic_launcher_foreground.xml.tpl")
            .unwrap(),
        &drawable_dir.join("ic_launcher_foreground.xml"),
        &context,
    )?;

    let mipmap_anydpi_dir = main_dir.join("res/mipmap-anydpi-v26");
    template::process_template_file(
        templates
            .get_file("android/app/src/main/res/mipmap-anydpi-v26/ic_launcher.xml.tpl")
            .unwrap(),
        &mipmap_anydpi_dir.join("ic_launcher.xml"),
        &context,
    )?;
    template::process_template_file(
        templates
            .get_file("android/app/src/main/res/mipmap-anydpi-v26/ic_launcher_round.xml.tpl")
            .unwrap(),
        &mipmap_anydpi_dir.join("ic_launcher_round.xml"),
        &context,
    )?;

    // Process Java/Kotlin source with dynamic path
    let package_path = android_package.replace('.', "/");
    let java_dir = main_dir.join(format!("java/{}", package_path));
    template::process_template_file(
        templates
            .get_file("android/app/src/main/java/MainActivity.kt.tpl")
            .unwrap(),
        &java_dir.join("MainActivity.kt"),
        &context,
    )?;
    template::process_template_file(
        templates
            .get_file("android/app/src/main/java/WaterUIApplication.kt.tpl")
            .unwrap(),
        &java_dir.join("WaterUIApplication.kt"),
        &context,
    )?;

    // Process root build script
    template::process_template_file(
        templates.get_file("android/build-rust.sh.tpl").unwrap(),
        &project_dir.join("build-rust.sh"),
        &context,
    )?;

    // Copy Gradle wrapper scripts and configuration
    let gradlew_path = android_dir.join("gradlew");
    fs::write(
        &gradlew_path,
        templates.get_file("android/gradlew").unwrap().contents(),
    )?;
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&gradlew_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&gradlew_path, perms)?;
    }

    fs::write(
        android_dir.join("gradlew.bat"),
        templates
            .get_file("android/gradlew.bat")
            .unwrap()
            .contents(),
    )?;

    let gradle_wrapper_dir = android_dir.join("gradle/wrapper");
    util::ensure_directory(&gradle_wrapper_dir)?;
    fs::write(
        gradle_wrapper_dir.join("gradle-wrapper.jar"),
        templates
            .get_file("android/gradle/wrapper/gradle-wrapper.jar")
            .unwrap()
            .contents(),
    )?;
    fs::write(
        gradle_wrapper_dir.join("gradle-wrapper.properties"),
        templates
            .get_file("android/gradle/wrapper/gradle-wrapper.properties")
            .unwrap()
            .contents(),
    )?;

    // Make scripts executable
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(project_dir.join("build-rust.sh"))
        .status()?;

    Ok(())
}

const ANDROID_BACKEND_GIT_URL: &str = "https://github.com/water-rs/android-backend.git";
const ANDROID_BACKEND_DEV_BRANCH: &str = "dev";

fn copy_android_backend(project_dir: &Path, use_dev_backend: bool) -> Result<()> {
    let destination = project_dir.join("backends/android");
    if destination.exists() {
        fs::remove_dir_all(&destination).with_context(|| {
            format!(
                "failed to remove existing backend directory {}",
                destination.display()
            )
        })?;
    }

    if use_dev_backend {
        clone_android_backend(&destination)?;
    } else {
        let source = util::workspace_root().join("backends/android");
        if !source.exists() {
            return Err(eyre!(
                "Android backend sources not found at {}",
                source.display()
            ));
        }

        copy_dir_filtered(&source, &destination)?;
    }

    #[cfg(unix)]
    {
        let gradlew = destination.join("gradlew");
        if gradlew.exists() {
            let mut perms = fs::metadata(&gradlew)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&gradlew, perms)?;
        }
    }

    Ok(())
}

fn clone_android_backend(destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        util::ensure_directory(parent)?;
    }

    util::require_tool(
        "git",
        "Install Git to fetch the Android backend or rerun without --dev.",
    )?;

    let repo_url = std::env::var("WATERUI_ANDROID_BACKEND_URL")
        .unwrap_or_else(|_| ANDROID_BACKEND_GIT_URL.to_string());
    let status = Command::new("git")
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg("--branch")
        .arg(ANDROID_BACKEND_DEV_BRANCH)
        .arg("--single-branch")
        .arg(&repo_url)
        .arg(destination)
        .status()
        .with_context(|| format!("failed to clone Android backend from {}", repo_url))?;

    if !status.success() {
        if destination.exists() {
            let _ = fs::remove_dir_all(destination);
        }
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "terminated by signal".to_string());
        bail!(
            "git clone failed when fetching Android backend from {} (exit status: {})",
            repo_url,
            code
        );
    }

    Ok(())
}

fn copy_dir_filtered(src: &Path, dst: &Path) -> Result<()> {
    util::ensure_directory(dst)?;

    for entry in fs::read_dir(src).with_context(|| format!("failed to read {}", src.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", src.display()))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if should_skip(&name_str) {
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(&name);

        if entry.file_type()?.is_dir() {
            copy_dir_filtered(&src_path, &dst_path)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                util::ensure_directory(parent)?;
            }
            fs::copy(&src_path, &dst_path).with_context(|| {
                format!(
                    "failed to copy {} to {}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn should_skip(name: &str) -> bool {
    matches!(
        name,
        "build"
            | ".gradle"
            | ".cxx"
            | "local.properties"
            | ".DS_Store"
            | "target"
            | ".idea"
            | ".git"
    )
}
