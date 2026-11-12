//! Embeds the commit through `waterui-git-head` and appends it to the run log
//! named by `GIT_HEAD_PROBE_RUNS`, one line per run, so the test sees both
//! whether Cargo reran the script and what it embedded.

use std::{env, fs::OpenOptions, io::Write, path::Path};

fn main() {
    let dir = env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR");
    let commit = waterui_git_head::head_commit(Path::new(&dir), None)
        .unwrap_or_else(|| "unknown".to_owned());
    let log = env::var_os("GIT_HEAD_PROBE_RUNS").expect("the test names the run log");
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .expect("open the run log");
    writeln!(log, "{commit}").expect("append to the run log");
    println!("cargo:rustc-env=GIT_HEAD_PROBE_COMMIT={commit}");
}
