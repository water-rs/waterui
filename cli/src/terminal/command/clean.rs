use clap::Args;
use color_eyre::eyre::{Context, Result, eyre};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use waterui_cli::output;

use crate::util;

#[derive(Args, Debug, Default, Clone)]
pub struct CleanArgs {
    /// Skip confirmation prompt
    #[arg(short = 'y', long)]
    pub yes: bool,
}

/// Remove build artifacts such as `target`, Gradle caches, and other generated files.
///
/// # Errors
/// Returns an error if any cleanup step fails unexpectedly.
#[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
pub fn run(args: CleanArgs) -> CleanReport {
    let workspace = util::workspace_root();
    let actions = build_actions(&workspace);

    if actions.is_empty() {
        return CleanReport {
            status: CleanStatus::Noop,
            workspace: workspace.display().to_string(),
            actions: Vec::new(),
            errors: Vec::new(),
        };
    }

    let auto_confirm = args.yes || output::global_output_format().is_json();
    if !auto_confirm {
        let pending: Vec<CleanActionReport> = actions
            .into_iter()
            .map(|action| CleanActionReport {
                description: action.description,
                result: CleanActionResult::Pending,
                detail: None,
                error: None,
            })
            .collect();
        return CleanReport {
            status: CleanStatus::PendingConfirmation,
            workspace: workspace.display().to_string(),
            actions: pending,
            errors: Vec::new(),
        };
    }

    let mut reports = Vec::new();
    let mut errors = Vec::new();

    for action in actions {
        let description = action.description.clone();
        match execute_action(&action) {
            Ok(ActionResult::Removed(detail)) => {
                reports.push(CleanActionReport {
                    description,
                    result: CleanActionResult::Removed,
                    detail,
                    error: None,
                });
            }
            Ok(ActionResult::Skipped(reason)) => {
                reports.push(CleanActionReport {
                    description,
                    result: CleanActionResult::Skipped,
                    detail: Some(reason.to_string()),
                    error: None,
                });
            }
            Ok(ActionResult::Done) => {
                reports.push(CleanActionReport {
                    description,
                    result: CleanActionResult::Done,
                    detail: None,
                    error: None,
                });
            }
            Err(err) => {
                let message = err.to_string();
                errors.push(message.clone());
                reports.push(CleanActionReport {
                    description,
                    result: CleanActionResult::Error,
                    detail: None,
                    error: Some(message),
                });
            }
        }
    }

    let status = if errors.is_empty() {
        CleanStatus::Ok
    } else {
        CleanStatus::Error
    };

    CleanReport {
        status,
        workspace: workspace.display().to_string(),
        actions: reports,
        errors,
    }
}

fn build_actions(workspace: &Path) -> Vec<Action> {
    let mut actions = Vec::new();

    actions.push(Action::command(
        format!("Run `cargo clean` in {}", workspace.display()),
        "cargo",
        vec!["clean".into()],
        Some(workspace.to_path_buf()),
    ));

    let mut directories: HashSet<PathBuf> = HashSet::new();

    let android_dirs = [
        workspace.join("backends/android/build"),
        workspace.join("backends/android/.gradle"),
        workspace.join("backends/android/runtime/build"),
        workspace.join("backends/android/runtime/.cxx"),
    ];
    directories.extend(android_dirs);

    let apple_dirs = [
        workspace.join("demo/apple/build"),
        workspace.join("demo/apple/DerivedData"),
        workspace.join("demo/apple/.swiftpm"),
    ];
    directories.extend(apple_dirs);

    if let Some(home) = home::home_dir() {
        let gradle_dirs = [
            home.join(".gradle/caches"),
            home.join(".gradle/daemon"),
            home.join(".gradle/native"),
            home.join(".gradle/buildOutputCleanup"),
            home.join(".gradle/notifications"),
        ];
        directories.extend(gradle_dirs);

        if cfg!(target_os = "macos") {
            directories.insert(home.join("Library/Developer/Xcode/DerivedData"));
        }
    }

    for dir in directories {
        actions.push(Action::remove_dir(format!("Remove {}", dir.display()), dir));
    }

    actions
}

fn execute_action(action: &Action) -> Result<ActionResult> {
    match &action.kind {
        ActionKind::Command {
            program,
            args,
            workdir,
        } => {
            let mut command = Command::new(program);
            command.args(args);
            if let Some(dir) = workdir {
                command.current_dir(dir);
            }

            let status = command
                .status()
                .with_context(|| format!("Failed to execute `{program}`"))?;

            if status.success() {
                Ok(ActionResult::Done)
            } else {
                Err(eyre!(format!(
                    "`{}` exited with status {}",
                    program,
                    status
                        .code()
                        .map_or_else(|| "signal".to_string(), |code| code.to_string())
                )))
            }
        }
        ActionKind::RemoveDir(path) => {
            if !path.exists() {
                return Ok(ActionResult::Skipped("nothing to remove"));
            }

            remove_path(path)?;
            Ok(ActionResult::Removed(None))
        }
    }
}

fn remove_path(path: &Path) -> Result<()> {
    if path.is_file() {
        fs::remove_file(path).with_context(|| format!("Failed to remove {}", path.display()))?;
    } else {
        fs::remove_dir_all(path).with_context(|| format!("Failed to remove {}", path.display()))?;
    }
    Ok(())
}

struct Action {
    description: String,
    kind: ActionKind,
}

impl Action {
    fn command(
        description: String,
        program: &str,
        args: Vec<String>,
        workdir: Option<PathBuf>,
    ) -> Self {
        Self {
            description,
            kind: ActionKind::Command {
                program: program.to_string(),
                args,
                workdir,
            },
        }
    }

    const fn remove_dir(description: String, path: PathBuf) -> Self {
        Self {
            description,
            kind: ActionKind::RemoveDir(path),
        }
    }
}

enum ActionKind {
    Command {
        program: String,
        args: Vec<String>,
        workdir: Option<PathBuf>,
    },
    RemoveDir(PathBuf),
}

enum ActionResult {
    Done,
    Removed(Option<String>),
    Skipped(&'static str),
}

#[derive(Debug, Serialize)]
pub struct CleanReport {
    pub status: CleanStatus,
    pub workspace: String,
    pub actions: Vec<CleanActionReport>,
    pub errors: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanStatus {
    Ok,
    Error,
    Noop,
    PendingConfirmation,
}

#[derive(Debug, Serialize)]
pub struct CleanActionReport {
    pub description: String,
    pub result: CleanActionResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanActionResult {
    Done,
    Removed,
    Skipped,
    Error,
    Pending,
}
