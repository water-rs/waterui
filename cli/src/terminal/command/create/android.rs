use color_eyre::eyre::{Context, Result, bail};
use home::home_dir;
use std::{
    collections::HashMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use super::template;
use crate::{ui, util};
use waterui_cli::backend::android;

pub const ANDROID_BACKEND_REPO: &str = "https://github.com/water-rs/android-backend.git";
pub const ANDROID_BACKEND_BRANCH: &str = "dev";
pub const ANDROID_BACKEND_OVERRIDE_ENV: &str = "WATERUI_ANDROID_DEV_BACKEND_DIR";
pub const ANDROID_BACKEND_TAG_PREFIX: &str = "android-backend-v";
const ANDROID_DEV_COMMIT_FILE: &str = ".waterui-dev-commit";

/// Generate the Android Gradle project and associated template files.
///
/// # Errors
/// Returns an error if any template cannot be written to the target project directory.
///
/// # Panics
/// Panics when an expected bundled template is missing; this indicates a build-time bug.
#[allow(clippy::too_many_lines)]
pub fn create_android_project(
    project_dir: &Path,
    display_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
    dev_mode: bool,
) -> Result<()> {
    let android_dir = project_dir.join("android");
    util::ensure_directory(&android_dir)?;

    let use_remote_dev_backend = dev_mode && should_use_remote_dev_backend();

    if !use_remote_dev_backend {
        let backend_source = if dev_mode {
            ensure_dev_android_backend_checkout()?
        } else {
            let source = util::workspace_root().join("backends/android");
            if !source.exists() {
                bail!(
                    "Android backend sources not found at {}. Make sure submodules are initialized.",
                    source.display()
                );
            }
            source
        };
        copy_android_backend(project_dir, &backend_source)?;
        configure_android_local_properties(project_dir)?;
        if dev_mode {
            if let Some(commit) = git_head_commit(&backend_source) {
                write_android_dev_commit(project_dir, &commit)?;
            } else {
                clear_android_dev_commit(project_dir)?;
            }
        } else {
            clear_android_dev_commit(project_dir)?;
        }
    }

    let android_package = android::sanitize_package_name(bundle_identifier);

    let mut context = HashMap::new();
    context.insert("APP_NAME", display_name.to_string());
    context.insert("CRATE_NAME", crate_name.to_string());
    context.insert("CRATE_NAME_SANITIZED", crate_name.replace('-', "_"));
    context.insert("BUNDLE_IDENTIFIER", android_package.clone());
    context.insert("USE_DEV_BACKEND", use_remote_dev_backend.to_string());

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
    let java_dir = main_dir.join(format!("java/{package_path}"));
    template::process_template_file(
        templates
            .get_file("android/app/src/main/java/MainActivity.kt.tpl")
            .unwrap(),
        &java_dir.join("MainActivity.kt"),
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

fn should_use_remote_dev_backend() -> bool {
    matches!(
        env::var("WATERUI_ANDROID_REMOTE_DEV_BACKEND")
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true")),
        Ok(true)
    )
}

pub fn copy_android_backend(project_dir: &Path, source: &Path) -> Result<()> {
    let destination = project_dir.join("backends/android");
    if destination.exists() {
        fs::remove_dir_all(&destination).with_context(|| {
            format!(
                "failed to remove existing backend directory {}",
                destination.display()
            )
        })?;
    }

    if !source.exists() {
        bail!(
            "Android backend sources not found at {}. Ensure the backend repository is available.",
            source.display()
        );
    }

    copy_dir_filtered(source, &destination)?;

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

pub fn configure_android_local_properties(project_dir: &Path) -> Result<()> {
    let backend_dir = project_dir.join("backends/android");
    if !backend_dir.exists() {
        return Ok(());
    }

    let local_properties = backend_dir.join("local.properties");
    if local_properties.exists() {
        return Ok(());
    }

    if let Some(sdk_dir) = android::resolve_android_sdk_path() {
        let escaped = escape_local_property_path(&sdk_dir);
        let contents = format!("sdk.dir={escaped}\n");
        fs::write(&local_properties, contents).with_context(|| {
            format!(
                "failed to write Android local.properties at {}",
                local_properties.display()
            )
        })?;
    }

    Ok(())
}

fn escape_local_property_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if cfg!(windows) {
        raw.replace('\\', "\\\\")
    } else {
        raw.into_owned()
    }
}

pub fn ensure_dev_android_backend_checkout() -> Result<PathBuf> {
    if let Ok(path) = env::var(ANDROID_BACKEND_OVERRIDE_ENV) {
        let path = PathBuf::from(path);
        if path.exists() {
            return Ok(path);
        }
        bail!(
            "WATERUI_ANDROID_DEV_BACKEND_DIR points to {}, but it does not exist.",
            path.display()
        );
    }

    util::require_tool(
        "git",
        "Install Git to download the Android backend sources when using --dev mode.",
    )?;
    let cache_root = dev_backend_cache_root()?;

    let checkout = cache_root.join("android");
    if checkout.exists() {
        update_dev_backend_repo(&checkout)?;
    } else {
        clone_dev_backend_repo(&checkout)?;
    }
    Ok(checkout)
}

fn clone_dev_backend_repo(destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        util::ensure_directory(parent)?;
    }
    run_git_command(
        "Downloading Android dev backend sources",
        "`git clone` did not exit successfully when downloading the Android backend.",
        |cmd| {
            cmd.arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--branch")
                .arg(ANDROID_BACKEND_BRANCH)
                .arg(ANDROID_BACKEND_REPO)
                .arg(destination);
        },
    )?;
    Ok(())
}

fn dev_backend_cache_root() -> Result<PathBuf> {
    backend_cache_root("dev-backends")
}

fn release_backend_cache_root() -> Result<PathBuf> {
    backend_cache_root("releases/android")
}

fn backend_cache_root(subdir: &str) -> Result<PathBuf> {
    if let Some(home) = home_dir() {
        let candidate = home.join(".waterui").join(subdir);
        match util::ensure_directory(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(err) => {
                if let Some(io_err) = err.downcast_ref::<io::Error>() {
                    if io_err.kind() != io::ErrorKind::PermissionDenied {
                        return Err(err);
                    }
                } else {
                    return Err(err);
                }
            }
        }
    }

    let fallback = util::workspace_root()
        .join("target")
        .join(".waterui")
        .join(subdir);
    util::ensure_directory(&fallback)?;
    Ok(fallback)
}

fn run_git_command(
    action: impl Into<String>,
    failure_message: impl Into<String>,
    configure: impl FnOnce(&mut Command),
) -> Result<()> {
    let action = action.into();
    let failure_message = failure_message.into();
    ui::step(&action);
    let mut command = Command::new("git");
    configure(&mut command);
    let output = command
        .output()
        .with_context(|| format!("failed to invoke git to {action}"))?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut details = stderr.trim().to_string();
    if details.is_empty() {
        details = stdout.trim().to_string();
    }
    if details.is_empty() {
        bail!(failure_message);
    }
    bail!("{}: {}", failure_message, details);
}

fn update_dev_backend_repo(repo_dir: &Path) -> Result<()> {
    run_git_command(
        "Fetching Android dev backend updates",
        "`git fetch` failed when updating the Android backend checkout.",
        |cmd| {
            cmd.arg("fetch")
                .arg("origin")
                .arg(ANDROID_BACKEND_BRANCH)
                .current_dir(repo_dir);
        },
    )?;

    run_git_command(
        "Switching Android dev backend branch",
        "`git checkout` failed when switching Android backend branch.",
        |cmd| {
            cmd.arg("checkout")
                .arg(ANDROID_BACKEND_BRANCH)
                .current_dir(repo_dir);
        },
    )?;

    let reset_target = format!("origin/{ANDROID_BACKEND_BRANCH}");
    run_git_command(
        "Resetting Android dev backend to origin",
        format!("`git reset --hard {reset_target}` failed for the Android backend checkout."),
        |cmd| {
            cmd.arg("reset")
                .arg("--hard")
                .arg(&reset_target)
                .current_dir(repo_dir);
        },
    )?;

    run_git_command(
        "Cleaning Android dev backend checkout",
        "`git clean -fdx` failed for the Android backend checkout.",
        |cmd| {
            cmd.arg("clean").arg("-fdx").current_dir(repo_dir);
        },
    )?;

    Ok(())
}

pub fn ensure_android_backend_release(version: &str) -> Result<PathBuf> {
    util::require_tool(
        "git",
        "Install Git to download Android backend releases when using stable projects.",
    )?;
    let cache_root = release_backend_cache_root()?;
    let checkout = cache_root.join(version);
    if checkout.exists() {
        return Ok(checkout);
    }
    if let Some(parent) = checkout.parent() {
        util::ensure_directory(parent)?;
    }
    let tag = format!("{ANDROID_BACKEND_TAG_PREFIX}{version}");
    let message = format!("Downloading Android backend release {version}");
    run_git_command(
        message,
        format!("`git clone` failed while downloading Android backend tag {tag}"),
        |cmd| {
            cmd.arg("clone")
                .arg("--depth")
                .arg("1")
                .arg("--branch")
                .arg(&tag)
                .arg(ANDROID_BACKEND_REPO)
                .arg(&checkout);
        },
    )?;
    Ok(checkout)
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

fn dev_commit_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join("backends")
        .join("android")
        .join(ANDROID_DEV_COMMIT_FILE)
}

pub fn read_android_dev_commit(project_dir: &Path) -> Result<Option<String>> {
    let path = dev_commit_path(project_dir);
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(&path).with_context(|| {
        format!(
            "failed to read Android dev commit metadata at {}",
            path.display()
        )
    })?;
    let commit = contents.trim().to_string();
    if commit.is_empty() {
        Ok(None)
    } else {
        Ok(Some(commit))
    }
}

pub fn write_android_dev_commit(project_dir: &Path, commit: &str) -> Result<()> {
    let path = dev_commit_path(project_dir);
    if let Some(parent) = path.parent() {
        util::ensure_directory(parent)?;
    }
    fs::write(&path, commit.trim()).with_context(|| {
        format!(
            "failed to record Android dev backend commit at {}",
            path.display()
        )
    })?;
    Ok(())
}

pub fn clear_android_dev_commit(project_dir: &Path) -> Result<()> {
    let path = dev_commit_path(project_dir);
    if path.exists() {
        fs::remove_file(&path).with_context(|| {
            format!(
                "failed to remove Android dev commit metadata at {}",
                path.display()
            )
        })?;
    }
    Ok(())
}

pub fn git_head_commit(repo_dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if hash.is_empty() { None } else { Some(hash) }
}
