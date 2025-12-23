//! Build script for waterui-icons-fontawesome7
//!
//! Parses individual SVG files from icons/{brands,regular,solid}/ and generates
//! one const per icon for optimal tree-shaking.

use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=icons");

    let out_dir = env::var("OUT_DIR").unwrap();
    let icons_dir = Path::new("icons");

    // Generate each style as a separate module
    for style in &["brands", "regular", "solid"] {
        let style_dir = icons_dir.join(style);
        if !style_dir.exists() {
            continue;
        }

        let dest_path = Path::new(&out_dir).join(format!("{}.rs", style));
        let mut output = String::new();

        // Header
        output.push_str(&format!(
            "// Auto-generated Font Awesome 7 {} icons.\n",
            style
        ));
        output.push_str("// Icons: CC BY 4.0 by Fonticons, Inc.\n");
        output.push_str("// Do not edit manually.\n\n");

        // Collect and sort SVG files
        let mut svg_files: Vec<_> = fs::read_dir(&style_dir)
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

            // Extract viewBox and path
            if let (Some(viewbox), Some(path_data)) =
                (extract_viewbox(&svg_content), extract_path_d(&svg_content))
            {
                let const_name = to_const_name(file_name);
                let fn_name = to_fn_name(file_name);

                // Path data constant
                output.push_str(&format!("/// SVG path for `{}`.\n", file_name));
                output.push_str(&format!(
                    "pub const {}_PATH: &str = {:?};\n",
                    const_name, path_data
                ));

                // ViewBox dimensions constant
                output.push_str(&format!(
                    "/// ViewBox dimensions for `{}`.\n",
                    file_name
                ));
                output.push_str(&format!(
                    "pub const {}_VIEWBOX: (f32, f32) = ({:.1}, {:.1});\n",
                    const_name, viewbox.0, viewbox.1
                ));

                // Function returning Svg (for svg feature)
                output.push_str("#[cfg(feature = \"svg\")]\n");
                output.push_str(&format!("/// `{}` icon as Svg.\n", file_name));
                output.push_str("#[inline]\n");
                output.push_str(&format!(
                    "pub fn {}() -> crate::Svg {{\n",
                    fn_name
                ));
                output.push_str(&format!(
                    "    crate::Svg::from_path({}_PATH, {}_VIEWBOX.0, {}_VIEWBOX.1)\n",
                    const_name, const_name, const_name
                ));
                output.push_str("}\n\n");
            }
        }

        let mut file = fs::File::create(&dest_path).expect("Failed to create output file");
        file.write_all(output.as_bytes())
            .expect("Failed to write output file");
    }
}

/// Extract viewBox width and height from SVG.
fn extract_viewbox(svg: &str) -> Option<(f32, f32)> {
    let vb_start = svg.find("viewBox=\"")? + 9;
    let vb_section = &svg[vb_start..];
    let vb_end = vb_section.find('"')?;
    let vb = &vb_section[..vb_end];

    let parts: Vec<&str> = vb.split_whitespace().collect();
    if parts.len() >= 4 {
        let width: f32 = parts[2].parse().ok()?;
        let height: f32 = parts[3].parse().ok()?;
        Some((width, height))
    } else {
        None
    }
}

/// Extract the d attribute from the first <path> element.
fn extract_path_d(svg: &str) -> Option<String> {
    let path_start = svg.find("<path")?;
    let path_section = &svg[path_start..];
    let d_start = path_section.find("d=\"")? + 3;
    let d_section = &path_section[d_start..];
    let d_end = d_section.find('"')?;
    Some(d_section[..d_end].to_string())
}

/// Convert kebab-case to SCREAMING_SNAKE_CASE.
fn to_const_name(name: &str) -> String {
    let name = if name.chars().next().map_or(false, |c| c.is_numeric()) {
        format!("ICON_{}", name)
    } else {
        name.to_string()
    };
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
