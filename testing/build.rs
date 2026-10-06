//! Forwards the target triple the harness is compiled for: its
//! `cargo metadata` call filters the graph to that platform.

fn main() {
    let target = std::env::var("TARGET").expect("cargo sets `TARGET` for build scripts");
    println!("cargo::rustc-env=WATERUI_TESTING_TARGET={target}");
}
