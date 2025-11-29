use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, ValueEnum};
use color_eyre::eyre::{Context, Result, bail, eyre};
use dialoguer::{Confirm, Input, MultiSelect, theme::ColorfulTheme};
use heck::{ToKebabCase, ToUpperCamelCase};
use tracing::warn;

use crate::util;
use serde::Serialize;
use waterui_cli::{
    WATERUI_ANDROID_BACKEND_VERSION, WATERUI_SWIFT_BACKEND_VERSION, WATERUI_VERSION, output,
    project::{Android, Config, Package, Swift, Web},
};

/// Validated WaterUI repository path for dev mode.
#[derive(Debug, Clone)]
pub struct ValidatedWaterUIPath {
    /// The root path to the WaterUI repository.
    pub root: PathBuf,
    /// Path to the Android backend within the repository.
    pub android_backend: PathBuf,
    /// Path to the Apple backend within the repository.
    pub apple_backend: PathBuf,
}

pub const DEFAULT_WATERUI_FFI_VERSION: &str = "0.1.0";

pub mod android;
pub mod rust;
pub mod swift;
pub mod template;
pub mod web;

pub const SWIFT_BACKEND_GIT_URL: &str = "https://github.com/water-rs/apple-backend.git";

pub const SWIFT_TAG_PREFIX: &str = "apple-backend-v";

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

    /// Apple Development Team ID
    #[arg(long)]
    pub team_id: Option<String>,

    /// Use the development version of `WaterUI` from GitHub
    #[arg(long)]
    pub dev: bool,

    /// Path to local WaterUI repository for dev mode (contains backends/android and backends/apple as submodules)
    #[arg(long)]
    pub waterui_path: Option<PathBuf>,

    /// Accept defaults without confirmation
    #[arg(short, long)]
    pub yes: bool,

    /// Backends to include (android, web, apple). Can be provided multiple times or as a comma-separated list.
    #[arg(long = "backend", value_enum, value_delimiter = ',', num_args = 1..)]
    pub backends: Vec<BackendChoice>,
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum BackendChoice {
    #[clap(name = "web")]
    Web,
    #[clap(name = "apple")]
    Apple,
    #[clap(name = "android")]
    Android,
}

impl BackendChoice {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Web => "Web",
            Self::Apple => "Apple",
            Self::Android => "Android",
        }
    }
}

/// Interactive entry point for `water create`.
///
/// # Errors
/// Returns an error if user input cannot be read, dependencies cannot be resolved, or
/// template files fail to write.
///
/// # Panics
/// Panics if required embedded templates are missing; this indicates a build-time bug.
#[allow(clippy::too_many_lines)]
pub fn run(args: CreateArgs) -> Result<CreateReport> {
    let is_json = output::global_output_format().is_json();
    if is_json && !args.yes {
        bail!(
            "JSON output requires --yes to avoid interactive prompts. Re-run with --yes or provide --backend, --name, and related flags."
        );
    }

    let theme = ColorfulTheme::default();

    // Handle local WaterUI path for dev mode
    let validated_waterui_path = if args.dev {
        let waterui_path = if let Some(path) = args.waterui_path.clone() {
            path
        } else if args.yes {
            // In non-interactive mode, dev without path falls back to git dependencies
            PathBuf::new()
        } else {
            use crate::ui;
            ui::info("Dev mode requires a local WaterUI repository path for instant feedback.");
            ui::info(
                "The repository should have backends/android and backends/apple as submodules.",
            );
            ui::newline();

            let default_path = std::env::current_dir().expect("failed to get current directory");

            Input::with_theme(&theme)
                .with_prompt("WaterUI repository path")
                .default(default_path.display().to_string())
                .interact_text()
                .map(PathBuf::from)?
        };

        if waterui_path.as_os_str().is_empty() {
            None
        } else {
            Some(validate_waterui_path(&waterui_path)?)
        }
    } else {
        None
    };

    let deps = resolve_dependencies_with_path(args.dev, validated_waterui_path.as_ref())?;

    let display_name = if let Some(name) = args.name.clone() {
        name
    } else if args.yes {
        "Water Demo".to_string()
    } else {
        Input::with_theme(&theme)
            .with_prompt("Application name")
            .default("Water Demo".to_string())
            .interact_text()?
    };

    let default_author = std::process::Command::new("git")
        .arg("config")
        .arg("user.name")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let author = if args.yes {
        default_author
    } else {
        Input::with_theme(&theme)
            .with_prompt("Author")
            .default(default_author)
            .interact_text()?
    };

    let crate_name = {
        let generated = display_name.to_kebab_case();
        if generated.is_empty() {
            "waterui-app".to_string()
        } else {
            generated
        }
    };
    let app_name = {
        let generated = display_name.to_upper_camel_case();
        if generated.is_empty() {
            "WaterUIApp".to_string()
        } else {
            generated
        }
    };

    let default_bundle_identifier = format!("com.waterui.{crate_name}");
    let bundle_identifier = if let Some(id) = args.bundle_identifier {
        id
    } else if args.yes {
        default_bundle_identifier
    } else {
        Input::with_theme(&theme)
            .with_prompt("Bundle identifier")
            .default(default_bundle_identifier)
            .interact_text()?
    };

    let project_dir = if let Some(dir) = args.directory {
        dir
    } else {
        let default = std::env::current_dir()?.join(&crate_name);
        if args.yes {
            default
        } else {
            Input::with_theme(&theme)
                .with_prompt("Project directory")
                .default(default.display().to_string())
                .interact_text()
                .map(PathBuf::from)?
        }
    };

    let selected_backends: Vec<BackendChoice> = if args.backends.is_empty() {
        let available_backends = [
            BackendChoice::Web,
            BackendChoice::Apple,
            BackendChoice::Android,
        ];
        let defaults = vec![true; available_backends.len()];
        let labels: Vec<String> = available_backends
            .iter()
            .map(|choice| choice.label())
            .map(str::to_string)
            .collect();
        let selected_indices = if args.yes {
            (0..available_backends.len()).collect()
        } else {
            MultiSelect::with_theme(&theme)
                .with_prompt("Choose project backends (space to select, enter to confirm)")
                .items(&labels)
                .defaults(&defaults)
                .interact()?
        };

        if selected_indices.is_empty() {
            warn!("No backends selected, aborting.");
            return Ok(build_report(
                CreateStatus::Cancelled,
                &project_dir,
                &crate_name,
                &display_name,
                &bundle_identifier,
                &[],
                args.dev,
            ));
        }

        selected_indices
            .iter()
            .map(|&index| available_backends[index])
            .collect()
    } else {
        args.backends.clone()
    };

    if !is_json {
        use crate::ui;
        ui::section("Project Configuration");
        ui::kv("Application", &display_name);
        ui::kv("Author", &author);
        ui::kv("Crate name", &crate_name);
        if selected_backends.contains(&BackendChoice::Apple) {
            ui::kv("Xcode scheme", &crate_name);
        }
        ui::kv("Bundle ID", &bundle_identifier);
        let backend_list = selected_backends
            .iter()
            .map(|choice| choice.label())
            .collect::<Vec<_>>()
            .join(", ");
        ui::kv("Backends", &backend_list);
        ui::kv("Location", project_dir.display().to_string());
        ui::newline();
    }

    if !args.yes {
        let proceed = Confirm::with_theme(&theme)
            .with_prompt("Create project with these settings?")
            .default(true)
            .interact()?;
        if !proceed {
            warn!("Cancelled");
            return Ok(build_report(
                CreateStatus::Cancelled,
                &project_dir,
                &crate_name,
                &display_name,
                &bundle_identifier,
                &selected_backends,
                args.dev,
            ));
        }
    }

    prepare_directory(&project_dir)?;

    rust::create_rust_sources(&project_dir, &crate_name, &author, &display_name, &deps)?;

    let mut config = Config::new(Package {
        name: crate_name.clone(),
        display_name: display_name.clone(),
        bundle_identifier: bundle_identifier.clone(),
        author,
    });
    config.dev_dependencies = args.dev;
    if let Some(ref validated_path) = validated_waterui_path {
        config.waterui_path = Some(validated_path.root.display().to_string());
    }

    let mut web_enabled = false;
    for backend in &selected_backends {
        match backend {
            BackendChoice::Web => {
                web::create_web_assets(&project_dir, &display_name)?;
                config.backends.web = Some(Web {
                    project_path: "web".to_string(),
                    version: None,
                    dev: args.dev,
                    ffi_version: Some(DEFAULT_WATERUI_FFI_VERSION.to_string()),
                });
                web_enabled = true;
            }
            BackendChoice::Android => {
                android::create_android_project(
                    &project_dir,
                    &app_name,
                    &crate_name,
                    &bundle_identifier,
                    args.dev,
                    deps.local_waterui_path.as_ref(),
                )?;
                config.backends.android = Some(Android {
                    project_path: "android".to_string(),
                    version: if args.dev || WATERUI_ANDROID_BACKEND_VERSION.is_empty() {
                        None
                    } else {
                        Some(WATERUI_ANDROID_BACKEND_VERSION.to_string())
                    },
                    dev: args.dev,
                    ffi_version: Some(DEFAULT_WATERUI_FFI_VERSION.to_string()),
                });
            }
            BackendChoice::Apple => {
                swift::create_xcode_project(
                    &project_dir,
                    &app_name,
                    &display_name,
                    &crate_name,
                    &bundle_identifier,
                    &deps.swift,
                )?;
                let (version, branch, revision, local_path) = match &deps.swift {
                    SwiftDependency::Git {
                        version,
                        branch,
                        revision,
                    } => (version.clone(), branch.clone(), revision.clone(), None),
                    SwiftDependency::Local { path } => {
                        (None, None, None, Some(path.display().to_string()))
                    }
                };
                config.backends.swift = Some(Swift {
                    project_path: "apple".to_string(),
                    scheme: crate_name.clone(),
                    project_file: Some(format!("{app_name}.xcodeproj")),
                    version,
                    branch,
                    revision,
                    local_path,
                    dev: args.dev,
                    ffi_version: Some(DEFAULT_WATERUI_FFI_VERSION.to_string()),
                });
            }
        }
    }

    if web_enabled && !config.hot_reload.watch.iter().any(|path| path == "web") {
        config.hot_reload.watch.push("web".to_string());
    }

    config.save(&project_dir)?;

    if !is_json {
        use crate::ui;
        ui::success("Project created successfully!");
        ui::newline();
        let current_dir = std::env::current_dir()?;
        let display_path = project_dir
            .strip_prefix(current_dir)
            .unwrap_or(&project_dir);
        ui::section("Next Steps");
        ui::step(format!("cd {}", display_path.display()));
        ui::step("water run");
    }

    let report = build_report(
        CreateStatus::Created,
        &project_dir,
        &crate_name,
        &display_name,
        &bundle_identifier,
        &selected_backends,
        args.dev,
    );

    // if which::which("git").is_ok() {
    //     std::process::Command::new("git")
    //         .arg("init")
    //         .current_dir(&project_dir)
    //         .output()?;
    //     info!("✅ Git repository initialized");
    // }

    Ok(report)
}

#[derive(Debug, Serialize)]
pub struct CreateReport {
    pub status: CreateStatus,
    pub project_dir: String,
    pub crate_name: String,
    pub display_name: String,
    pub bundle_identifier: String,
    pub backends: Vec<String>,
    pub using_dev_dependencies: bool,
    pub config_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CreateStatus {
    Created,
    Cancelled,
}

fn build_report(
    status: CreateStatus,
    project_dir: &Path,
    crate_name: &str,
    display_name: &str,
    bundle_identifier: &str,
    backends: &[BackendChoice],
    using_dev_dependencies: bool,
) -> CreateReport {
    CreateReport {
        status,
        project_dir: project_dir.display().to_string(),
        crate_name: crate_name.to_string(),
        display_name: display_name.to_string(),
        bundle_identifier: bundle_identifier.to_string(),
        backends: backends
            .iter()
            .map(|backend| backend.label().to_string())
            .collect(),
        using_dev_dependencies,
        config_path: Config::path(project_dir).display().to_string(),
    }
}

#[derive(Debug, Clone)]
pub struct ProjectDependencies {
    rust_toml: String,
    pub swift: SwiftDependency,
    /// If set, the local path to the WaterUI repository for dev mode.
    pub local_waterui_path: Option<ValidatedWaterUIPath>,
}

#[derive(Debug, Clone)]
pub enum SwiftDependency {
    Git {
        version: Option<String>,
        branch: Option<String>,
        revision: Option<String>,
    },
    Local {
        path: PathBuf,
    },
}

/// Validate that the given path is a valid WaterUI repository with android and apple backends.
///
/// # Errors
/// Returns an error if the path doesn't exist or doesn't contain the required backend directories.
pub fn validate_waterui_path(path: &Path) -> Result<ValidatedWaterUIPath> {
    let path = path.canonicalize().with_context(|| {
        format!(
            "Failed to resolve WaterUI repository path: {}",
            path.display()
        )
    })?;

    if !path.exists() {
        bail!("WaterUI repository path does not exist: {}", path.display());
    }

    if !path.is_dir() {
        bail!(
            "WaterUI repository path is not a directory: {}",
            path.display()
        );
    }

    // Check for backends/android
    let android_backend = path.join("backends/android");
    if !android_backend.exists() || !android_backend.is_dir() {
        bail!(
            "Invalid WaterUI repository: missing backends/android directory at {}.\n\
             Make sure git submodules are initialized with: git submodule update --init --recursive",
            path.display()
        );
    }

    // Check for backends/apple
    let apple_backend = path.join("backends/apple");
    if !apple_backend.exists() || !apple_backend.is_dir() {
        bail!(
            "Invalid WaterUI repository: missing backends/apple directory at {}.\n\
             Make sure git submodules are initialized with: git submodule update --init --recursive",
            path.display()
        );
    }

    // Verify that the android backend has essential files
    let android_build_gradle = android_backend.join("build.gradle.kts");
    if !android_build_gradle.exists() {
        bail!(
            "Invalid Android backend: missing build.gradle.kts at {}.\n\
             The backends/android submodule may not be properly initialized.",
            android_backend.display()
        );
    }

    // Verify that the apple backend has essential files
    let apple_package_swift = apple_backend.join("Package.swift");
    if !apple_package_swift.exists() {
        bail!(
            "Invalid Apple backend: missing Package.swift at {}.\n\
             The backends/apple submodule may not be properly initialized.",
            apple_backend.display()
        );
    }

    Ok(ValidatedWaterUIPath {
        root: path,
        android_backend,
        apple_backend,
    })
}

/// Resolve the template dependencies with an optional local WaterUI path for dev mode.
///
/// # Errors
/// Returns an error if the crates index cannot be queried or if the local path is invalid.
#[allow(clippy::const_is_empty)]
pub fn resolve_dependencies_with_path(
    dev: bool,
    waterui_path: Option<&ValidatedWaterUIPath>,
) -> Result<ProjectDependencies> {
    if dev {
        if let Some(validated_path) = waterui_path {
            // Local dev mode - use path dependencies
            let root_path = validated_path.root.display();
            let rust_toml = format!(
                r#"waterui = {{ path = "{root_path}" }}
waterui-ffi = {{ path = "{root_path}/ffi" }}"#
            );
            return Ok(ProjectDependencies {
                rust_toml,
                swift: SwiftDependency::Local {
                    path: validated_path.apple_backend.clone(),
                },
                local_waterui_path: Some(validated_path.clone()),
            });
        }

        // Remote dev mode - use git dependencies
        let branch = "dev";
        let revision = fetch_swift_branch_head(branch)?;
        let rust_toml =
            r#"waterui = { git = "https://github.com/water-rs/waterui", branch = "dev" }
waterui-ffi = { git = "https://github.com/water-rs/waterui", branch = "dev" }"#
                .to_string();
        return Ok(ProjectDependencies {
            rust_toml,
            swift: SwiftDependency::Git {
                version: None,
                branch: Some(branch.to_string()),
                revision: Some(revision),
            },
            local_waterui_path: None,
        });
    }

    let waterui_version = WATERUI_VERSION;
    if waterui_version.is_empty() {
        bail!("WATERUI_VERSION is not set. This should be set at build time.");
    }

    let rust_toml = format!(
        r#"waterui = "{waterui_version}"
waterui-ffi = "{waterui_version}""#
    );

    let swift_backend_version = WATERUI_SWIFT_BACKEND_VERSION;
    if swift_backend_version.is_empty() {
        bail!("WATERUI_SWIFT_BACKEND_VERSION is not set. This should be set at build time.");
    }

    Ok(ProjectDependencies {
        rust_toml,
        swift: SwiftDependency::Git {
            version: Some(swift_backend_version.to_string()),
            branch: None,
            revision: None,
        },
        local_waterui_path: None,
    })
}

pub fn swift_backend_repo_url() -> String {
    std::env::var("WATERUI_SWIFT_BACKEND_URL").unwrap_or_else(|_| SWIFT_BACKEND_GIT_URL.to_string())
}

pub fn fetch_swift_branch_head(branch: &str) -> Result<String> {
    let repo_url = swift_backend_repo_url();
    git_ls_remote(&repo_url, branch)
}

fn git_ls_remote(repo_url: &str, reference: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["ls-remote", repo_url, reference])
        .output()
        .with_context(|| format!("failed to query '{reference}' from {repo_url}"))?;
    if !output.status.success() {
        bail!(
            "git ls-remote for {repo_url} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let hash = stdout
        .lines()
        .find_map(|line| line.split('\t').next().map(str::to_string))
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| eyre!("reference '{reference}' not found in {repo_url}"))?;
    Ok(hash)
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
