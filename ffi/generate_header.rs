use std::{env, fs, path::PathBuf};

use cbindgen::{Config, generate_with_config};

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("⌛️ Generating bindings...");
    generate_with_config(
        &crate_dir,
        Config::from_file(crate_dir.join("cbindgen.toml")).expect("failed to load cbindgen.toml"),
    )
    .expect("Unable to generate bindings")
    .write_to_file(crate_dir.join("waterui.h"));
    println!(
        "✅ Bindings generated at {}",
        crate_dir.join("waterui.h").display()
    );
    propagate_to_backends(&crate_dir.join("waterui.h"));
}

fn propagate_to_backends(header_path: &PathBuf) {
    let workspace_root = header_path
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| {
            panic!(
                "failed to determine workspace root from {}",
                header_path.display()
            )
        });

    let destinations = [
        workspace_root.join("backends/apple/Sources/CWaterUI/include/waterui.h"),
        workspace_root.join("backends/android/runtime/src/main/cpp/waterui.h"),
    ];

    for dest in destinations {
        if let Some(parent) = dest.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("⚠️  Failed to create directory {}: {err}", parent.display());
                continue;
            }
        }

        match fs::copy(header_path, &dest) {
            Ok(_) => println!("📦 Copied bindings to {}", dest.display()),
            Err(err) => eprintln!("⚠️  Failed to copy to {}: {err}", dest.display()),
        }
    }
}
