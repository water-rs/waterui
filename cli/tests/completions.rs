//! `water completions` end-to-end coverage.
//!
//! Runs the built `water` binary once per supported shell and asserts the
//! generated script carries the shell-specific completer anchor, so a clap
//! definition drift that breaks generation fails here rather than at install
//! time on a user's machine.

use std::process::Command;

#[test]
fn completions_generate_a_script_for_each_supported_shell() {
    let home = tempfile::tempdir().expect("scratch home for the child process");
    for (shell, anchor) in [
        ("bash", "_water"),
        ("zsh", "#compdef water"),
        ("fish", "complete -c water"),
        ("powershell", "Register-ArgumentCompleter"),
        ("elvish", "arg-completer"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_water"))
            .args(["completions", shell])
            // Redirect the child's home so `ensure_global_config` never writes
            // `~/.water/config.toml` on the machine running the tests.
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env_remove("RUST_LOG")
            .output()
            .expect("spawn `water completions`");
        assert!(
            output.status.success(),
            "`water completions {shell}` must exit 0; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let script = String::from_utf8(output.stdout).expect("completion script is UTF-8");
        assert!(
            script.contains(anchor),
            "{shell} completion script must contain `{anchor}`"
        );
    }
}
