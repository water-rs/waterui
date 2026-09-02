//! Build script for waterui-icons-lucide
//!
//! Generates Rust code with icon definitions from the icon data vendored in
//! this crate's `data/` directory. A plain checkout never touches the network:
//! the pinned data is tracked in the repository so the build is offline and
//! deterministic, and a missing vendored file is a hard error.
//!
//! # Data Sources
//!
//! - `icon-nodes.json`: Icon definitions with SVG element data
//!
//! # Environment Variables
//!
//! This exists for re-vendoring a new upstream version and takes precedence
//! over the vendored file when set.
//!
//! - `LUCIDE_ICON_URL`: Override URL or local file path for icon-nodes.json

use std::collections::HashMap;
use std::env;
use std::path::Path;
use waterui_icons_codegen::icons::{
    FontFamily, IconEntry, IconModule, SvgConst, write_icon_module,
};
use waterui_icons_codegen::{
    is_http_url, load_cached_text, resolve_source, rust_const_name, rust_fn_name,
};

/// Lucide icons version
const LUCIDE_VERSION: &str = "0.562.0";

/// CDN URL for icon data
const ICON_NODES_URL: &str = "https://cdn.jsdelivr.net/npm/lucide-static@0.562.0/icon-nodes.json";

/// Parsed `icon-nodes.json`: icon name -> list of `(element_type, attributes)`.
type IconNodes = HashMap<String, Vec<(String, HashMap<String, serde_json::Value>)>>;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LUCIDE_ICON_URL");

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR must be set by Cargo");
    let out_path = Path::new(&out_dir);
    let cache_file = out_path.join(format!("lucide-{LUCIDE_VERSION}-icon-nodes.json"));

    let vendored = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join(format!("lucide-{LUCIDE_VERSION}-icon-nodes.json"));
    println!("cargo:rerun-if-changed={}", vendored.display());

    // Vendored data is the only default, so the build is deterministic and
    // needs no network. The override exists for re-vendoring a new version and
    // can point to a local file path or a URL.
    let source = resolve_source(&vendored, "LUCIDE_ICON_URL", ICON_NODES_URL)
        .unwrap_or_else(|error| panic!("waterui-icons-lucide: {error}"));

    if !is_http_url(&source) {
        println!("cargo:rerun-if-changed={source}");
    }

    let icon_nodes_json = load_cached_text(&source, &cache_file, "Lucide icon-nodes.json").unwrap_or_else(|e| {
        panic!(
            "waterui-icons-lucide: {e}\nHint: provide vendored data in `data/` or set `LUCIDE_ICON_URL` to a local file path/URL."
        )
    });

    // Parse icon nodes
    let icons: IconNodes =
        serde_json::from_str(&icon_nodes_json).expect("Failed to parse icon-nodes.json");

    // Generate icons.rs
    generate_icons_rs(&out_dir, &icons);
}

/// Parse a JSON value as a number (handles both Number and String variants)
fn parse_number(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Extract SVG path from icon elements
fn extract_path(elements: &[(String, HashMap<String, serde_json::Value>)]) -> Option<String> {
    let mut paths = Vec::new();

    for (element_type, attrs) in elements {
        if element_type == "path" {
            if let Some(serde_json::Value::String(d)) = attrs.get("d") {
                paths.push(d.clone());
            }
        } else if element_type == "circle" {
            // Parse cx, cy, r - can be either Number or String in JSON
            let cx = attrs.get("cx").and_then(parse_number);
            let cy = attrs.get("cy").and_then(parse_number);
            let r = attrs.get("r").and_then(parse_number);

            if let (Some(cx), Some(cy), Some(r)) = (cx, cy, r) {
                // Approximate circle with four cubic Bezier curves
                let k = 0.552_284_749_8;
                paths.push(format!(
                    "M{},{} C{},{},{},{},{},{} C{},{},{},{},{},{} C{},{},{},{},{},{} C{},{},{},{},{},{}Z",
                    cx - r, cy,
                    cx - r, r.mul_add(-k, cy), r.mul_add(-k, cx), cy - r, cx, cy - r,
                    r.mul_add(k, cx), cy - r, cx + r, r.mul_add(-k, cy), cx + r, cy,
                    cx + r, r.mul_add(k, cy), r.mul_add(k, cx), cy + r, cx, cy + r,
                    r.mul_add(-k, cx), cy + r, cx - r, r.mul_add(k, cy), cx - r, cy
                ));
            }
        } else if element_type == "rect" {
            // Parse rect attributes - can be either Number or String
            let x = attrs.get("x").and_then(parse_number).unwrap_or(0.0);
            let y = attrs.get("y").and_then(parse_number).unwrap_or(0.0);
            let w = attrs.get("width").and_then(parse_number);
            let h = attrs.get("height").and_then(parse_number);

            if let (Some(w), Some(h)) = (w, h) {
                let rx = attrs.get("rx").and_then(parse_number).unwrap_or(0.0);
                let ry = attrs.get("ry").and_then(parse_number).unwrap_or(rx);

                if rx > 0.0 || ry > 0.0 {
                    paths.push(format!(
                        "M{},{} H{} A{},{},0,0,1,{},{} V{} A{},{},0,0,1,{},{} H{} A{},{},0,0,1,{},{} V{} A{},{},0,0,1,{},{}Z",
                        x + rx, y,
                        x + w - rx,
                        rx, ry, x + w, y + ry,
                        y + h - ry,
                        rx, ry, x + w - rx, y + h,
                        x + rx,
                        rx, ry, x, y + h - ry,
                        y + ry,
                        rx, ry, x + rx, y
                    ));
                } else {
                    paths.push(format!("M{},{} H{} V{} H{} Z", x, y, x + w, y + h, x));
                }
            }
        } else if element_type == "line" {
            // Parse line attributes - can be either Number or String
            let x1 = attrs.get("x1").and_then(parse_number);
            let y1 = attrs.get("y1").and_then(parse_number);
            let x2 = attrs.get("x2").and_then(parse_number);
            let y2 = attrs.get("y2").and_then(parse_number);

            if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (x1, y1, x2, y2) {
                paths.push(format!("M{x1},{y1} L{x2},{y2}"));
            }
        } else if (element_type == "polyline" || element_type == "polygon")
            && let Some(serde_json::Value::String(points)) = attrs.get("points")
        {
            let coords: Vec<&str> = points.split_whitespace().collect();
            if !coords.is_empty() {
                let mut path = String::new();
                for (i, coord) in coords.iter().enumerate() {
                    path.push_str(if i == 0 { "M" } else { "L" });
                    path.push_str(coord);
                    path.push(' ');
                }
                if element_type == "polygon" {
                    path.push('Z');
                }
                paths.push(path.trim().to_string());
            }
        }
    }

    if paths.is_empty() {
        None
    } else {
        Some(paths.join(" "))
    }
}

/// Generate icons.rs with all icon definitions
fn generate_icons_rs(out_dir: &str, icons: &IconNodes) {
    let mut icon_names: Vec<_> = icons.keys().collect();
    icon_names.sort();

    let mut entries = Vec::new();
    for icon_name in icon_names {
        let elements = &icons[icon_name];
        let Some(path) = extract_path(elements) else {
            continue;
        };

        let const_name = rust_const_name(icon_name);
        let path_const = format!("{const_name}_PATH");
        let ctor = format!("crate::Svg::from_stroke_path({path_const}, 24.0, 24.0)");
        entries.push(IconEntry {
            source_name: icon_name.clone(),
            glyph: None,
            svg: Some(SvgConst {
                path_const,
                path_literal: format!("{path:?}"),
                viewbox: None,
                fn_name: rust_fn_name(icon_name),
                ctor,
            }),
        });
    }

    let count = entries.len();
    let module = IconModule {
        banner: vec![
            "Auto-generated Lucide icon definitions.".to_owned(),
            "Icons: ISC license by Lucide Contributors.".to_owned(),
            "Do not edit manually.".to_owned(),
        ],
        font_family: Some(FontFamily {
            doc: "Font family name for Lucide webfont.".to_owned(),
            name: "lucide".to_owned(),
            cfg_webfont: true,
        }),
        reexport_glyph: false,
        icons: entries,
    };
    write_icon_module(out_dir, "icons.rs", &module);

    eprintln!("Generated {count} icon definitions");
}
