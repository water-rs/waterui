use std::{env, fs, path::PathBuf};

use cbindgen::{Config, generate_with_config};

fn main() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    println!("⌛️ Generating bindings...");
    let bindings = generate_with_config(
        &crate_dir,
        Config::from_file(crate_dir.join("cbindgen.toml")).expect("failed to load cbindgen.toml"),
    )
    .expect("Unable to generate bindings");
    let header = repair_known_cbindgen_misgenerations(bindings.to_string());
    fs::write(crate_dir.join("waterui.h"), header).expect("failed to write generated header");
    println!(
        "✅ Bindings generated at {}",
        crate_dir.join("waterui.h").display()
    );
    propagate_to_backends(&crate_dir.join("waterui.h"));
}

fn repair_known_cbindgen_misgenerations(header: String) -> String {
    const WRONG_NAVIGATION_SEARCH: &str = "typedef struct WuiNavigationSearch {\n  WuiBinding_Str *text;\n  struct WuiStr prompt;\n} WuiNavigationSearch;\n";
    const FIXED_NAVIGATION_SEARCH: &str = "typedef struct WuiNavigationSearch {\n  WuiBinding_Str *text;\n  struct WuiAnyView *prompt;\n} WuiNavigationSearch;\n";

    if header.contains(WRONG_NAVIGATION_SEARCH) {
        return header.replacen(WRONG_NAVIGATION_SEARCH, FIXED_NAVIGATION_SEARCH, 1);
    }

    if header.contains(FIXED_NAVIGATION_SEARCH) {
        return header;
    }

    panic!("generated header did not contain the expected WuiNavigationSearch definition");
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

#[cfg(test)]
mod tests {
    use super::repair_known_cbindgen_misgenerations;

    #[test]
    fn rewrites_navigation_search_prompt_to_anyview_pointer() {
        let input = "typedef struct WuiNavigationSearch {\n  WuiBinding_Str *text;\n  struct WuiStr prompt;\n} WuiNavigationSearch;\n".to_string();
        let output = repair_known_cbindgen_misgenerations(input);
        assert!(output.contains("struct WuiAnyView *prompt;"));
        assert!(!output.contains("struct WuiStr prompt;"));
    }
}
