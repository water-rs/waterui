//! Pointing `WATERUI_TS_BUNDLE` at a bundle for the test binary that includes
//! this file.
//!
//! `water test` builds the crate's bundle, writes each mounted module's props
//! contract hash into the entry it generates, and sets the variable before it
//! runs `cargo nextest`. A test binary here stands in for the CLI: it takes
//! the checked-in `fixtures/mount.js` — whose entry publishes whatever
//! `globalThis.__waterui_test_contracts` holds — puts the contract table in
//! front of it, exactly as the CLI's entry declares it, writes the result
//! under `CARGO_TARGET_TMPDIR`, and names it in the variable.
//!
//! Included with `#[path]` so a test target chooses whether it configures the
//! variable at all: the targets that assert what happens without it must not
//! link this in.
//!
//! # Process environment
//!
//! Writing a process's environment is `unsafe` in Rust 2024 because a
//! concurrent read on another thread is a data race. Every write here runs
//! inside a [`Once`] that each test calls first thing, so the first test to
//! arrive performs the write while any other test in the binary is blocked in
//! `call_once`, before it has run anything that could read the environment;
//! under nextest, which runs each test in its own process, there is no other
//! test at all. `remove_var` follows the same rule in the targets that use it.

use std::path::PathBuf;
use std::sync::Once;

use waterui::ts::BUNDLE_VARIABLE;

/// The whole application bundle: library, module and entry.
const BUNDLE: &str = include_str!("../fixtures/mount.js");

/// The module id `tsx!("fixtures/promo.tsx", …)` in a test beside this file
/// resolves to, and the key the bundle publishes the module under.
pub const MODULE: &str = "tests/fixtures/promo.tsx";

/// Sets `WATERUI_TS_BUNDLE` to a bundle that declares `contract` for
/// [`MODULE`], once per process.
///
/// The file is written to a temporary name and renamed into place, so a
/// second test process writing the same bundle at the same time can never
/// hand this one a torn read.
///
/// # Panics
///
/// Panics when the bundle cannot be written.
pub fn configure(contract: u64) {
    static CONFIGURED: Once = Once::new();
    CONFIGURED.call_once(|| {
        let path = write_bundle(contract);
        // SAFETY: inside the `Once`, as the module documentation explains —
        // no other test in this process is past `call_once`, so nothing reads
        // the environment while it is written.
        unsafe { std::env::set_var(BUNDLE_VARIABLE, &path) };
    });
}

/// Writes the seeded bundle and answers with its absolute path.
fn write_bundle(contract: u64) -> PathBuf {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("waterui-ts-hosts");
    std::fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("creating {}: {error}", directory.display()));
    let path = directory.join(format!("{contract:016x}.js"));
    let staged = directory.join(format!("{contract:016x}.{}.tmp", std::process::id()));
    let source = format!(
        "globalThis.__waterui_test_contracts = {{ \"{MODULE}\": \"{contract:016x}\" }};\n{BUNDLE}"
    );
    std::fs::write(&staged, source)
        .unwrap_or_else(|error| panic!("writing {}: {error}", staged.display()));
    std::fs::rename(&staged, &path)
        .unwrap_or_else(|error| panic!("renaming into {}: {error}", path.display()));
    path
}
