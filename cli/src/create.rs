use std::{
    fs,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail};
use clap::Args;
use dialoguer::{Confirm, Input, theme::ColorfulTheme};
use walkdir::WalkDir;

use crate::{
    config::{Config, Package, Swift},
    util,
};

#[derive(Args, Debug, Default)]
pub struct CreateArgs {
    /// Application display name
    #[arg(long)]
    pub name: Option<String>,

    /// Directory to create the project in
    #[arg(long)]
    pub directory: Option<PathBuf>,

    /// Bundle identifier used for Apple platforms
    #[arg(long)]
    pub bundle_identifier: Option<String>,

    /// Accept defaults without confirmation
    #[arg(short, long)]
    pub yes: bool,
}

pub fn run(args: CreateArgs) -> Result<()> {
    let theme = ColorfulTheme::default();

    let display_name = match args.name {
        Some(name) => name,
        None => Input::with_theme(&theme)
            .with_prompt("Application name")
            .default("Water Demo".to_string())
            .interact_text()?,
    };

    let crate_name = util::kebab_case(&display_name);
    let app_name = util::pascal_case(&display_name);

    let bundle_identifier = match args.bundle_identifier {
        Some(id) => id,
        None => Input::with_theme(&theme)
            .with_prompt("Bundle identifier")
            .default(format!("com.example.{crate_name}"))
            .interact_text()?,
    };

    let project_dir = match args.directory {
        Some(dir) => dir,
        None => {
            let default = std::env::current_dir()?.join(&crate_name);
            Input::with_theme(&theme)
                .with_prompt("Project directory")
                .default(default.display().to_string())
                .interact_text()
                .map(PathBuf::from)?
        }
    };

    let project_dir = project_dir;

    util::info(format!("Application: {display_name}"));
    util::info(format!("Crate name: {crate_name}"));
    util::info(format!("Xcode scheme: {app_name}"));
    util::info(format!("Bundle ID: {bundle_identifier}"));
    util::info(format!("Location: {}", project_dir.display()));

    if !args.yes {
        let proceed = Confirm::with_theme(&theme)
            .with_prompt("Create project with these settings?")
            .default(true)
            .interact()?;
        if !proceed {
            util::warn("Cancelled");
            return Ok(());
        }
    }

    prepare_directory(&project_dir)?;
    create_rust_sources(&project_dir, &crate_name, &display_name)?;
    create_xcode_project(&project_dir, &app_name, &crate_name, &bundle_identifier)?;

    let config = Config::new(
        Package {
            name: crate_name.clone(),
            display_name: display_name.clone(),
            bundle_identifier,
        },
        Swift {
            project_path: "apple".to_string(),
            scheme: app_name.clone(),
        },
    );
    config.save(&project_dir)?;

    util::info("✅ Project created");
    util::info(format!(
        "Next steps:\n  cd {}\n  water run",
        project_dir.display()
    ));
    Ok(())
}

fn prepare_directory(project_dir: &Path) -> Result<()> {
    if project_dir.exists() {
        if project_dir.is_file() {
            bail!("{} already exists and is a file", project_dir.display());
        }
        if project_dir.read_dir()?.next().is_some() {
            bail!("{} already exists and is not empty", project_dir.display());
        }
    }

    util::ensure_directory(project_dir)?;
    util::ensure_directory(&project_dir.join("src"))?;
    util::ensure_directory(&project_dir.join("apple"))?;
    Ok(())
}

fn create_rust_sources(project_dir: &Path, crate_name: &str, display_name: &str) -> Result<()> {
    let cargo_toml = project_dir.join("Cargo.toml");
    let cargo_contents = format!(
        r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"
authors = ["WaterUI CLI"]
license = "MIT"

[lib]
crate-type = ["lib", "staticlib", "cdylib"]

[dependencies]
waterui = "0.1"
waterui-ffi = "0.1"
"#
    );
    fs::write(&cargo_toml, cargo_contents)?;

    let lib_rs = project_dir.join("src/lib.rs");
    let lib_contents = format!(
        r#"use waterui::{{
    component::{{layout::stack::vstack, progress::loading}},
    text,
    prelude::layout::padding::EdgeInsets,
    Environment,
    View,
}};

pub fn init() -> Environment {{
    Environment::new()
}}

pub fn main() -> impl View {{
    vstack((
        text!("Hello, {display_name}!"),
        "Edit src/lib.rs and the view will hot reload",
        loading(),
    ))
    .padding_with(EdgeInsets::all(32.0))
}}

waterui_ffi::export!();
"#
    );
    fs::write(&lib_rs, lib_contents)?;

    let gitignore = project_dir.join(".gitignore");
    let gitignore_contents =
        ".DS_Store\ntarget\napple/DerivedData\napple/build\n*.xcworkspace/xcuserdata\n";
    fs::write(&gitignore, gitignore_contents)?;

    Ok(())
}

fn create_xcode_project(
    project_dir: &Path,
    app_name: &str,
    crate_name: &str,
    bundle_identifier: &str,
) -> Result<()> {
    let template_root = util::workspace_root().join("demo/apple");
    if !template_root.exists() {
        bail!(
            "Xcode template not found at {}. Ensure you are running the CLI from the WaterUI workspace.",
            template_root.display()
        );
    }

    let apple_root = project_dir.join("apple");
    util::ensure_directory(&apple_root)?;
    let lib_name = crate_name.replace('-', "_");

    copy_swift_backend(&apple_root.join("WaterUI"))?;
    write_swift_sources(&apple_root, app_name)?;
    write_build_script(&apple_root, &lib_name)?;
    write_xcode_project(
        &apple_root,
        &template_root,
        app_name,
        &lib_name,
        bundle_identifier,
    )?;

    Ok(())
}

fn write_swift_sources(root: &Path, app_name: &str) -> Result<()> {
    let app_dir = root.join(app_name);
    util::ensure_directory(&app_dir)?;

    let app_file = app_dir.join(format!("{app_name}App.swift"));
    let app_contents = format!(
        r#"import SwiftUI

@main
struct {app_name}App: App {{
    var body: some Scene {{
        WindowGroup {{
            ContentView()
        }}
    }}
}}
"#
    );
    fs::write(app_file, app_contents)?;

    let content_view = app_dir.join("ContentView.swift");
    let content_contents = format!(
        r#"import SwiftUI
import WaterUI

struct ContentView: View {{
    var body: some View {{
        WaterUI.App()
    }}
}}
"#
    );
    fs::write(content_view, content_contents)?;

    let entitlements = app_dir.join(format!("{app_name}.entitlements"));
    let entitlement_contents = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict/>\n</plist>\n";
    fs::write(entitlements, entitlement_contents)?;

    Ok(())
}

fn write_build_script(root: &Path, lib_name: &str) -> Result<()> {
    let script_path = root.join("build-rust.sh");
    let script_contents = format!(
        r#"#!/bin/bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${{BASH_SOURCE[0]}}" )" && pwd )"
PROJECT_ROOT="$( cd "${{SCRIPT_DIR}}/.." && pwd )"

LIB_NAME="{lib_name}"
TARGET_DIR="$PROJECT_ROOT/target"

if [ -z "$CONFIGURATION" ]; then
    CONFIGURATION="Debug"
fi

if [ "$PLATFORM_NAME" = "iphonesimulator" ]; then
    if [ "$ARCHS" = "x86_64" ]; then
        RUST_TARGET="x86_64-apple-ios"
    else
        RUST_TARGET="aarch64-apple-ios-sim"
    fi
elif [ "$PLATFORM_NAME" = "iphoneos" ]; then
    RUST_TARGET="aarch64-apple-ios"
elif [ "$PLATFORM_NAME" = "macosx" ]; then
    if [ "$ARCHS" = "x86_64" ]; then
        RUST_TARGET="x86_64-apple-darwin"
    else
        RUST_TARGET="aarch64-apple-darwin"
    fi
elif [ "$PLATFORM_NAME" = "xrsimulator" ]; then
    RUST_TARGET="aarch64-apple-ios-sim"
elif [ "$PLATFORM_NAME" = "xros" ]; then
    RUST_TARGET="aarch64-apple-ios"
else
    if [ "$(uname -m)" = "x86_64" ]; then
        RUST_TARGET="x86_64-apple-darwin"
    else
        RUST_TARGET="aarch64-apple-darwin"
    fi
fi

if [ "$CONFIGURATION" = "Release" ]; then
    PROFILE="release"
    CARGO_ARGS="--release"
else
    PROFILE="debug"
    CARGO_ARGS=""
fi

export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
    echo "error: cargo not found. Install Rust from https://rustup.rs"
    exit 1
fi

rustup target add "$RUST_TARGET" >/dev/null 2>&1 || true

cd "$PROJECT_ROOT"
cargo build --target "$RUST_TARGET" $CARGO_ARGS

OUTPUT_DIR="$BUILT_PRODUCTS_DIR"
if [ -z "$OUTPUT_DIR" ]; then
    OUTPUT_DIR="$SCRIPT_DIR/build"
fi
mkdir -p "$OUTPUT_DIR"

RUST_LIB="$TARGET_DIR/$RUST_TARGET/$PROFILE/lib{lib_name}.a"
if [ ! -f "$RUST_LIB" ]; then
    echo "error: Rust library not found at $RUST_LIB"
    exit 1
fi

cp "$RUST_LIB" "$OUTPUT_DIR/lib{lib_name}.a"
echo "RUST_LIBRARY_PATH=$RUST_LIB" > "$SCRIPT_DIR/rust_build_info.xcconfig"
"#
    );
    fs::write(&script_path, script_contents)?;
    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&script_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms)?;
    }

    let xcconfig = root.join("rust_build_info.xcconfig");
    fs::write(xcconfig, "RUST_LIBRARY_PATH=\n")?;

    Ok(())
}

fn write_xcode_project(
    apple_root: &Path,
    template_root: &Path,
    app_name: &str,
    lib_name: &str,
    bundle_identifier: &str,
) -> Result<()> {
    let template = template_root.join("demo.xcodeproj/project.pbxproj");
    let mut pbx = fs::read_to_string(&template)
        .with_context(|| format!("failed to read template project {}", template.display()))?;

    pbx = pbx.replace("demo", app_name);
    pbx = pbx.replace(&format!("lib{app_name}.a"), &format!("lib{lib_name}.a"));
    pbx = pbx.replace("../../backends/swift", "WaterUI");
    pbx = pbx.replace(&format!("waterui.{app_name}"), bundle_identifier);
    pbx = pbx.replace("DEVELOPMENT_TEAM = 4AZ53N9R83;", "DEVELOPMENT_TEAM = \"\";");

    let script_marker = "shellScript = \"";
    if let Some(start) = pbx.find(script_marker) {
        let after = start + script_marker.len();
        if let Some(end_rel) = pbx[after..].find("\";\n") {
            let end = after + end_rel + "\";\n".len();
            pbx.replace_range(
                start..end,
                "shellScript = \"bash \\\"$PROJECT_DIR/build-rust.sh\\\"\";\n",
            );
        }
    }

    let project_dir = apple_root.join(format!("{app_name}.xcodeproj"));
    util::ensure_directory(&project_dir)?;
    let pbx_path = project_dir.join("project.pbxproj");
    fs::write(&pbx_path, pbx)?;

    let workspace_template =
        template_root.join("demo.xcodeproj/project.xcworkspace/contents.xcworkspacedata");
    let workspace_contents = fs::read_to_string(&workspace_template)?;
    let workspace_dir = project_dir.join("project.xcworkspace");
    util::ensure_directory(&workspace_dir)?;
    fs::write(
        workspace_dir.join("contents.xcworkspacedata"),
        workspace_contents,
    )?;

    Ok(())
}

fn copy_swift_backend(destination: &Path) -> Result<()> {
    let source_root = util::workspace_root().join("backends/swift");
    if !source_root.exists() {
        bail!(
            "Swift backend sources not found at {}. Ensure you are running the CLI from the WaterUI workspace.",
            source_root.display()
        );
    }

    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    util::ensure_directory(destination)?;

    for entry in WalkDir::new(&source_root).into_iter() {
        let entry = entry?;
        let relative = match entry.path().strip_prefix(&source_root) {
            Ok(rel) if rel.as_os_str().is_empty() => continue,
            Ok(rel) => rel,
            Err(_) => continue,
        };

        if should_skip(relative) {
            continue;
        }

        let target_path = destination.join(relative);
        if entry.file_type().is_dir() {
            util::ensure_directory(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                util::ensure_directory(parent)?;
            }
            fs::copy(entry.path(), &target_path)
                .with_context(|| format!("failed to copy {}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn should_skip(path: &Path) -> bool {
    path.components().any(|component| match component {
        std::path::Component::Normal(name) => {
            matches!(
                name.to_str(),
                Some(".git") | Some(".build") | Some(".swiftpm") | Some(".DS_Store")
            )
        }
        _ => false,
    })
}
