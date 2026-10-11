//! `cherenkov-bench` binary entry — runs [`cherenkov_bench::cli::run_args`],
//! the same code path the iOS host app reaches through its C entry point.

use std::ffi::OsString;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().collect();
    let code = cherenkov_bench::cli::run_args(&args);
    ExitCode::from(u8::try_from(code).expect("run_args returns a process exit code"))
}
