use std::{
    io::{self, IsTerminal, Write},
    sync::{OnceLock, atomic::{AtomicBool, Ordering}},
};

use color_eyre::eyre::Result;
use serde::Serialize;

// ============================================================================
// Output Format
// ============================================================================

/// Supported output formats for CLI commands.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub enum OutputFormat {
    /// Human-readable output with colors and formatting
    #[default]
    Human,
    /// Machine-readable JSON output (single final payload)
    Json,
    /// Streaming JSON Lines format (one JSON object per line)
    /// Better for LLMs and real-time automation
    JsonLines,
}

impl OutputFormat {
    /// Check if the format expects JSON payloads.
    #[must_use]
    pub const fn is_json(self) -> bool {
        matches!(self, Self::Json | Self::JsonLines)
    }

    /// Check if the format is streaming (JSON Lines)
    #[must_use]
    pub const fn is_streaming(self) -> bool {
        matches!(self, Self::JsonLines)
    }

    /// Check if the format is human-readable
    #[must_use]
    pub const fn is_human(self) -> bool {
        matches!(self, Self::Human)
    }
}

// ============================================================================
// Global State
// ============================================================================

static GLOBAL_OUTPUT_FORMAT: OnceLock<OutputFormat> = OnceLock::new();
static NON_INTERACTIVE: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Store the desired global output format used across CLI commands.
pub fn set_global_output_format(format: OutputFormat) {
    let _ = GLOBAL_OUTPUT_FORMAT.set(format);
}

/// Access the configured global output format, defaulting to human-friendly logs.
#[must_use]
pub fn global_output_format() -> OutputFormat {
    *GLOBAL_OUTPUT_FORMAT.get().unwrap_or(&OutputFormat::Human)
}

/// Mark the CLI as running in non-interactive mode.
pub fn set_non_interactive(non_interactive: bool) {
    NON_INTERACTIVE.store(non_interactive, Ordering::Relaxed);
}

/// Check if the CLI should behave non-interactively.
///
/// Returns true if:
/// - Explicitly set via `--non-interactive` flag
/// - JSON output mode is enabled
/// - stdin/stdout are not terminals
#[must_use]
pub fn is_non_interactive() -> bool {
    if NON_INTERACTIVE.load(Ordering::Relaxed) {
        return true;
    }
    if global_output_format().is_json() {
        return true;
    }
    // Auto-detect non-TTY environment (important for LLMs/CI)
    !io::stdin().is_terminal() || !io::stdout().is_terminal()
}

/// Check if the CLI is running in an interactive terminal.
#[must_use]
pub fn is_interactive() -> bool {
    !is_non_interactive()
}

/// Set verbose output mode.
pub fn set_verbose(verbose: bool) {
    VERBOSE.store(verbose, Ordering::Relaxed);
}

/// Check if verbose mode is enabled.
#[must_use]
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

// ============================================================================
// JSON Output
// ============================================================================

/// Emit a JSON payload to stdout.
///
/// # Errors
/// Returns an error if the payload cannot be serialized.
pub fn emit_json<T>(payload: &T) -> Result<()>
where
    T: Serialize,
{
    println!("{}", serde_json::to_string(payload)?);
    Ok(())
}

/// Emit a pretty-printed JSON payload to stdout.
///
/// # Errors
/// Returns an error if the payload cannot be serialized.
pub fn emit_json_pretty<T>(payload: &T) -> Result<()>
where
    T: Serialize,
{
    println!("{}", serde_json::to_string_pretty(payload)?);
    Ok(())
}

// ============================================================================
// Streaming Events (JSON Lines)
// ============================================================================

/// Event types that can be streamed during command execution.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Command started
    Started {
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        platform: Option<String>,
    },
    /// Progress update
    Progress {
        stage: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        percent: Option<u8>,
    },
    /// Log message
    Log {
        level: LogLevel,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    /// Warning or issue detected
    Warning {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        suggestion: Option<String>,
    },
    /// Error occurred (non-fatal)
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        suggestion: Option<String>,
    },
    /// Prompt for user input (informational in non-interactive mode)
    Prompt {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        default: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        options: Vec<String>,
    },
    /// Command completed successfully
    Completed {
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
    },
    /// Command failed
    Failed {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<i32>,
    },
}

/// Log level for stream events
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl From<tracing::Level> for LogLevel {
    fn from(level: tracing::Level) -> Self {
        match level {
            tracing::Level::TRACE => Self::Trace,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::INFO => Self::Info,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::ERROR => Self::Error,
        }
    }
}

/// Emit a streaming event (JSON Lines format).
///
/// In JSON Lines mode, this emits immediately.
/// In other modes, this is a no-op (use the appropriate output method).
pub fn emit_event(event: &StreamEvent) {
    if !global_output_format().is_streaming() {
        return;
    }
    if let Ok(json) = serde_json::to_string(event) {
        let _ = writeln!(io::stdout(), "{json}");
        let _ = io::stdout().flush();
    }
}

/// Helper to emit a progress event
pub fn emit_progress(stage: impl Into<String>, detail: Option<String>) {
    emit_event(&StreamEvent::Progress {
        stage: stage.into(),
        detail,
        percent: None,
    });
}

/// Helper to emit a log event
pub fn emit_log(level: LogLevel, message: impl Into<String>) {
    emit_event(&StreamEvent::Log {
        level,
        message: message.into(),
        target: None,
    });
}

/// Helper to emit a warning event
pub fn emit_warning(message: impl Into<String>, suggestion: Option<String>) {
    emit_event(&StreamEvent::Warning {
        message: message.into(),
        suggestion,
    });
}

/// Helper to emit an error event
pub fn emit_error(message: impl Into<String>, suggestion: Option<String>) {
    emit_event(&StreamEvent::Error {
        message: message.into(),
        code: None,
        suggestion,
    });
}

// ============================================================================
// Conditional Output
// ============================================================================

/// Print a message only in human output mode.
#[macro_export]
macro_rules! human_println {
    ($($arg:tt)*) => {
        if !$crate::output::global_output_format().is_json() {
            println!($($arg)*);
        }
    };
}

/// Print to stderr only in human output mode.
#[macro_export]
macro_rules! human_eprintln {
    ($($arg:tt)*) => {
        if !$crate::output::global_output_format().is_json() {
            eprintln!($($arg)*);
        }
    };
}

// ============================================================================
// Environment Detection
// ============================================================================

/// Detect the execution environment for better defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    /// Interactive terminal with user present
    Interactive,
    /// CI/CD pipeline
    Ci,
    /// Non-interactive terminal (piped, etc.)
    NonInteractive,
    /// LLM/AI agent (detected via common indicators)
    Agent,
}

impl Environment {
    /// Detect the current execution environment.
    #[must_use]
    pub fn detect() -> Self {
        // Check for common CI environment variables
        if std::env::var("CI").is_ok()
            || std::env::var("GITHUB_ACTIONS").is_ok()
            || std::env::var("GITLAB_CI").is_ok()
            || std::env::var("JENKINS_URL").is_ok()
            || std::env::var("BUILDKITE").is_ok()
        {
            return Self::Ci;
        }

        // Check for LLM/agent indicators
        if std::env::var("CURSOR_AGENT").is_ok()
            || std::env::var("ANTHROPIC_API_KEY").is_ok()
            || std::env::var("OPENAI_API_KEY").is_ok()
        {
            return Self::Agent;
        }

        // Check terminal interactivity
        if io::stdin().is_terminal() && io::stdout().is_terminal() {
            Self::Interactive
        } else {
            Self::NonInteractive
        }
    }

    /// Get recommended output format for this environment.
    #[must_use]
    pub const fn recommended_format(self) -> OutputFormat {
        match self {
            Self::Interactive => OutputFormat::Human,
            Self::Ci | Self::NonInteractive => OutputFormat::Json,
            Self::Agent => OutputFormat::JsonLines,
        }
    }
}

// ============================================================================
// Structured Error Output
// ============================================================================

/// Structured error for machine consumption.
#[derive(Debug, Serialize)]
pub struct StructuredError {
    /// Error message
    pub message: String,
    /// Error code (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Detailed description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Suggested fix or action
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// Related `water doctor` fix ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix_id: Option<String>,
    /// Causal chain of errors
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub causes: Vec<String>,
}

impl StructuredError {
    /// Create a new structured error from a message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
            detail: None,
            suggestion: None,
            fix_id: None,
            causes: Vec::new(),
        }
    }

    /// Add a suggestion.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Add a fix ID for `water doctor`.
    #[must_use]
    pub fn with_fix_id(mut self, fix_id: impl Into<String>) -> Self {
        self.fix_id = Some(fix_id.into());
        self
    }

    /// Create from a color_eyre Report.
    #[must_use]
    pub fn from_report(report: &color_eyre::eyre::Report) -> Self {
        let causes: Vec<String> = report
            .chain()
            .skip(1)
            .map(|e| e.to_string())
            .collect();

        Self {
            message: report.to_string(),
            code: None,
            detail: None,
            suggestion: None,
            fix_id: None,
            causes,
        }
    }
}

/// Emit an error in the appropriate format.
pub fn emit_structured_error(error: &StructuredError) {
    match global_output_format() {
        OutputFormat::Human => {
            // Human format handled by caller
        }
        OutputFormat::Json => {
            let _ = emit_json(error);
        }
        OutputFormat::JsonLines => {
            emit_event(&StreamEvent::Failed {
                message: error.message.clone(),
                code: None,
            });
        }
    }
}
