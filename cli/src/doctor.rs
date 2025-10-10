use anyhow::{Result, anyhow};
use clap::Args;
use console::style;
use core::time::Duration;
use dialoguer::Confirm;
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashSet;
use std::process::Command;

#[derive(Args, Debug, Default)]
pub struct DoctorArgs {
    /// Attempt to fix required issues after running checks
    #[arg(long)]
    pub fix: bool,
}

pub fn run(args: DoctorArgs) -> Result<()> {
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(80));
    pb.set_style(
        ProgressStyle::with_template("{spinner:.blue} {msg}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );

    pb.set_message(style("Preparing environment checks…").dim().to_string());
    pb.tick();

    let include_swift = cfg!(target_os = "macos");
    let total = 2 + usize::from(include_swift);
    let mut sections = Vec::new();
    let mut fixes = Vec::new();
    let mut step = 1;

    pb.set_message(progress_label(step, total, "Rust toolchain"));
    pb.tick();
    let rust = check_rust();
    announce_section(&pb, step, total, rust.section());
    let (rust_section, mut rust_fixes) = rust.into_parts();
    sections.push(rust_section);
    fixes.append(&mut rust_fixes);
    step += 1;

    if include_swift {
        pb.set_message(progress_label(step, total, "Swift toolchain"));
        pb.tick();
        if let Some(swift) = check_swift() {
            announce_section(&pb, step, total, swift.section());
            let (swift_section, mut swift_fixes) = swift.into_parts();
            sections.push(swift_section);
            fixes.append(&mut swift_fixes);
        }
        step += 1;
    }

    pb.set_message(progress_label(step, total, "Android tooling"));
    pb.tick();
    let android = check_android();
    announce_section(&pb, step, total, android.section());
    let (android_section, mut android_fixes) = android.into_parts();
    sections.push(android_section);
    fixes.append(&mut android_fixes);

    pb.finish_and_clear();

    let has_failures = sections.iter().any(|section| section.has_failure());

    println!("{}", style("WaterUI doctor report").bold().underlined());
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            println!();
        }
        for line in section.render() {
            println!("{line}");
        }
    }

    println!();
    println!("{}", style("Doctor check complete.").green());
    println!(
        "{}",
        style("Resolve ⚠ or ✘ entries to keep your toolchain healthy.").dim()
    );

    if has_failures && !args.fix {
        println!(
            "{}",
            style("Tip: run `water doctor --fix` to attempt automatic repairs.").yellow()
        );
    }

    if args.fix {
        apply_fixes(fixes)?;
    }

    Ok(())
}

fn announce_section(pb: &ProgressBar, step: usize, total: usize, section: &Section) {
    let line = format!(
        "{} {}",
        style(format!("[{step}/{total}]")).dim(),
        section.summary_line()
    );
    pb.suspend(|| println!("{line}"));
}

fn progress_label(step: usize, total: usize, message: &str) -> String {
    format!(
        "{} {}",
        style(format!("[{step}/{total}]")).dim(),
        style(message).bold()
    )
}

fn check_rust() -> SectionOutcome {
    let mut outcome = SectionOutcome::new("Rust toolchain");

    outcome.push(check_command(
        "cargo",
        "Install Rust from https://rustup.rs",
    ));
    outcome.push(check_command(
        "rustup",
        "Install Rust from https://rustup.rs",
    ));

    if which::which("rustup").is_ok() {
        outcome.push(Row::info("Installed Rust targets"));

        let output = Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output();

        match output {
            Ok(output) => {
                let installed_targets = String::from_utf8_lossy(&output.stdout);

                if cfg!(target_os = "macos") {
                    outcome.push(Row::info("Apple platforms").with_indent(1));
                    if cfg!(target_arch = "aarch64") {
                        let required = [
                            (
                                "aarch64-apple-darwin",
                                "Required for macOS builds on Apple Silicon",
                            ),
                            (
                                "aarch64-apple-ios-sim",
                                "Required for Apple Silicon iOS Simulator",
                            ),
                        ];
                        for (target, note) in required {
                            outcome.push_outcome(target_row(
                                &installed_targets,
                                target,
                                TargetKind::Required { note: Some(note) },
                            ));
                        }
                        let optional = [
                            ("aarch64-apple-ios", "for deploying to physical iOS devices"),
                            (
                                "x86_64-apple-darwin",
                                "for building macOS binaries for Intel Macs",
                            ),
                            ("x86_64-apple-ios", "for Intel-based iOS Simulator support"),
                        ];
                        for (target, reason) in optional {
                            outcome.push_outcome(target_row(
                                &installed_targets,
                                target,
                                TargetKind::Optional(reason),
                            ));
                        }
                    } else if cfg!(target_arch = "x86_64") {
                        let required = [
                            ("x86_64-apple-darwin", "Required for macOS builds on Intel"),
                            ("x86_64-apple-ios", "Required for Intel iOS Simulator"),
                        ];
                        for (target, note) in required {
                            outcome.push_outcome(target_row(
                                &installed_targets,
                                target,
                                TargetKind::Required { note: Some(note) },
                            ));
                        }
                        let optional = [
                            (
                                "aarch64-apple-darwin",
                                "for building macOS binaries for Apple Silicon",
                            ),
                            (
                                "aarch64-apple-ios",
                                "for deploying to Apple Silicon iOS devices",
                            ),
                            (
                                "aarch64-apple-ios-sim",
                                "for Apple Silicon iOS Simulator compatibility",
                            ),
                        ];
                        for (target, reason) in optional {
                            outcome.push_outcome(target_row(
                                &installed_targets,
                                target,
                                TargetKind::Optional(reason),
                            ));
                        }
                    }
                }

                outcome.push(Row::info("Android targets").with_indent(1));
                if cfg!(target_arch = "aarch64") {
                    outcome.push_outcome(target_row(
                        &installed_targets,
                        "aarch64-linux-android",
                        TargetKind::Required {
                            note: Some("Required for Android emulator on Apple Silicon"),
                        },
                    ));
                    let optional = [
                        ("armv7-linux-androideabi", "for legacy Android (armv7)"),
                        ("i686-linux-android", "for Android x86 emulators"),
                        ("x86_64-linux-android", "for Android x86_64 emulators"),
                    ];
                    for (target, reason) in optional {
                        outcome.push_outcome(target_row(
                            &installed_targets,
                            target,
                            TargetKind::Optional(reason),
                        ));
                    }
                } else if cfg!(target_arch = "x86_64") {
                    outcome.push_outcome(target_row(
                        &installed_targets,
                        "x86_64-linux-android",
                        TargetKind::Required {
                            note: Some("Required for Android emulator on Intel"),
                        },
                    ));
                    let optional = [
                        ("aarch64-linux-android", "for Android devices (arm64)"),
                        ("armv7-linux-androideabi", "for legacy Android (armv7)"),
                        ("i686-linux-android", "for Android x86 emulators"),
                    ];
                    for (target, reason) in optional {
                        outcome.push_outcome(target_row(
                            &installed_targets,
                            target,
                            TargetKind::Optional(reason),
                        ));
                    }
                } else {
                    let optional = [
                        ("aarch64-linux-android", "for Android devices (arm64)"),
                        ("armv7-linux-androideabi", "for legacy Android (armv7)"),
                        ("i686-linux-android", "for Android x86 emulators"),
                        ("x86_64-linux-android", "for Android x86_64 emulators"),
                    ];
                    for (target, reason) in optional {
                        outcome.push_outcome(target_row(
                            &installed_targets,
                            target,
                            TargetKind::Optional(reason),
                        ));
                    }
                }
            }
            Err(err) => outcome.push(
                Row::warn("Could not query installed Rust targets")
                    .with_detail(format!("rustup target list --installed failed: {err}")),
            ),
        }
    }

    outcome
}

fn check_swift() -> Option<SectionOutcome> {
    if cfg!(not(target_os = "macos")) {
        return None;
    }

    let mut outcome = SectionOutcome::new("Swift (macOS)");
    outcome.push(check_command(
        "xcodebuild",
        "Install Xcode and command line tools (xcode-select --install)",
    ));
    outcome.push(check_command(
        "xcrun",
        "Install Xcode and command line tools (xcode-select --install)",
    ));
    Some(outcome)
}

fn check_android() -> SectionOutcome {
    let mut outcome = SectionOutcome::new("Android tooling");
    outcome.push(check_command(
        "adb",
        "Install Android SDK Platform-Tools and add to PATH.",
    ));
    outcome.push(check_command(
        "emulator",
        "Install Android SDK command-line tools and add to PATH.",
    ));
    outcome.push(check_env_var(
        "ANDROID_HOME",
        "Set ANDROID_HOME to your Android SDK path.",
    ));
    outcome.push(check_env_var(
        "ANDROID_NDK_HOME",
        "Set ANDROID_NDK_HOME to your Android NDK path.",
    ));
    outcome.push(check_env_var(
        "JAVA_HOME",
        "Set JAVA_HOME to your JDK path (Java 17 or newer recommended).",
    ));

    let java_version_cmd = if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let java_exe = std::path::Path::new(&java_home).join("bin/java");
        if java_exe.exists() {
            Some(Command::new(java_exe))
        } else {
            None
        }
    } else if which::which("java").is_ok() {
        Some(Command::new("java"))
    } else {
        None
    };

    if let Some(mut cmd) = java_version_cmd {
        let output = cmd.arg("-version").output();
        match output {
            Ok(output) => {
                let version_info = String::from_utf8_lossy(&output.stderr);
                if let Some(line) = version_info.lines().next() {
                    outcome.push(Row::pass("Java detected").with_detail(line.trim().to_string()));
                } else {
                    outcome.push(Row::warn("Could not determine Java version"));
                }
            }
            Err(err) => outcome.push(
                Row::warn("Failed to read Java version")
                    .with_detail(format!("java -version failed: {err}")),
            ),
        }
    } else {
        outcome.push(Row::fail("Java not found in JAVA_HOME or PATH"));
    }

    outcome
}

fn check_command(name: &str, help: &str) -> Row {
    match which::which(name) {
        Ok(path) => Row::pass(format!("Found `{name}`")).with_detail(path.display().to_string()),
        Err(_) => Row::fail(format!("`{name}` not found")).with_detail(help),
    }
}

fn check_env_var(name: &str, help: &str) -> Row {
    match std::env::var(name) {
        Ok(value) => Row::pass(format!("Environment `{name}` set")).with_detail(value),
        Err(_) => Row::fail(format!("Environment `{name}` missing")).with_detail(help),
    }
}

fn target_row(installed: &str, target: &str, kind: TargetKind) -> RowOutcome {
    let present = installed.contains(target);
    match kind {
        TargetKind::Required { note } => {
            if present {
                RowOutcome::new(Row::pass(format!("Rust target `{target}`")).with_indent(2))
            } else {
                let mut detail = format!("Run `rustup target add {target}`");
                let description = if let Some(note) = note {
                    detail = format!("{note}. {detail}");
                    format!("Install Rust target `{target}` ({note})")
                } else {
                    format!("Install Rust target `{target}`")
                };
                let row = Row::fail(format!("Rust target `{target}` missing"))
                    .with_detail(detail)
                    .with_indent(2);
                RowOutcome::with_fix(
                    row,
                    FixSuggestion::new(
                        format!("rust-target-{target}"),
                        description,
                        vec![
                            "rustup".into(),
                            "target".into(),
                            "add".into(),
                            target.to_string(),
                        ],
                    ),
                )
            }
        }
        TargetKind::Optional(reason) => {
            if present {
                RowOutcome::new(
                    Row::pass(format!("Rust target `{target}`"))
                        .with_detail(format!("Optional {reason}"))
                        .with_indent(2),
                )
            } else {
                RowOutcome::new(
                    Row::warn(format!("Rust target `{target}` not installed"))
                        .with_detail(format!(
                            "Optional {reason}. Run `rustup target add {target}` if needed."
                        ))
                        .with_indent(2),
                )
            }
        }
    }
}

enum TargetKind {
    Required { note: Option<&'static str> },
    Optional(&'static str),
}

#[derive(Clone, Copy)]
enum Status {
    Pass,
    Warn,
    Fail,
    Info,
}

struct Row {
    status: Status,
    message: String,
    detail: Option<String>,
    indent: usize,
}

struct RowOutcome {
    row: Row,
    fix: Option<FixSuggestion>,
}

impl RowOutcome {
    fn new(row: Row) -> Self {
        Self { row, fix: None }
    }

    fn with_fix(row: Row, fix: FixSuggestion) -> Self {
        Self {
            row,
            fix: Some(fix),
        }
    }
}

impl Row {
    fn pass(message: impl Into<String>) -> Self {
        Self::new(Status::Pass, message)
    }

    fn warn(message: impl Into<String>) -> Self {
        Self::new(Status::Warn, message)
    }

    fn fail(message: impl Into<String>) -> Self {
        Self::new(Status::Fail, message)
    }

    fn info(message: impl Into<String>) -> Self {
        Self::new(Status::Info, message)
    }

    fn new(status: Status, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            detail: None,
            indent: 0,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn with_indent(mut self, indent: usize) -> Self {
        self.indent = indent;
        self
    }

    fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let indent = "  ".repeat(self.indent + 1);
        let icon = match self.status {
            Status::Pass => format!("{}", style("✔").green()),
            Status::Warn => format!("{}", style("⚠").yellow()),
            Status::Fail => format!("{}", style("✘").red()),
            Status::Info => format!("{}", style("•").cyan()),
        };
        lines.push(format!("{indent}{icon} {}", self.message));
        if let Some(detail) = &self.detail {
            let detail_indent = "  ".repeat(self.indent + 2);
            lines.push(format!("{}{}", detail_indent, style(detail).dim()));
        }
        lines
    }

    fn status(&self) -> Status {
        self.status
    }
}

struct Section {
    title: String,
    rows: Vec<Row>,
}

impl Section {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
        }
    }

    fn push(&mut self, row: Row) {
        self.rows.push(row);
    }

    fn render(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "{} {}",
            style("◈").cyan(),
            style(&self.title).bold()
        ));
        for row in &self.rows {
            lines.extend(row.render());
        }
        lines
    }

    fn summary_line(&self) -> String {
        let icon = match self.overall_status() {
            Status::Pass => style("✔").green(),
            Status::Warn => style("⚠").yellow(),
            Status::Fail => style("✘").red(),
            Status::Info => style("•").cyan(),
        };
        format!("{} {}", icon, style(&self.title).bold())
    }

    fn overall_status(&self) -> Status {
        if self
            .rows
            .iter()
            .any(|row| matches!(row.status(), Status::Fail))
        {
            Status::Fail
        } else if self
            .rows
            .iter()
            .any(|row| matches!(row.status(), Status::Warn))
        {
            Status::Warn
        } else if self
            .rows
            .iter()
            .any(|row| matches!(row.status(), Status::Pass))
        {
            Status::Pass
        } else {
            Status::Info
        }
    }

    fn has_failure(&self) -> bool {
        self.rows
            .iter()
            .any(|row| matches!(row.status(), Status::Fail))
    }
}

struct SectionOutcome {
    section: Section,
    fixes: Vec<FixSuggestion>,
}

impl SectionOutcome {
    fn new(title: impl Into<String>) -> Self {
        Self {
            section: Section::new(title),
            fixes: Vec::new(),
        }
    }

    fn push(&mut self, row: Row) {
        self.section.push(row);
    }

    fn push_outcome(&mut self, outcome: RowOutcome) {
        if let Some(fix) = outcome.fix {
            self.fixes.push(fix);
        }
        self.section.push(outcome.row);
    }

    fn section(&self) -> &Section {
        &self.section
    }

    fn into_parts(self) -> (Section, Vec<FixSuggestion>) {
        (self.section, self.fixes)
    }
}

#[derive(Clone)]
struct FixSuggestion {
    id: String,
    description: String,
    command: Vec<String>,
}

impl FixSuggestion {
    fn new(id: String, description: String, command: Vec<String>) -> Self {
        Self {
            id,
            description,
            command,
        }
    }

    fn command_preview(&self) -> String {
        self.command.join(" ")
    }
}

fn apply_fixes(fixes: Vec<FixSuggestion>) -> Result<()> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for fix in fixes {
        if seen.insert(fix.id.clone()) {
            unique.push(fix);
        }
    }

    if unique.is_empty() {
        println!(
            "{}",
            style("All required checks are already satisfied.").green()
        );
        return Ok(());
    }

    println!();
    println!("{}", style("Attempting to fix required issues").bold());

    for fix in unique {
        println!("\n{} {}", style("•").cyan(), style(&fix.description).bold());
        println!("    {}", style(fix.command_preview()).dim());

        let apply = Confirm::new()
            .with_prompt("Apply this fix?")
            .default(true)
            .interact()
            .map_err(|err| {
                anyhow!(
                    "Unable to prompt for confirmation (is this running in an interactive terminal?): {err}"
                )
            })?;

        if !apply {
            println!("    {}", style("Skipped.").yellow());
            continue;
        }

        if fix.command.is_empty() {
            println!(
                "    {}",
                style("No command associated with this fix. Skipping.").yellow()
            );
            continue;
        }

        let mut command = Command::new(&fix.command[0]);
        if fix.command.len() > 1 {
            command.args(&fix.command[1..]);
        }

        match command.status() {
            Ok(status) if status.success() => {
                println!("    {}", style("Completed successfully.").green());
            }
            Ok(status) => {
                let code = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                println!(
                    "    {}",
                    style(format!("Command exited with status {code}.")).red()
                );
            }
            Err(err) => {
                println!(
                    "    {}",
                    style(format!("Failed to execute command: {err}")).red()
                );
            }
        }
    }

    Ok(())
}
