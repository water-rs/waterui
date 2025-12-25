//! Build script for waterui-icons-material-icon
//!
//! Downloads icon metadata and SVG paths from jsdelivr CDN and generates
//! Rust code with icon definitions.
//!
//! # Data Sources
//!
//! - `meta.json`: Icon names and Unicode codepoints from `@mdi/svg`
//! - `mdi.js`: SVG path data from `@mdi/js`
//!
//! # Environment Variables
//!
//! - `MDI_META_URL`: Override URL or local file path for meta.json
//! - `MDI_PATH_URL`: Override URL or local file path for mdi.js

use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Material Design Icons version
const MDI_VERSION: &str = "7.4.47";

/// CDN URLs for icon data
const META_JSON_URL: &str = "https://cdn.jsdelivr.net/npm/@mdi/svg@7.4.47/meta.json";
const MDI_JS_URL: &str = "https://cdn.jsdelivr.net/npm/@mdi/js@7.4.47/mdi.js";

/// Icon metadata from meta.json
#[derive(Debug, Deserialize)]
struct IconMeta {
    name: String,
    codepoint: String,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MDI_META_URL");
    println!("cargo:rerun-if-env-changed=MDI_PATH_URL");

    let out_dir = env::var("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir);
    let meta_cache = out_path.join(format!("mdi-{MDI_VERSION}-meta.json"));
    let js_cache = out_path.join(format!("mdi-{MDI_VERSION}.js"));

    // Get URLs from env or use defaults
    let meta_url = env::var("MDI_META_URL").unwrap_or_else(|_| META_JSON_URL.to_string());
    let path_url = env::var("MDI_PATH_URL").unwrap_or_else(|_| MDI_JS_URL.to_string());

    // Get metadata from cache or download
    let meta_json = if meta_cache.exists() {
        fs::read_to_string(&meta_cache).expect("Failed to read cached meta.json")
    } else {
        let content = fetch_content(&meta_url, "MDI metadata").unwrap_or_else(|e| {
            fail_build(&out_dir, &format!("Failed to get MDI metadata: {e}"));
        });
        fs::write(&meta_cache, &content).ok();
        content
    };

    // Get SVG paths from cache or download
    let mdi_js = if js_cache.exists() {
        fs::read_to_string(&js_cache).expect("Failed to read cached mdi.js")
    } else {
        let content = fetch_content(&path_url, "MDI paths").unwrap_or_else(|e| {
            fail_build(&out_dir, &format!("Failed to get MDI paths: {e}"));
        });
        fs::write(&js_cache, &content).ok();
        content
    };

    // Parse metadata
    let icons: Vec<IconMeta> = serde_json::from_str(&meta_json).expect("Failed to parse meta.json");

    // Parse SVG paths from mdi.js
    let paths = parse_mdi_js(&mdi_js);

    // Generate icons.rs
    generate_icons_rs(&out_dir, &icons, &paths);
}

fn fail_build(out_dir: &str, msg: &str) -> ! {
    eprintln!("cargo:warning={msg}");
    eprintln!("cargo:warning=Building without icon definitions.");
    fs::write(Path::new(out_dir).join("icons.rs"), "// No icons available\n")
        .expect("Failed to write output file");
    std::process::exit(0);
}

/// Fetch content from URL or local file path
fn fetch_content(path: &str, desc: &str) -> Result<String, String> {
    if path.starts_with("http://") || path.starts_with("https://") {
        eprintln!("Downloading {desc} from {path}...");
        ureq::get(path)
            .call()
            .map_err(|e| format!("Download failed: {e}"))?
            .into_string()
            .map_err(|e| format!("Failed to read response: {e}"))
    } else {
        eprintln!("Reading {desc} from local file: {path}");
        fs::read_to_string(path).map_err(|e| format!("Failed to read file: {e}"))
    }
}

/// Parse SVG paths from mdi.js
///
/// Format: `export var mdiIconName = "M12,4...";`
fn parse_mdi_js(js: &str) -> HashMap<String, String> {
    let mut paths = HashMap::new();
    let re = Regex::new(r#"export var (mdi\w+) = "([^"]+)";"#).unwrap();

    for cap in re.captures_iter(js) {
        let var_name = &cap[1];
        let path = &cap[2];
        let icon_name = mdi_var_to_kebab(var_name);
        paths.insert(icon_name, path.to_string());
    }

    paths
}

/// Convert mdiIconName to icon-name
fn mdi_var_to_kebab(var_name: &str) -> String {
    let name = var_name.strip_prefix("mdi").unwrap_or(var_name);
    let mut result = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                result.push('-');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// Generate icons.rs with all icon definitions
fn generate_icons_rs(out_dir: &str, icons: &[IconMeta], paths: &HashMap<String, String>) {
    let dest_path = Path::new(out_dir).join("icons.rs");
    let mut output = String::new();

    output.push_str("// Auto-generated Material Design icon definitions.\n");
    output.push_str("// Icons: Apache 2.0 by Pictogrammers.\n");
    output.push_str("// Do not edit manually.\n\n");

    output.push_str("/// Font family name for Material Design Icons webfont.\n");
    output.push_str("#[cfg(feature = \"webfont\")]\n");
    output.push_str("pub const FONT_FAMILY: &str = \"Material Design Icons\";\n\n");

    let mut count = 0;
    for icon in icons {
        let Some(path) = paths.get(&icon.name) else {
            continue;
        };

        let const_name = to_const_name(&icon.name);
        let fn_name = to_fn_name(&icon.name);
        let codepoint = u32::from_str_radix(&icon.codepoint, 16).unwrap_or(0);

        output.push_str(&format!("/// `{}` icon as webfont glyph.\n", icon.name));
        output.push_str("#[cfg(feature = \"webfont\")]\n");
        output.push_str(&format!(
            "pub const {const_name}: crate::IconGlyph = crate::IconGlyph::new('\\u{{{codepoint:04X}}}', FONT_FAMILY);\n"
        ));

        output.push_str(&format!("/// SVG path for `{}`.\n", icon.name));
        output.push_str(&format!("pub const {const_name}_PATH: &str = {path:?};\n"));

        output.push_str("#[cfg(feature = \"svg\")]\n");
        output.push_str(&format!("/// `{}` icon as Svg.\n", icon.name));
        output.push_str("#[inline]\n");
        output.push_str(&format!(
            "pub fn {fn_name}() -> crate::Svg {{\n    crate::Svg::from_path({const_name}_PATH, 24.0, 24.0)\n}}\n"
        ));

        output.push('\n');
        count += 1;
    }

    eprintln!("Generated {count} icon definitions");

    fs::File::create(&dest_path)
        .and_then(|mut f| f.write_all(output.as_bytes()))
        .expect("Failed to write icons.rs");
}

/// Convert kebab-case to SCREAMING_SNAKE_CASE
fn to_const_name(name: &str) -> String {
    let name = if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("ICON_{name}")
    } else {
        name.to_string()
    };
    name.replace('-', "_").to_uppercase()
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
    "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
    "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
    "abstract", "become", "box", "do", "final", "macro", "override", "priv", "try",
    "typeof", "unsized", "virtual", "yield",
];

/// Convert kebab-case to snake_case, escaping keywords
fn to_fn_name(name: &str) -> String {
    let name = if name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("icon_{name}")
    } else {
        name.to_string()
    };
    let snake = name.replace('-', "_");

    if RUST_KEYWORDS.contains(&snake.as_str()) {
        format!("r#{snake}")
    } else {
        snake
    }
}
