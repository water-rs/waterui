use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{android, output};
use color_eyre::eyre::{Context, Result, eyre};
use console::style;
use indexmap::IndexSet;
use serde::Serialize;
use which::which;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckMode {
    Full,
    Quick,
}

impl CheckMode {
    fn is_full(self) -> bool {
        matches!(self, CheckMode::Full)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckTarget {
    Rust,
    Swift,
    Android,
}

impl CheckTarget {
    pub fn label(self) -> &'static str {
        match self {
            CheckTarget::Rust => "Rust toolchain",
            CheckTarget::Swift => "Swift toolchain",
            CheckTarget::Android => "Android tooling",
        }
    }
}

pub struct ToolchainReport {
    pub sections: Vec<Section>,
}

impl ToolchainReport {
    pub fn has_failures(&self) -> bool {
        self.sections.iter().any(Section::has_failure)
    }

    pub fn failure_summaries(&self) -> Vec<String> {
        self.sections
            .iter()
            .flat_map(Section::failure_summaries)
            .collect()
    }
}

pub fn run_checks(mode: CheckMode, targets: &[CheckTarget]) -> ToolchainReport {
    let mut sections = Vec::new();

    for &target in targets {
        if let Some(outcome) = perform_check(mode, target) {
            let (section, _) = outcome.into_parts();
            sections.push(section);
        }
    }

    ToolchainReport { sections }
}

pub fn ensure_ready(mode: CheckMode, targets: &[CheckTarget]) -> Result<()> {
    let report = run_checks(mode, targets);
    if !report.has_failures() {
        return Ok(());
    }

    let mut message = String::from("Required toolchain components are missing:");
    for failure in report.failure_summaries() {
        message.push_str("\n  - ");
        message.push_str(&failure);
    }
    message.push_str("\nRun `water doctor` for a full report.");

    Err(eyre!(message))
}

pub fn perform_check(mode: CheckMode, target: CheckTarget) -> Option<SectionOutcome> {
    match target {
        CheckTarget::Rust => Some(check_rust(mode)),
        CheckTarget::Swift => {
            if cfg!(target_os = "macos") {
                Some(check_swift())
            } else {
                None
            }
        }
        CheckTarget::Android => Some(check_android(mode)),
    }
}

pub fn apply_fixes(fixes: Vec<FixSuggestion>, mode: FixMode) -> Result<Vec<FixApplication>> {
    let is_json = output::global_output_format().is_json();

    let mut seen = IndexSet::new();
    let mut unique = Vec::new();
    for fix in fixes {
        if seen.insert(fix.id.clone()) {
            unique.push(fix);
        }
    }

    if unique.is_empty() {
        if !is_json {
            println!(
                "{}",
                style("All required checks are already satisfied.").green()
            );
        }
        return Ok(Vec::new());
    }

    if !is_json {
        println!();
        println!("{}", style("Attempting to fix required issues").bold());
    }

    let mut results = Vec::new();

    for fix in unique {
        if !is_json {
            println!(
                "\n{} {}",
                style("•").cyan(),
                style(fix.description()).bold()
            );
            println!("    {}", style(fix.command_preview()).dim());
        }

        let mut outcome = FixApplication {
            id: fix.id.clone(),
            description: fix.description().to_string(),
            command: fix.command.clone(),
            outcome: FixApplicationOutcome::Skipped,
            detail: None,
        };

        if !mode.should_apply_fix()? {
            if !is_json {
                println!("    {}", style("Skipped.").yellow());
            }
            outcome.outcome = FixApplicationOutcome::Skipped;
            outcome.detail = Some("User skipped this fix.".to_string());
            results.push(outcome);
            continue;
        }

        if fix.command.is_empty() {
            if !is_json {
                println!(
                    "    {}",
                    style("No command associated with this fix. Skipping.").yellow()
                );
            }
            outcome.outcome = FixApplicationOutcome::Unavailable;
            outcome.detail = Some("No command associated with this fix.".to_string());
            results.push(outcome);
            continue;
        }

        let mut command = Command::new(&fix.command[0]);
        if fix.command.len() > 1 {
            command.args(&fix.command[1..]);
        }

        match command.status() {
            Ok(status) if status.success() => {
                if !is_json {
                    println!("    {}", style("Completed successfully.").green());
                }
                outcome.outcome = FixApplicationOutcome::Applied;
                outcome.detail = None;
            }
            Ok(status) => {
                let code = status
                    .code()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                if !is_json {
                    println!(
                        "    {}",
                        style(format!("Command exited with status {code}.")).red()
                    );
                }
                outcome.outcome = FixApplicationOutcome::Failed;
                outcome.detail = Some(format!("Command exited with status {code}."));
            }
            Err(err) => {
                if !is_json {
                    println!(
                        "    {}",
                        style(format!("Failed to execute command: {err}")).red()
                    );
                }
                outcome.outcome = FixApplicationOutcome::Failed;
                outcome.detail = Some(format!("Failed to execute command: {err}"));
            }
        }

        results.push(outcome);
    }

    Ok(results)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixMode {
    Automatic,
    Interactive,
}

impl FixMode {
    fn should_apply_fix(self) -> Result<bool> {
        match self {
            FixMode::Automatic => Ok(true),
            FixMode::Interactive => {
                use dialoguer::Confirm;
                Confirm::new()
                    .with_prompt("Apply this fix?")
                    .default(true)
                    .interact()
                    .map_err(|err| {
                        eyre!(
                            "Unable to prompt for confirmation (is this running in an interactive terminal?): {err}"
                        )
                    })
            }
        }
    }
}

#[derive(Serialize)]
pub struct FixApplication {
    pub id: String,
    pub description: String,
    pub command: Vec<String>,
    pub outcome: FixApplicationOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixApplicationOutcome {
    Applied,
    Skipped,
    Failed,
    Unavailable,
}

pub struct SectionOutcome {
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

    pub fn section(&self) -> &Section {
        &self.section
    }

    pub fn into_parts(self) -> (Section, Vec<FixSuggestion>) {
        (self.section, self.fixes)
    }
}

#[derive(Clone, Serialize)]
pub struct FixSuggestion {
    pub id: String,
    description: String,
    command: Vec<String>,
}

impl FixSuggestion {
    pub fn new(id: String, description: String, command: Vec<String>) -> Self {
        Self {
            id,
            description,
            command,
        }
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn command_preview(&self) -> String {
        self.command.join(" ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Info,
}

#[derive(Clone, Serialize)]
pub struct Row {
    pub status: Status,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    indent: usize,
}

pub struct RowOutcome {
    pub row: Row,
    pub fix: Option<FixSuggestion>,
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

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_indent(mut self, indent: usize) -> Self {
        self.indent = indent;
        self
    }

    pub fn render(&self) -> Vec<String> {
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

    pub fn status(&self) -> Status {
        self.status
    }

    fn summary(&self, title: &str) -> String {
        format!("{title}: {}", self.message)
    }
}

#[derive(Clone, Serialize)]
pub struct Section {
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

    pub fn render(&self) -> Vec<String> {
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

    pub fn summary_line(&self) -> String {
        let icon = match self.overall_status() {
            Status::Pass => style("✔").green(),
            Status::Warn => style("⚠").yellow(),
            Status::Fail => style("✘").red(),
            Status::Info => style("•").cyan(),
        };
        format!("{} {}", icon, style(&self.title).bold())
    }

    pub fn overall_status(&self) -> Status {
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

    pub fn has_failure(&self) -> bool {
        self.rows
            .iter()
            .any(|row| matches!(row.status(), Status::Fail))
    }

    pub fn has_warning(&self) -> bool {
        self.rows
            .iter()
            .any(|row| matches!(row.status(), Status::Warn))
    }

    fn failure_summaries(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|row| matches!(row.status(), Status::Fail))
            .map(|row| row.summary(&self.title))
            .collect()
    }
}

fn check_rust(mode: CheckMode) -> SectionOutcome {
    let mut outcome = SectionOutcome::new("Rust toolchain");

    outcome.push_outcome(check_command(
        "cargo",
        "Install Rust from https://rustup.rs",
    ));
    outcome.push_outcome(check_command(
        "rustup",
        "Install Rust from https://rustup.rs",
    ));

    if mode.is_full() {
        extend_rust_details(&mut outcome);
    }

    outcome
}

fn extend_rust_details(outcome: &mut SectionOutcome) {
    outcome.push_outcome(RowOutcome::new(Row::info("Installed Rust targets")));

    if which("rustup").is_ok() {
        match Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
        {
            Ok(output) => {
                let installed_targets = String::from_utf8_lossy(&output.stdout);
                collect_rust_targets(&installed_targets, outcome);
            }
            Err(err) => outcome.push_outcome(RowOutcome::new(
                Row::warn("Could not query installed Rust targets")
                    .with_detail(format!("rustup target list --installed failed: {err}")),
            )),
        }
    }

    outcome.push_outcome(check_sccache_tool());
    if let Some(mold) = check_mold_tool() {
        outcome.push_outcome(mold);
    }
}

fn collect_rust_targets(installed_targets: &str, outcome: &mut SectionOutcome) {
    let (required, optional) = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            (
                vec![
                    (
                        "aarch64-apple-darwin",
                        Some("Required for macOS builds on Apple Silicon"),
                    ),
                    (
                        "aarch64-apple-ios-sim",
                        Some("Required for Apple Silicon iOS Simulator"),
                    ),
                ],
                vec![
                    ("aarch64-apple-ios", "for deploying to physical iOS devices"),
                    ("armv7-apple-ios", "for legacy 32-bit iOS devices"),
                    ("x86_64-apple-darwin", "for Rosetta macOS builds"),
                    ("x86_64-apple-ios", "for Intel iOS Simulator"),
                ],
            )
        } else {
            (
                vec![
                    (
                        "x86_64-apple-darwin",
                        Some("Required for macOS builds on Intel"),
                    ),
                    (
                        "aarch64-apple-ios-sim",
                        Some("Required for Apple Silicon iOS Simulator"),
                    ),
                ],
                vec![
                    ("aarch64-apple-ios", "for Apple Silicon devices"),
                    ("armv7-apple-ios", "for legacy 32-bit iOS devices"),
                    ("x86_64-apple-ios", "for Intel iOS Simulator"),
                ],
            )
        }
    } else if cfg!(target_os = "windows") {
        (
            vec![(
                "aarch64-pc-windows-msvc",
                Some("Required for Windows on ARM"),
            )],
            vec![
                ("x86_64-pc-windows-gnu", "for MinGW environments"),
                ("i686-pc-windows-msvc", "for 32-bit Windows"),
            ],
        )
    } else {
        (
            vec![(
                "x86_64-unknown-linux-gnu",
                Some("Required for native Linux builds"),
            )],
            vec![
                (
                    "aarch64-unknown-linux-gnu",
                    "for cross-compiling to ARM64 Linux",
                ),
                ("armv7-unknown-linux-gnueabihf", "for ARMv7 Linux"),
            ],
        )
    };

    if !required.is_empty() {
        outcome.push(Row::info("Required targets").with_indent(1));
        for (target, note) in required {
            outcome.push_outcome(target_row(
                installed_targets,
                target,
                TargetKind::Required { note },
            ));
        }
    }

    if !optional.is_empty() {
        outcome.push(Row::info("Optional targets").with_indent(1));
        for (target, reason) in optional {
            outcome.push_outcome(target_row(
                installed_targets,
                target,
                TargetKind::Optional(reason),
            ));
        }
    }
}

fn check_swift() -> SectionOutcome {
    let mut outcome = SectionOutcome::new("Swift (macOS)");
    outcome.push_outcome(check_command(
        "xcodebuild",
        "Install Xcode and command line tools (xcode-select --install)",
    ));
    outcome.push_outcome(check_command(
        "xcrun",
        "Install Xcode and command line tools (xcode-select --install)",
    ));
    outcome
}

fn check_android(mode: CheckMode) -> SectionOutcome {
    let mut outcome = SectionOutcome::new("Android tooling");
    let prerequisites = match check_android_prerequisites(mode) {
        Ok(outcomes) => outcomes,
        Err(err) => {
            outcome.push(
                Row::fail("Failed to run Android prerequisites check").with_detail(err.to_string()),
            );
            return outcome;
        }
    };

    for row_outcome in prerequisites {
        outcome.push_outcome(row_outcome);
    }

    outcome
}

fn check_command(name: &str, help: &str) -> RowOutcome {
    match which(name) {
        Ok(path) => RowOutcome::new(
            Row::pass(format!("Found `{name}`")).with_detail(path.display().to_string()),
        ),
        Err(_) => RowOutcome::new(Row::fail(format!("`{name}` not found")).with_detail(help)),
    }
}

fn check_sccache_tool() -> RowOutcome {
    match which("sccache") {
        Ok(path) => RowOutcome::new(
            Row::pass("`sccache` build cache available")
                .with_indent(1)
                .with_detail(path.display().to_string()),
        ),
        Err(_) => {
            let detail =
                "Install sccache to cache Rust compilation outputs. Run `cargo install sccache`.";
            let row = Row::warn("`sccache` not installed")
                .with_indent(1)
                .with_detail(detail);
            RowOutcome::with_fix(
                row,
                FixSuggestion::new(
                    "tool-sccache".into(),
                    "Install sccache build cache".into(),
                    vec!["cargo".into(), "install".into(), "sccache".into()],
                ),
            )
        }
    }
}

fn check_mold_tool() -> Option<RowOutcome> {
    if !cfg!(target_os = "linux") {
        return None;
    }

    match which("mold") {
        Ok(path) => Some(RowOutcome::new(
            Row::pass("`mold` linker available")
                .with_indent(1)
                .with_detail(path.display().to_string()),
        )),
        Err(_) => {
            let mut detail = String::from("Install mold to speed up Rust linking on Linux.");
            if let Some((fix, hint)) = mold_fix_suggestion() {
                if !hint.is_empty() {
                    detail.push(' ');
                    detail.push_str(&hint);
                }
                Some(RowOutcome::with_fix(
                    Row::warn("`mold` linker not installed")
                        .with_indent(1)
                        .with_detail(detail),
                    fix,
                ))
            } else {
                detail
                    .push_str(" See https://github.com/rui314/mold for installation instructions.");
                Some(RowOutcome::new(
                    Row::warn("`mold` linker not installed")
                        .with_indent(1)
                        .with_detail(detail),
                ))
            }
        }
    }
}

fn mold_fix_suggestion() -> Option<(FixSuggestion, String)> {
    if cfg!(not(any(target_os = "linux", target_os = "macos"))) {
        return None;
    }
    if which("apt-get").is_ok() {
        let mut command = Vec::new();
        if which("sudo").is_ok() {
            command.push("sudo".into());
        }
        command.extend(
            ["apt-get", "install", "-y", "mold"]
                .into_iter()
                .map(String::from),
        );
        let preview = command.join(" ");
        let description = "Install mold linker via apt".to_string();
        let fix = FixSuggestion::new("tool-mold".into(), description, command);
        return Some((fix, format!("Try `{preview}`.")));
    }

    if which("dnf").is_ok() {
        let mut command = Vec::new();
        if which("sudo").is_ok() {
            command.push("sudo".into());
        }
        command.extend(
            ["dnf", "install", "-y", "mold"]
                .into_iter()
                .map(String::from),
        );
        let preview = command.join(" ");
        let description = "Install mold linker via dnf".to_string();
        let fix = FixSuggestion::new("tool-mold".into(), description, command);
        return Some((fix, format!("Try `{preview}`.")));
    }

    if which("pacman").is_ok() {
        let mut command = Vec::new();
        if which("sudo").is_ok() {
            command.push("sudo".into());
        }
        command.extend(
            ["pacman", "-S", "--noconfirm", "mold"]
                .into_iter()
                .map(String::from),
        );
        let preview = command.join(" ");
        let description = "Install mold linker via pacman".to_string();
        let fix = FixSuggestion::new("tool-mold".into(), description, command);
        return Some((fix, format!("Try `{preview}`.")));
    }

    if which("brew").is_ok() {
        let mut command = Vec::new();
        if which("sudo").is_ok() {
            command.push("sudo".into());
        }
        command.extend(["brew", "install", "mold"].into_iter().map(String::from));
        let preview = command.join(" ");
        let description = "Install mold linker via Homebrew".to_string();
        let fix = FixSuggestion::new("tool-mold".into(), description, command);
        return Some((fix, format!("Try `{preview}`.")));
    }

    None
}

fn check_android_prerequisites(_mode: CheckMode) -> Result<Vec<RowOutcome>> {
    let mut outcomes = vec![
        check_android_tool(
            "adb",
            "Install Android SDK Platform-Tools and ensure they are in your Android SDK or PATH.",
        ),
        check_android_tool(
            "emulator",
            "Install Android SDK command-line tools and ensure they are in your Android SDK or PATH.",
        ),
    ];

    outcomes.push(check_java_environment());

    let env_status = evaluate_android_env();
    outcomes.extend(env_status.rows);

    if let Some(root) = env_status.root.clone() {
        let sdk = AndroidSdk::new(root);
        outcomes.extend(sdk.check_components()?);
    }

    if which("rustup").is_ok() {
        let output = Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .context("failed to query installed rust targets")?;

        let installed_targets = String::from_utf8_lossy(&output.stdout);

        if cfg!(target_arch = "aarch64") {
            outcomes.push(target_row(
                &installed_targets,
                "aarch64-linux-android",
                TargetKind::Required {
                    note: Some("Required for Android emulator on Apple Silicon"),
                },
            ));
        }
    } else {
        outcomes.push(RowOutcome::new(Row::warn(
            "rustup not found, cannot check Rust targets.",
        )));
    }

    Ok(outcomes)
}

fn check_java_environment() -> RowOutcome {
    let java_env = env::var("JAVA_HOME").ok();
    let java_version_cmd = if let Some(java_home) = java_env.clone() {
        let java_exe = Path::new(&java_home).join("bin/java");
        if java_exe.exists() {
            Some(Command::new(java_exe))
        } else {
            None
        }
    } else if which("java").is_ok() {
        Some(Command::new("java"))
    } else {
        None
    };

    if let Some(mut cmd) = java_version_cmd {
        match cmd.arg("-version").output() {
            Ok(output) => {
                let version_info = String::from_utf8_lossy(&output.stderr);
                if let Some(line) = version_info.lines().next() {
                    RowOutcome::new(Row::pass("Java detected").with_detail(line.trim().to_string()))
                } else {
                    RowOutcome::new(Row::warn("Could not determine Java version"))
                }
            }
            Err(err) => RowOutcome::new(
                Row::warn("Failed to read Java version")
                    .with_detail(format!("java -version failed: {err}")),
            ),
        }
    } else {
        let detail = if cfg!(target_os = "macos") {
            "Install a Java Development Kit (JDK 17 or newer). Try `brew install --cask temurin@17`."
        } else if cfg!(target_os = "linux") {
            "Install a Java Development Kit (JDK 17 or newer) using your distribution's package manager."
        } else if cfg!(target_os = "windows") {
            "Install a Java Development Kit (JDK 17 or newer) and ensure `JAVA_HOME` is set."
        } else {
            "Install a Java Development Kit (JDK 17 or newer) and ensure it is on PATH."
        };
        let row = Row::fail("Java not found in JAVA_HOME or PATH").with_detail(detail.to_string());
        if let Some(fix) = java_install_fix() {
            RowOutcome::with_fix(row, fix)
        } else {
            RowOutcome::new(row)
        }
    }
}

fn check_android_tool(name: &str, help: &str) -> RowOutcome {
    if let Some(path) = android::find_android_tool(name) {
        RowOutcome::new(
            Row::pass(format!("Found `{name}`")).with_detail(path.display().to_string()),
        )
    } else {
        RowOutcome::new(Row::fail(format!("`{name}` not found")).with_detail(help))
    }
}

struct AndroidEnvStatus {
    root: Option<PathBuf>,
    rows: Vec<RowOutcome>,
}

fn evaluate_android_env() -> AndroidEnvStatus {
    let mut rows = Vec::new();

    let root = check_env_path(
        &mut rows,
        "ANDROID_SDK_ROOT",
        true,
        "Set ANDROID_SDK_ROOT to the root of your Android SDK installation.",
    )
    .or_else(|| {
        check_env_path(
            &mut rows,
            "ANDROID_HOME",
            false,
            "Set ANDROID_HOME to the root of your Android SDK installation.",
        )
    });

    let _ndk = check_env_path(
        &mut rows,
        "ANDROID_NDK_HOME",
        false,
        "Set ANDROID_NDK_HOME to the Android NDK directory (usually inside the SDK's ndk folder).",
    );

    AndroidEnvStatus { root, rows }
}

fn check_env_path(
    rows: &mut Vec<RowOutcome>,
    name: &str,
    required: bool,
    help: &str,
) -> Option<PathBuf> {
    if let Ok(value) = env::var(name) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                rows.push(RowOutcome::new(
                    Row::pass(format!("Environment `{name}` set")).with_detail(trimmed.to_string()),
                ));
                return Some(path);
            }

            rows.push(RowOutcome::new(
                Row::fail(format!("Environment `{name}` points to a missing path"))
                    .with_detail(format!("{trimmed} does not exist")),
            ));
            return None;
        }
    }

    if let Some(fallback) = auto_detect_android_path(name) {
        rows.push(RowOutcome::new(
            Row::pass(format!("Environment `{name}` auto-detected"))
                .with_detail(fallback.display().to_string()),
        ));
        return Some(fallback);
    }

    let row = if required {
        Row::fail(format!("Environment `{name}` missing"))
    } else {
        Row::warn(format!("Environment `{name}` missing"))
    }
    .with_detail(help);
    rows.push(RowOutcome::new(row));
    None
}

fn auto_detect_android_path(name: &str) -> Option<PathBuf> {
    match name {
        "ANDROID_SDK_ROOT" | "ANDROID_HOME" => android::resolve_android_sdk_path(),
        "ANDROID_NDK_HOME" => android::resolve_android_ndk_path(),
        _ => None,
    }
}

fn java_install_fix() -> Option<FixSuggestion> {
    if cfg!(target_os = "macos") && which("brew").is_ok() {
        return Some(FixSuggestion::new(
            "java-install-brew".into(),
            "Install Temurin JDK 17 via Homebrew".into(),
            vec![
                "brew".into(),
                "install".into(),
                "--cask".into(),
                "temurin@17".into(),
            ],
        ));
    }

    if which("apt-get").is_ok() {
        let mut command = Vec::new();
        if which("sudo").is_ok() {
            command.push("sudo".into());
        }
        command.extend(
            ["apt-get", "install", "-y", "openjdk-17-jdk"]
                .into_iter()
                .map(String::from),
        );
        return Some(FixSuggestion::new(
            "java-install-apt".into(),
            "Install OpenJDK 17 via apt".into(),
            command,
        ));
    }

    if which("dnf").is_ok() {
        let mut command = Vec::new();
        if which("sudo").is_ok() {
            command.push("sudo".into());
        }
        command.extend(
            ["dnf", "install", "-y", "java-17-openjdk-devel"]
                .into_iter()
                .map(String::from),
        );
        return Some(FixSuggestion::new(
            "java-install-dnf".into(),
            "Install OpenJDK 17 via dnf".into(),
            command,
        ));
    }

    if which("pacman").is_ok() {
        let mut command = Vec::new();
        if which("sudo").is_ok() {
            command.push("sudo".into());
        }
        command.extend(
            ["pacman", "-S", "--noconfirm", "jdk-openjdk"]
                .into_iter()
                .map(String::from),
        );
        return Some(FixSuggestion::new(
            "java-install-pacman".into(),
            "Install OpenJDK via pacman".into(),
            command,
        ));
    }

    if cfg!(target_os = "windows") && which("choco").is_ok() {
        return Some(FixSuggestion::new(
            "java-install-choco".into(),
            "Install Temurin JDK 17 via Chocolatey".into(),
            vec![
                "choco".into(),
                "install".into(),
                "temurin17jdk".into(),
                "-y".into(),
            ],
        ));
    }

    None
}

struct AndroidSdk {
    root: PathBuf,
    sdkmanager: Option<PathBuf>,
}

impl AndroidSdk {
    fn new(root: PathBuf) -> Self {
        let sdkmanager = locate_sdkmanager(&root);
        Self { root, sdkmanager }
    }

    fn check_components(&self) -> Result<Vec<RowOutcome>> {
        let mut rows = Vec::new();

        if let Some(path) = &self.sdkmanager {
            rows.push(RowOutcome::new(
                Row::pass("`sdkmanager` available")
                    .with_indent(1)
                    .with_detail(path.display().to_string()),
            ));
        } else {
            rows.push(RowOutcome::new(
                Row::warn("`sdkmanager` not found")
                    .with_indent(1)
                    .with_detail("Install Android command-line tools to obtain sdkmanager."),
            ));
        }

        rows.push(self.component_row(
            "android-platform-tools",
            "Android Platform Tools",
            self.root.join("platform-tools"),
            &["platform-tools"],
        ));

        rows.push(self.build_tools_row()?);
        rows.push(self.platforms_row()?);
        rows.push(self.ndk_row());

        Ok(rows)
    }

    fn component_row(&self, id: &str, label: &str, path: PathBuf, packages: &[&str]) -> RowOutcome {
        if path.exists() {
            RowOutcome::new(
                Row::pass(format!("{label} installed"))
                    .with_indent(1)
                    .with_detail(path.display().to_string()),
            )
        } else {
            let row = Row::fail(format!("{label} missing"))
                .with_indent(1)
                .with_detail(format!(
                    "Install {label} using sdkmanager (package{} {}).",
                    if packages.len() == 1 { "" } else { "s" },
                    packages.join(", "),
                ));
            if let Some(fix) = self.install_fix(id, format!("Install {label}"), packages) {
                RowOutcome::with_fix(row, fix)
            } else {
                RowOutcome::new(row)
            }
        }
    }

    fn build_tools_row(&self) -> Result<RowOutcome> {
        let build_tools_dir = self.root.join("build-tools");
        if !build_tools_dir.exists() {
            return Ok(self.component_row(
                "android-build-tools",
                "Android Build Tools",
                build_tools_dir,
                &["build-tools;34.0.0"],
            ));
        }

        let has_version = fs::read_dir(&build_tools_dir)?
            .filter_map(Result::ok)
            .any(|entry| entry.path().is_dir());

        if has_version {
            Ok(RowOutcome::new(
                Row::pass("Android Build Tools installed")
                    .with_indent(1)
                    .with_detail(build_tools_dir.display().to_string()),
            ))
        } else {
            Ok(self.component_row(
                "android-build-tools",
                "Android Build Tools",
                build_tools_dir,
                &["build-tools;34.0.0"],
            ))
        }
    }

    fn platforms_row(&self) -> Result<RowOutcome> {
        let platforms_dir = self.root.join("platforms");
        if !platforms_dir.exists() {
            return Ok(self.component_row(
                "android-platform",
                "Android platform SDK",
                platforms_dir,
                &["platforms;android-34"],
            ));
        }

        let has_platform = fs::read_dir(&platforms_dir)?
            .filter_map(Result::ok)
            .any(|entry| entry.path().is_dir());

        if has_platform {
            Ok(RowOutcome::new(
                Row::pass("Android platform SDK installed")
                    .with_indent(1)
                    .with_detail(platforms_dir.display().to_string()),
            ))
        } else {
            Ok(self.component_row(
                "android-platform",
                "Android platform SDK",
                platforms_dir,
                &["platforms;android-34"],
            ))
        }
    }

    fn ndk_row(&self) -> RowOutcome {
        let ndk_dir = self.root.join("ndk");
        if ndk_dir.exists() {
            if let Some(first_ndk) = first_child_dir(&ndk_dir) {
                return RowOutcome::new(
                    Row::pass("Android NDK installed")
                        .with_indent(1)
                        .with_detail(first_ndk.display().to_string()),
                );
            }
        }

        let row = Row::fail("Android NDK not installed")
            .with_indent(1)
            .with_detail("Install the Android NDK using sdkmanager.");
        if let Some(fix) =
            self.install_fix("android-ndk", "Install Android NDK", &["ndk;26.1.10909125"])
        {
            RowOutcome::with_fix(row, fix)
        } else {
            RowOutcome::new(row)
        }
    }

    fn install_fix(
        &self,
        id: &str,
        description: impl Into<String>,
        packages: &[&str],
    ) -> Option<FixSuggestion> {
        let sdkmanager = self.sdkmanager.as_ref()?;
        let mut command = Vec::new();
        command.push(sdkmanager.display().to_string());
        command.push(format!("--sdk_root={}", self.root.display()));
        command.push("--install".into());
        command.extend(packages.iter().map(|pkg| pkg.to_string()));
        Some(FixSuggestion::new(id.into(), description.into(), command))
    }
}

fn first_child_dir(path: &Path) -> Option<PathBuf> {
    fs::read_dir(path)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|candidate| candidate.is_dir())
}

fn locate_sdkmanager(root: &Path) -> Option<PathBuf> {
    if let Ok(path) = which("sdkmanager") {
        return Some(path);
    }

    let candidates: &[&str] = if cfg!(windows) {
        &[
            "cmdline-tools/latest/bin/sdkmanager.bat",
            "cmdline-tools/bin/sdkmanager.bat",
        ]
    } else {
        &[
            "cmdline-tools/latest/bin/sdkmanager",
            "cmdline-tools/bin/sdkmanager",
        ]
    };

    for candidate in candidates {
        let full = root.join(candidate);
        if full.exists() {
            return Some(full);
        }
    }

    None
}

enum TargetKind {
    Required { note: Option<&'static str> },
    Optional(&'static str),
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
