use crate::{output, util};
use clap::Args;
use color_eyre::eyre::{Context, Result, eyre};
use console::style;
use dialoguer::Confirm;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Args, Debug, Default)]
pub struct CleanArgs {
    /// Skip confirmation prompt
    #[arg(short = 'y', long)]
    pub yes: bool,
}

pub fn run(args: CleanArgs) -> Result<()> {
    let format = output::global_output_format();
    let is_json = format.is_json();
    let auto_confirm = args.yes || is_json;

    let workspace = util::workspace_root();
    let actions = build_actions(&workspace)?;

    if actions.is_empty() {
        if is_json {
            let report = CleanReport {
                status: CleanStatus::Noop,
                workspace: workspace.display().to_string(),
                actions: Vec::new(),
                errors: Vec::new(),
            };
            output::emit_json(&report)?;
        } else {
            println!("{}", style("Nothing to clean.").green());
        }
        return Ok(());
    }

    if !auto_confirm {
        println!(
            "{}",
            style("The following cleanup actions will be performed:").bold()
        );
        for action in &actions {
            println!("  • {}", action.description);
        }
        let proceed = Confirm::new()
            .with_prompt("Continue?")
            .default(false)
            .interact()
            .context("Unable to read confirmation response")?;
        if !proceed {
            println!("{}", style("Cleanup aborted.").yellow());
            return Ok(());
        }
    }

    if !is_json {
        println!("{}", style("Starting cleanup…").bold());
    }

    let mut errors = Vec::new();
    let mut reports = Vec::new();

    for action in actions {
        let description = action.description.clone();
        match execute_action(&action) {
            Ok(ActionResult::Removed(detail)) => {
                if !is_json {
                    println!(
                        "  {} {}",
                        style("[ok]").green(),
                        format_detail(&description, detail.as_deref())
                    );
                }
                if is_json {
                    reports.push(CleanActionReport {
                        description,
                        result: CleanActionResult::Removed,
                        detail,
                        error: None,
                    });
                }
            }
            Ok(ActionResult::Skipped(reason)) => {
                if !is_json {
                    println!(
                        "  {} {}",
                        style("[skip]").yellow(),
                        format_detail(&description, Some(reason))
                    );
                }
                if is_json {
                    reports.push(CleanActionReport {
                        description,
                        result: CleanActionResult::Skipped,
                        detail: Some(reason.to_string()),
                        error: None,
                    });
                }
            }
            Ok(ActionResult::Done) => {
                if !is_json {
                    println!("  {} {}", style("[ok]").green(), description);
                }
                if is_json {
                    reports.push(CleanActionReport {
                        description,
                        result: CleanActionResult::Done,
                        detail: None,
                        error: None,
                    });
                }
            }
            Err(err) => {
                let message = err.to_string();
                if !is_json {
                    println!("  {} {}", style("[err]").red(), description);
                }
                errors.push(err);
                if is_json {
                    reports.push(CleanActionReport {
                        description,
                        result: CleanActionResult::Error,
                        detail: None,
                        error: Some(message),
                    });
                }
            }
        }
    }

    let success = errors.is_empty();

    if is_json {
        let status = if success {
            CleanStatus::Ok
        } else {
            CleanStatus::Error
        };
        let error_messages: Vec<String> = errors.iter().map(|err| err.to_string()).collect();
        let report = CleanReport {
            status,
            workspace: workspace.display().to_string(),
            actions: reports,
            errors: error_messages,
        };
        output::emit_json(&report)?;
    } else if success {
        println!("{}", style("Cleanup complete.").green().bold());
    } else {
        println!(
            "{}",
            style(format!(
                "Cleanup finished with {} error(s). See details below:",
                errors.len()
            ))
            .red()
            .bold()
        );
        for err in &errors {
            eprintln!("    - {err}");
        }
    }

    if success {
        Ok(())
    } else {
        Err(eyre!("One or more cleanup steps failed"))
    }
}

fn build_actions(workspace: &Path) -> Result<Vec<Action>> {
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

    Ok(actions)
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
                .with_context(|| format!("Failed to execute `{}`", program))?;

            if status.success() {
                Ok(ActionResult::Done)
            } else {
                Err(eyre!(format!(
                    "`{}` exited with status {}",
                    program,
                    status
                        .code()
                        .map(|code| code.to_string())
                        .unwrap_or_else(|| "signal".to_string())
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

fn format_detail(base: &str, detail: Option<&str>) -> String {
    match detail {
        Some(detail) => format!("{base} ({detail})"),
        None => base.to_string(),
    }
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

    fn remove_dir(description: String, path: PathBuf) -> Self {
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

#[derive(Serialize)]
struct CleanReport {
    status: CleanStatus,
    workspace: String,
    actions: Vec<CleanActionReport>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CleanStatus {
    Ok,
    Error,
    Noop,
}

#[derive(Serialize)]
struct CleanActionReport {
    description: String,
    result: CleanActionResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum CleanActionResult {
    Done,
    Removed,
    Skipped,
    Error,
}
