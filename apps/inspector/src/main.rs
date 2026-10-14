//! The `waterui-inspector` standalone desktop application.
//!
//! Built only when the `desktop` feature is enabled; the binary is a runtime
//! artifact, not a crate component.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    target_os = "linux",
    windows
)))]
compile_error!("waterui-inspector desktop binary supports ARM64 macOS, Linux and Windows only");

use clap::Parser;
use waterui_inspector_protocol::InspectorBuildIdentity;

/// Command line for the Inspector.
#[derive(clap::Parser)]
#[command(version)]
struct Args {
    /// Print this build's framework and protocol identity as JSON, then exit.
    #[arg(long)]
    identity: bool,
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init()
        .expect("the inspector logger installs once at startup");

    let args = Args::parse();
    if args.identity {
        // Headless metadata: report before any environment or window exists.
        let identity = waterui_inspector_protocol::build_identity(waterui::FRAMEWORK_VERSION);
        if let Err(error) = write_identity(&mut std::io::stdout().lock(), &identity) {
            tracing::error!(%error, "failed to write the inspector identity");
            return std::process::ExitCode::FAILURE;
        }
        return std::process::ExitCode::SUCCESS;
    }

    // The packaged client resolves fonts and assets from the directories
    // beside its executable; install that context before the app exists.
    let env = match waterui::ResourceContext::application() {
        Ok(resources) => inspector_environment(resources),
        Err(error) => {
            tracing::error!(%error, "could not locate the inspector's resources");
            return std::process::ExitCode::FAILURE;
        }
    };
    hydrolysis::run(
        waterui_inspector_app::app(env),
        hydrolysis_m3::Material3::defaults(),
    );
    std::process::ExitCode::SUCCESS
}

/// Serializes the identity and a trailing newline, propagating every failure —
/// a stdout write is output, not a best-effort log.
fn write_identity(
    writer: &mut impl std::io::Write,
    identity: &InspectorBuildIdentity,
) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, identity).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

/// The environment the GUI branch runs under: the configured environment
/// plus the packaged resource context — one factory so `main` and its test
/// exercise the same construction.
fn inspector_environment(resources: waterui::ResourceContext) -> waterui::env::Environment {
    let mut env = waterui::configure_environment!(waterui::env::Environment::new());
    env.insert(resources);
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Accepts every byte of the payload but refuses the trailing newline.
    struct FailOnNewline;

    impl Write for FailOnNewline {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes == b"\n" {
                return Err(std::io::Error::other("newline refused"));
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FailOnFlush;

    impl Write for FailOnFlush {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("flush refused"))
        }
    }

    fn identity() -> InspectorBuildIdentity {
        waterui_inspector_protocol::build_identity("0.0.0")
    }

    #[test]
    fn a_newline_write_failure_is_reported() {
        let mut writer = FailOnNewline;
        assert!(write_identity(&mut writer, &identity()).is_err());
    }

    #[test]
    fn a_flush_failure_is_reported() {
        let mut writer = FailOnFlush;
        assert!(write_identity(&mut writer, &identity()).is_err());
    }

    #[test]
    fn the_identity_writes_valid_json_and_a_newline() {
        let mut bytes = Vec::new();
        write_identity(&mut bytes, &identity()).expect("identity serialized");
        let parsed: serde_json::Value =
            serde_json::from_slice(&bytes[..bytes.len() - 1]).expect("valid JSON");
        assert_eq!(parsed["framework_version"], "0.0.0");
        assert_eq!(bytes[bytes.len() - 1], b'\n');
    }

    #[test]
    fn the_environment_carries_the_application_resource_context() {
        // The packaged client resolves fonts and assets from beside its
        // executable — the same context `main` installs before the app runs.
        let env = inspector_environment(
            waterui::ResourceContext::application()
                .expect("the app's own executable has a resource context"),
        );
        assert!(env.get::<waterui::ResourceContext>().is_some());
    }
}
