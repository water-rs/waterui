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
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::Path;
use waterui_icons_codegen::icons::{
    FontFamily, GlyphConst, IconEntry, IconModule, SvgConst, write_icon_module,
};
use waterui_icons_codegen::{is_http_url, load_cached_text, rust_const_name, rust_fn_name};

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

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo");
    let out_path = Path::new(&out_dir);
    let meta_cache = out_path.join(format!("mdi-{MDI_VERSION}-meta.json"));
    let js_cache = out_path.join(format!("mdi-{MDI_VERSION}.js"));

    let vendored_meta = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(format!("mdi-{MDI_VERSION}-meta.json"));
    let vendored_js = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(format!("mdi-{MDI_VERSION}.js"));

    println!("cargo:rerun-if-changed={}", vendored_meta.display());
    println!("cargo:rerun-if-changed={}", vendored_js.display());

    // Default to vendored data for deterministic, offline builds.
    // Overrides can point to a local file path or a URL.
    let default_meta_source = if vendored_meta.exists() {
        vendored_meta.display().to_string()
    } else {
        META_JSON_URL.to_string()
    };
    let default_path_source = if vendored_js.exists() {
        vendored_js.display().to_string()
    } else {
        MDI_JS_URL.to_string()
    };

    let meta_source = env::var("MDI_META_URL").unwrap_or(default_meta_source);
    let path_source = env::var("MDI_PATH_URL").unwrap_or(default_path_source);

    if !is_http_url(&meta_source) {
        println!("cargo:rerun-if-changed={meta_source}");
    }
    if !is_http_url(&path_source) {
        println!("cargo:rerun-if-changed={path_source}");
    }

    let meta_json = load_cached_text(&meta_source, &meta_cache, "MDI metadata").unwrap_or_else(|e| {
        panic!(
            "waterui-icons-material-icon: {e}\nHint: provide vendored data in `data/` or set `MDI_META_URL` to a local file path/URL."
        )
    });
    let mdi_js = load_cached_text(&path_source, &js_cache, "MDI paths").unwrap_or_else(|e| {
        panic!(
            "waterui-icons-material-icon: {e}\nHint: provide vendored data in `data/` or set `MDI_PATH_URL` to a local file path/URL."
        )
    });

    // Parse metadata
    let icons: Vec<IconMeta> = serde_json::from_str(&meta_json).expect("Failed to parse meta.json");

    // Parse SVG paths from mdi.js
    let paths = parse_mdi_js(&mdi_js);

    // Generate icons.rs
    generate_icons_rs(&out_dir, &icons, &paths);
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
    let path_const_names = icons
        .iter()
        .filter(|icon| paths.contains_key(&icon.name))
        .map(|icon| format!("{}_PATH", rust_const_name(&icon.name)))
        .collect::<HashSet<_>>();
    let mut used_symbols = HashSet::new();

    let mut entries = Vec::new();
    for icon in icons {
        let Some(path) = paths.get(&icon.name) else {
            continue;
        };

        let const_name = rust_const_name(&icon.name);
        let glyph_const_name = if path_const_names.contains(&const_name) {
            format!("{const_name}_GLYPH")
        } else {
            const_name.clone()
        };
        let path_const = format!("{const_name}_PATH");
        let codepoint = u32::from_str_radix(&icon.codepoint, 16).unwrap_or(0);

        assert!(
            used_symbols.insert(glyph_const_name.clone()),
            "duplicate generated glyph constant `{glyph_const_name}` for material icon `{}`",
            icon.name
        );
        assert!(
            used_symbols.insert(path_const.clone()),
            "duplicate generated SVG path constant `{path_const}` for material icon `{}`",
            icon.name
        );

        let ctor = format!("crate::Svg::from_path({path_const}, 24.0, 24.0)");
        entries.push(IconEntry {
            source_name: icon.name.clone(),
            glyph: Some(GlyphConst {
                const_name: glyph_const_name,
                type_path: "crate::IconGlyph".to_owned(),
                escape: format!("\\u{{{codepoint:04X}}}"),
            }),
            svg: Some(SvgConst {
                path_const,
                path_literal: format!("{path:?}"),
                viewbox: None,
                fn_name: rust_fn_name(&icon.name),
                ctor,
            }),
        });
    }

    let count = entries.len();
    let module = IconModule {
        banner: vec![
            "Auto-generated Material Design icon definitions.".to_owned(),
            "Icons: Apache 2.0 by Pictogrammers.".to_owned(),
            "Do not edit manually.".to_owned(),
        ],
        font_family: Some(FontFamily {
            doc: "Font family name for Material Design Icons webfont.".to_owned(),
            name: "Material Design Icons".to_owned(),
            cfg_webfont: true,
        }),
        reexport_glyph: false,
        icons: entries,
    };
    write_icon_module(out_dir, "icons.rs", &module);

    eprintln!("Generated {count} icon definitions");
}
