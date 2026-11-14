//! Build script for waterui-icons-fontawesome7
//!
//! Downloads icons.json from Font Awesome release and generates icon definitions
//! with both SVG path data and webfont codepoints.
//!
//! # Font Management
//!
//! Fonts are managed by the `WaterUI` CLI, which downloads them to `~/.water/cache/fontawesome/fonts/`
//! before compilation. This build script expects fonts to already be present.
//!
//! For docs.rs builds (detected via `DOCS_RS` env var), this script downloads fonts itself.
//!
//! # Environment Variables
//!
//! - `FONTAWESOME_ICONS_JSON`: Override with path to a local icons.json file
//! - `FONTAWESOME_CACHE_DIR`: Override cache directory (default: ~/.water/cache/fontawesome)
//! - `DOCS_RS`: When set, enables download mode for docs.rs builds

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use waterui_icons_codegen::icons::{
    FontFamily, GlyphConst, IconEntry, IconModule, SvgConst, ViewboxConst, write_icon_module,
};
use waterui_icons_codegen::{rust_const_name, rust_fn_name};

/// Font Awesome version
const FA_VERSION: &str = "7.1.0";

/// Download URL for the desktop release (contains icons.json with SVG paths)
const FA_DESKTOP_URL: &str = "https://github.com/FortAwesome/Font-Awesome/releases/download/7.1.0/fontawesome-free-7.1.0-desktop.zip";

/// SHA-256 of the archive at `FA_DESKTOP_URL`, from
/// `curl -L $FA_DESKTOP_URL | shasum -a 256`.
const FA_DESKTOP_SHA256: &str = "d9776d5e9983faf5c791b2d2d579c53cc566e17963d13e3a4c01fc43dfa2dfd2";

/// Icon metadata from icons.json
#[derive(Debug, Deserialize)]
struct IconData {
    /// Unicode codepoint as hex string (e.g., "f015")
    unicode: String,
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
    /// `ViewBox` dimensions [minX, minY, width, height]
    #[serde(rename = "viewBox")]
    viewbox: Vec<f32>,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=FONTAWESOME_ICONS_JSON");
    println!("cargo:rerun-if-env-changed=FONTAWESOME_CACHE_DIR");
    println!("cargo:rerun-if-env-changed=DOCS_RS");

    let out_dir = env::var("OUT_DIR").unwrap();
    let cache_dir = get_cache_dir();
    let is_docs_rs = env::var("DOCS_RS").is_ok();

    // Get icons.json content
    let icons_json = match get_icons_json(&cache_dir, is_docs_rs) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("cargo:warning=Failed to get Font Awesome icons: {e}");
            eprintln!("cargo:warning=Building without icon definitions.");
            // Generate empty modules
            for style in &["brands", "regular", "solid"] {
                let dest_path = Path::new(&out_dir).join(format!("{style}.rs"));
                fs::write(&dest_path, "// No icons available\n")
                    .expect("Failed to write output file");
            }
            return;
        }
    };

    // Parse icons.json
    let icons: HashMap<String, IconData> =
        serde_json::from_str(&icons_json).expect("Failed to parse icons.json");

    // Generate each style module
    for style in &["brands", "regular", "solid"] {
        generate_style_module(&out_dir, style, &icons);
    }
}

fn generate_style_module(out_dir: &str, style: &str, icons: &HashMap<String, IconData>) {
    // Font family constant - must match the actual font family name inside the OTF files.
    // Use fc-scan to check: fc-scan font.otf | grep "family:"
    let font_family = match style {
        "regular" => "Font Awesome 7 Free",
        "brands" => "Font Awesome 7 Brands",
        // "solid" and any unknown style fall back to the solid family.
        _ => "Font Awesome 7 Free Solid",
    };

    // Collect and sort icons
    let mut style_icons: Vec<(&String, &IconData)> = icons
        .iter()
        .filter(|(_, data)| data.free.contains(&style.to_string()))
        .collect();
    style_icons.sort_by_key(|(name, _)| *name);

    let mut entries = Vec::new();
    for (icon_name, icon_data) in style_icons {
        let const_name = rust_const_name(icon_name);
        let codepoint =
            u32::from_str_radix(&icon_data.unicode, 16).expect("Invalid unicode codepoint");

        let svg = icon_data.svg.get(style).map(|svg_data| {
            let path_const = format!("{const_name}_PATH");
            let viewbox_const = format!("{const_name}_VIEWBOX");
            let width = svg_data.viewbox.get(2).copied().unwrap_or(512.0);
            let height = svg_data.viewbox.get(3).copied().unwrap_or(512.0);
            let ctor = format!(
                "crate::Svg::from_path({path_const}, {viewbox_const}.0, {viewbox_const}.1)"
            );
            SvgConst {
                path_const,
                path_literal: format!("{:?}", svg_data.path),
                viewbox: Some(ViewboxConst {
                    const_name: viewbox_const,
                    literal: format!("({width:.1}, {height:.1})"),
                }),
                fn_name: rust_fn_name(icon_name),
                ctor,
            }
        });

        entries.push(IconEntry {
            source_name: icon_name.clone(),
            glyph: Some(GlyphConst {
                const_name,
                type_path: "IconGlyph".to_owned(),
                escape: format!("\\u{{{codepoint:04x}}}"),
            }),
            svg,
        });
    }

    let module = IconModule {
        banner: vec![
            format!("Auto-generated Font Awesome 7 {style} icons."),
            "Icons: CC BY 4.0 by Fonticons, Inc.".to_owned(),
        ],
        font_family: Some(FontFamily {
            doc: format!("Font family name for {style} icons."),
            name: font_family.to_owned(),
            cfg_webfont: false,
        }),
        reexport_glyph: true,
        icons: entries,
    };
    write_icon_module(out_dir, &format!("{style}.rs"), &module);
}

/// Get icons.json from cache or download (docs.rs only).
fn get_icons_json(cache_dir: &Path, is_docs_rs: bool) -> Result<String, String> {
    // Check for local override
    if let Ok(local_path) = env::var("FONTAWESOME_ICONS_JSON") {
        return fs::read_to_string(&local_path)
            .map_err(|e| format!("Failed to read {local_path}: {e}"));
    }

    let cache_file = cache_dir.join(format!("fontawesome-{FA_VERSION}-icons.json"));

    // Try cached version first
    if cache_file.exists() {
        return fs::read_to_string(&cache_file).map_err(|e| format!("Failed to read cache: {e}"));
    }

    // Only download on docs.rs
    if is_docs_rs {
        eprintln!("docs.rs build: downloading Font Awesome metadata...");
        return download_icons_json(cache_dir, &cache_file);
    }

    Err(format!(
        "Font Awesome icons not found in cache at {}. \
         Run `water build` to download fonts, or set FONTAWESOME_ICONS_JSON.",
        cache_file.display()
    ))
}

/// Download icons.json (for docs.rs builds).
fn download_icons_json(cache_dir: &Path, cache_file: &Path) -> Result<String, String> {
    let bytes = ureq::get(FA_DESKTOP_URL)
        .call()
        .map_err(|e| format!("Download failed: {e}"))?
        .into_body()
        .read_to_vec()
        .map_err(|e| format!("Failed to read response: {e}"))?;

    let actual_hash = hex::encode(Sha256::digest(&bytes));
    if actual_hash != FA_DESKTOP_SHA256 {
        return Err(format!(
            "Font Awesome archive hash mismatch: expected {FA_DESKTOP_SHA256}, got {actual_hash}"
        ));
    }

    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("Failed to open archive: {e}"))?;
    let entry_name = format!("fontawesome-free-{FA_VERSION}-desktop/metadata/icons.json");
    let mut content = String::new();
    archive
        .by_name(&entry_name)
        .map_err(|e| format!("Missing {entry_name} in archive: {e}"))?
        .read_to_string(&mut content)
        .map_err(|e| format!("Failed to read {entry_name}: {e}"))?;

    // Cache the content
    fs::create_dir_all(cache_dir).ok();
    fs::write(cache_file, &content).ok();

    Ok(content)
}

fn get_cache_dir() -> PathBuf {
    if let Ok(dir) = env::var("FONTAWESOME_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir()
        .expect("Could not determine home directory")
        .join(".water")
        .join("cache")
        .join("fontawesome")
}
