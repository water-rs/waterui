//! `nextest-fixtures [--root <dir>]`: checks the nextest setup-script rules
//! in `<root>/.config/nextest.toml` against the workspace's package graph;
//! the `fixtures` module documents the rule. Exits 1 when they drift.

mod fixtures;

use std::process::Command;

use camino::Utf8PathBuf;
use guppy::CargoMetadata;

fn cargo_metadata(manifest_path: &Utf8PathBuf) -> CargoMetadata {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest_path)
        .output()
        .expect("cargo metadata failed to start");
    if !output.status.success() {
        panic!(
            "cargo metadata on {manifest_path} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let json = String::from_utf8(output.stdout).expect("cargo metadata output is not UTF-8");
    CargoMetadata::parse_json(&json).expect("cargo metadata output did not parse")
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut root = Utf8PathBuf::from(".");
    while let Some(arg) = args.next() {
        match arg.as_ref() {
            "--root" => root = Utf8PathBuf::from(args.next().expect("--root needs a value")),
            other => panic!("unknown argument: {other}"),
        }
    }
    let graph = cargo_metadata(&root.join("Cargo.toml"))
        .build_graph()
        .expect("the workspace's package graph did not resolve");
    if !fixtures::check(&root, &graph) {
        std::process::exit(1);
    }
}
