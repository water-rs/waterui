use std::{env, path::PathBuf};

use cbindgen::{Config, generate_with_config};

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("⌛️ Generating bindings...");
    generate_with_config(
        &crate_dir,
        Config::from_file(&crate_dir.join("cbindgen.toml")).unwrap(),
    )
    .expect("Unable to generate bindings")
    .write_to_file(&crate_dir.join("waterui.h"));
    println!(
        "✅ Bindings generated at {}",
        crate_dir.join("waterui.h").display()
    );
}
