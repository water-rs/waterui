//! Build script for waterui-icons-fontawesome7
//!
//! Downloads icons.json from Font Awesome release and generates icon definitions
//! with both SVG path data and webfont codepoints.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Font Awesome version to download
const FA_VERSION: &str = "7.1.0";

/// Download URL for the desktop release (contains icons.json with SVG paths)
const FA_DESKTOP_URL: &str =
    "https://github.com/FortAwesome/Font-Awesome/releases/download/7.1.0/fontawesome-free-7.1.0-desktop.zip";

/// Icon metadata from icons.json
#[derive(Debug, Deserialize)]
struct IconData {
    /// Unicode codepoint as hex string (e.g., "f015")
    unicode: String,
    /// Available styles for this icon
    styles: Vec<String>,
    /// Which styles are free (not pro)
    free: Vec<String>,
    /// SVG data per style
    #[serde(default)]
    svg: HashMap<String, SvgData>,
}

/// SVG data for a specific style
#[derive(Debug, Deserialize)]
struct SvgData {
    /// Path d attribute
    path: String,
    /// ViewBox dimensions [minX, minY, width, height]
    #[serde(rename = "viewBox")]
    viewbox: Vec<f32>,
}

fn main() {
    // Only re-run if build.rs changes (downloaded data is cached)
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = env::var("OUT_DIR").unwrap();

    // Get icons.json content (download if needed)
    let icons_json = get_icons_json();

    // Parse icons.json
    let icons: HashMap<String, IconData> =
        serde_json::from_str(&icons_json).expect("Failed to parse icons.json");

    // Generate each style module
    for style in &["brands", "regular", "solid"] {
        let dest_path = Path::new(&out_dir).join(format!("{}.rs", style));
        let mut output = String::new();

        // Header
        output.push_str(&format!(
            "// Auto-generated Font Awesome 7 {} icons.\n",
            style
        ));
        output.push_str("// Icons: CC BY 4.0 by Fonticons, Inc.\n");
        output.push_str("// Do not edit manually.\n\n");

        // Re-export IconGlyph for webfont feature
        output.push_str("#[cfg(feature = \"webfont\")]\n");
        output.push_str("pub use waterui_icon::IconGlyph;\n\n");

        // Font family constant for this style
        let font_family = match *style {
            "solid" => "FontAwesome7Free-Solid",
            "regular" => "FontAwesome7Free-Regular",
            "brands" => "FontAwesome7Free-Brands",
            _ => "FontAwesome7Free-Solid",
        };

        output.push_str(&format!(
            "/// Font family name for {} icons.\n",
            style
        ));
        output.push_str(&format!(
            "pub const FONT_FAMILY: &str = \"{}\";\n\n",
            font_family
        ));

        // Collect icons for this style
        let mut style_icons: Vec<(&String, &IconData)> = icons
            .iter()
            .filter(|(_, data)| data.free.contains(&style.to_string()))
            .collect();
        style_icons.sort_by_key(|(name, _)| *name);

        for (icon_name, icon_data) in style_icons {
            let const_name = to_const_name(icon_name);
            let fn_name = to_fn_name(icon_name);

            // Parse unicode codepoint
            let codepoint = u32::from_str_radix(&icon_data.unicode, 16)
                .expect("Invalid unicode codepoint");

            // Generate webfont constant
            output.push_str(&format!("/// `{}` icon as webfont glyph.\n", icon_name));
            output.push_str("#[cfg(feature = \"webfont\")]\n");
            output.push_str(&format!(
                "pub const {}: IconGlyph = IconGlyph::new('\\u{{{:04x}}}', FONT_FAMILY);\n",
                const_name, codepoint
            ));

            // Generate SVG data if available
            if let Some(svg_data) = icon_data.svg.get(*style) {
                // Path data constant
                output.push_str(&format!("/// SVG path for `{}`.\n", icon_name));
                output.push_str(&format!(
                    "pub const {}_PATH: &str = {:?};\n",
                    const_name, svg_data.path
                ));

                // ViewBox dimensions (width, height from viewbox array)
                let width = svg_data.viewbox.get(2).copied().unwrap_or(512.0);
                let height = svg_data.viewbox.get(3).copied().unwrap_or(512.0);
                output.push_str(&format!(
                    "/// ViewBox dimensions for `{}`.\n",
                    icon_name
                ));
                output.push_str(&format!(
                    "pub const {}_VIEWBOX: (f32, f32) = ({:.1}, {:.1});\n",
                    const_name, width, height
                ));

                // Function returning Svg (for svg feature)
                output.push_str("#[cfg(feature = \"svg\")]\n");
                output.push_str(&format!("/// `{}` icon as Svg.\n", icon_name));
                output.push_str("#[inline]\n");
                output.push_str(&format!(
                    "pub fn {}() -> crate::Svg {{\n",
                    fn_name
                ));
                output.push_str(&format!(
                    "    crate::Svg::from_path({}_PATH, {}_VIEWBOX.0, {}_VIEWBOX.1)\n",
                    const_name, const_name, const_name
                ));
                output.push_str("}\n");
            }

            output.push('\n');
        }

        let mut file = fs::File::create(&dest_path).expect("Failed to create output file");
        file.write_all(output.as_bytes())
            .expect("Failed to write output file");
    }
}

/// Get icons.json content, downloading from Font Awesome release if not cached.
fn get_icons_json() -> String {
    let cache_dir = get_cache_dir();
    let cache_file = cache_dir.join(format!("fontawesome-{}-icons.json", FA_VERSION));

    // Return cached version if exists
    if cache_file.exists() {
        return fs::read_to_string(&cache_file).expect("Failed to read cached icons.json");
    }

    // Download the desktop release zip
    eprintln!("Downloading Font Awesome {} icons metadata...", FA_VERSION);

    let response = ureq::get(FA_DESKTOP_URL)
        .call()
        .expect("Failed to download Font Awesome release");

    let mut zip_data = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut zip_data)
        .expect("Failed to read response body");

    // Extract icons.json from the zip
    let cursor = std::io::Cursor::new(zip_data);
    let mut archive = zip::ZipArchive::new(cursor).expect("Failed to open zip archive");

    let icons_json_path = format!("fontawesome-free-{}-desktop/metadata/icons.json", FA_VERSION);
    let mut icons_file = archive
        .by_name(&icons_json_path)
        .expect("icons.json not found in archive");

    let mut icons_json = String::new();
    icons_file
        .read_to_string(&mut icons_json)
        .expect("Failed to read icons.json");

    // Cache for future builds
    fs::create_dir_all(&cache_dir).ok();
    fs::write(&cache_file, &icons_json).ok();

    eprintln!("Downloaded and cached icons.json ({} bytes)", icons_json.len());
    icons_json
}

/// Get cache directory for downloaded assets.
fn get_cache_dir() -> PathBuf {
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".water")
        .join("cache")
        .join("fontawesome")
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
