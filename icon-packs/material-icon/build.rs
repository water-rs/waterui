//! Build script for waterui-icons-material-icon
//!
//! Parses individual SVG files from icons/ and generates one const per icon
//! for optimal tree-shaking.

use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=icons");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("icons.rs");
    let icons_dir = Path::new("icons");

    let mut output = String::new();

    // Header
    output.push_str("// Auto-generated Material Design icon definitions.\n");
    output.push_str("// Each icon is a separate const for tree-shaking.\n");
    output.push_str("// Do not edit manually.\n\n");

    // Collect and sort SVG files
    let mut svg_files: Vec<_> = fs::read_dir(icons_dir)
        .expect("Failed to read icons directory")
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "svg"))
        .collect();
    svg_files.sort_by_key(|e| e.file_name());

    for entry in svg_files {
        let path = entry.path();
        let file_name = path.file_stem().unwrap().to_str().unwrap();

        // Read and parse SVG
        let svg_content = fs::read_to_string(&path).expect("Failed to read SVG file");

        // Extract path d attribute
        if let Some(path_data) = extract_path_d(&svg_content) {
            let const_name = to_const_name(file_name);
            let fn_name = to_fn_name(file_name);

            // Path data constant
            output.push_str(&format!(
                "/// SVG path for `{}`.\n",
                file_name
            ));
            output.push_str(&format!(
                "pub const {}_PATH: &str = {:?};\n",
                const_name, path_data
            ));

            // Function returning Svg (for svg feature)
            output.push_str("#[cfg(feature = \"svg\")]\n");
            output.push_str(&format!(
                "/// `{}` icon as Svg.\n",
                file_name
            ));
            output.push_str("#[inline]\n");
            output.push_str(&format!(
                "pub fn {}() -> crate::Svg {{\n",
                fn_name
            ));
            output.push_str(&format!(
                "    crate::Svg::from_path({}_PATH, 24.0, 24.0)\n",
                const_name
            ));
            output.push_str("}\n\n");
        }
    }

    let mut file = fs::File::create(&dest_path).expect("Failed to create icons.rs");
    file.write_all(output.as_bytes())
        .expect("Failed to write icons.rs");
}

/// Extract the d attribute from the first <path> element.
fn extract_path_d(svg: &str) -> Option<String> {
    // Simple regex-like extraction for d="..."
    let path_start = svg.find("<path")?;
    let path_section = &svg[path_start..];
    let d_start = path_section.find("d=\"")? + 3;
    let d_section = &path_section[d_start..];
    let d_end = d_section.find('"')?;
    Some(d_section[..d_end].to_string())
}

/// Convert kebab-case to SCREAMING_SNAKE_CASE.
fn to_const_name(name: &str) -> String {
    name.replace('-', "_").to_uppercase()
}

/// Rust reserved keywords that need r# prefix.
const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
    "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
    "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try",
    "typeof", "unsized", "virtual", "yield",
];

/// Convert kebab-case to snake_case, escaping keywords.
fn to_fn_name(name: &str) -> String {
    let name = if name.chars().next().map_or(false, |c| c.is_numeric()) {
        format!("icon_{}", name)
    } else {
        name.to_string()
    };
    let snake = name.replace('-', "_");

    if RUST_KEYWORDS.contains(&snake.as_str()) {
        format!("r#{}", snake)
    } else {
        snake
    }
}
