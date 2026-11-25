//! Typed, askama-templated code generation for icon-set crates.
//!
//! Icon build scripts (`material`, `lucide`, `fontawesome7`, …) parse their
//! upstream data into an [`IconModule`] and call [`write_icon_module`], which
//! renders the shared [`icon_module.rs.txt`](../templates/icon_module.rs.txt)
//! template to `OUT_DIR`. The document layout lives entirely in the template;
//! only scalar leaf values (escaped path literals, constructor expressions,
//! glyph escapes) are precomputed in Rust.

use std::path::Path;

use askama::Template;

/// A complete generated icon module: banner, optional font-family constant,
/// optional `IconGlyph` re-export, and the per-icon definitions.
#[derive(Debug, Template)]
#[template(path = "icon_module.rs.txt", escape = "none")]
pub struct IconModule {
    /// Leading comment lines, each emitted verbatim as `// <line>`.
    pub banner: Vec<String>,
    /// Font-family constant emitted under the `webfont` feature, if any.
    pub font_family: Option<FontFamily>,
    /// When `true`, the module re-exports `waterui_icon::IconGlyph` under the
    /// `webfont` feature so generated glyph functions can name it unqualified.
    pub reexport_glyph: bool,
    /// The icons rendered into this module, in output order.
    pub icons: Vec<IconEntry>,
}

/// The `FONT_FAMILY` constant emitted for webfont-capable icon sets.
#[derive(Debug)]
pub struct FontFamily {
    /// Doc-comment text placed above the constant.
    pub doc: String,
    /// The string value of the `FONT_FAMILY` constant.
    pub name: String,
    /// When `true`, the constant is gated behind the `webfont` feature.
    pub cfg_webfont: bool,
}

/// A single icon: `snake_case` constructor returning `crate::Icon` backed by
/// SVG path data and/or a webfont glyph depending on pack features.
#[derive(Debug)]
pub struct IconEntry {
    /// Original (kebab-case) icon name, used in doc comments.
    pub source_name: String,
    /// Constructor function identifier, e.g. `r#ab_testing`.
    pub fn_name: String,
    /// Webfont glyph data, when the set ships codepoints.
    pub glyph: Option<GlyphConst>,
    /// SVG path/viewbox/constructor section, when the set ships vector data.
    pub svg: Option<SvgConst>,
}

/// Webfont glyph data for an icon constructor (`IconGlyph::new('...', FONT_FAMILY)`).
#[derive(Debug)]
pub struct GlyphConst {
    /// How `IconGlyph` is named here (`crate::IconGlyph` or `IconGlyph`).
    pub type_path: String,
    /// Pre-built unicode escape, e.g. `\u{F01C4}`.
    pub escape: String,
}

/// The SVG-backed part of an icon: path constant, optional viewbox constant,
/// and the `Icon` constructor expression.
#[derive(Debug)]
pub struct SvgConst {
    /// Path constant identifier, e.g. `AB_TESTING_PATH`.
    pub path_const: String,
    /// Pre-escaped Rust string literal for the path data (includes quotes).
    pub path_literal: String,
    /// Viewbox constant, when the set encodes per-icon viewboxes.
    pub viewbox: Option<ViewboxConst>,
    /// Full `crate::Icon` constructor expression placed in the function body.
    pub ctor: String,
}

/// A `pub const NAME_VIEWBOX: (f32, f32) = (w, h);` constant.
#[derive(Debug)]
pub struct ViewboxConst {
    /// Constant identifier, e.g. `STAR_VIEWBOX`.
    pub const_name: String,
    /// Pre-formatted tuple literal, e.g. `(512.0, 512.0)`.
    pub literal: String,
}

/// Renders `module` and writes it to `<out_dir>/<file_name>`.
///
/// # Panics
///
/// Panics if the template fails to render or the file cannot be written; both
/// are unrecoverable build-script errors.
pub fn write_icon_module(out_dir: &str, file_name: &str, module: &IconModule) {
    let rendered = module
        .render()
        .expect("failed to render icon module template");
    let dest = Path::new(out_dir).join(file_name);
    std::fs::write(&dest, rendered)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", dest.display()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(module: &IconModule) -> String {
        module.render().expect("render")
    }

    #[test]
    fn material_shape_emits_unified_icon_fn_for_svg_and_webfont() {
        let module = IconModule {
            banner: vec![
                "Auto-generated Material Design icon definitions.".to_owned(),
                "Do not edit manually.".to_owned(),
            ],
            font_family: Some(FontFamily {
                doc: "Font family name for Material Design Icons webfont.".to_owned(),
                name: "Material Design Icons".to_owned(),
                cfg_webfont: true,
            }),
            reexport_glyph: false,
            icons: vec![IconEntry {
                source_name: "ab-testing".to_owned(),
                fn_name: "r#ab_testing".to_owned(),
                glyph: Some(GlyphConst {
                    type_path: "crate::IconGlyph".to_owned(),
                    escape: "\\u{F01C4}".to_owned(),
                }),
                svg: Some(SvgConst {
                    path_const: "AB_TESTING_PATH".to_owned(),
                    path_literal: "\"M1,2\"".to_owned(),
                    viewbox: None,
                    ctor: "crate::Icon::from_path(AB_TESTING_PATH, 24.0, 24.0)".to_owned(),
                }),
            }],
        };
        let out = render(&module);
        assert!(out.contains("// Auto-generated Material Design icon definitions.\n"));
        assert!(out.contains(
            "#[cfg(feature = \"webfont\")]\npub const FONT_FAMILY: &str = \"Material Design Icons\";\n"
        ));
        assert!(out.contains("pub const AB_TESTING_PATH: &str = \"M1,2\";\n"));
        assert!(out.contains("#[cfg(feature = \"svg\")]\n/// `ab-testing` icon.\n#[inline]\npub fn r#ab_testing() -> crate::Icon {\n    crate::Icon::from_path(AB_TESTING_PATH, 24.0, 24.0)\n}\n"));
        assert!(out.contains("#[cfg(all(feature = \"webfont\", not(feature = \"svg\")))]\n/// `ab-testing` icon.\n#[inline]\npub const fn r#ab_testing() -> crate::Icon {\n    crate::Icon::from_glyph(crate::IconGlyph::new('\\u{F01C4}', FONT_FAMILY))\n}\n"));
        assert!(!out.contains("_VIEWBOX"));
        assert!(!out.contains("pub use waterui_icon::IconGlyph;"));
    }

    #[test]
    fn lucide_shape_has_path_and_stroke_fn_no_glyph() {
        let module = IconModule {
            banner: vec!["Auto-generated Lucide icon definitions.".to_owned()],
            font_family: Some(FontFamily {
                doc: "Font family name for Lucide webfont.".to_owned(),
                name: "lucide".to_owned(),
                cfg_webfont: true,
            }),
            reexport_glyph: false,
            icons: vec![IconEntry {
                source_name: "activity".to_owned(),
                fn_name: "r#activity".to_owned(),
                glyph: None,
                svg: Some(SvgConst {
                    path_const: "ACTIVITY_PATH".to_owned(),
                    path_literal: "\"M22 12\"".to_owned(),
                    viewbox: None,
                    ctor: "crate::Icon::from_path(ACTIVITY_PATH, 24.0, 24.0).stroked()".to_owned(),
                }),
            }],
        };
        let out = render(&module);
        assert!(out.contains("pub const ACTIVITY_PATH: &str = \"M22 12\";\n"));
        assert!(out.contains("    crate::Icon::from_path(ACTIVITY_PATH, 24.0, 24.0).stroked()\n"));
        assert!(!out.contains("IconGlyph"));
    }

    #[test]
    fn fontawesome_shape_emits_icon_fn_under_both_features_and_glyph_only() {
        let module = IconModule {
            banner: vec!["Auto-generated Font Awesome 7 solid icons.".to_owned()],
            font_family: Some(FontFamily {
                doc: "Font family name for solid icons.".to_owned(),
                name: "Font Awesome 7 Free Solid".to_owned(),
                cfg_webfont: false,
            }),
            reexport_glyph: true,
            icons: vec![
                IconEntry {
                    source_name: "star".to_owned(),
                    fn_name: "r#star".to_owned(),
                    glyph: Some(GlyphConst {
                        type_path: "IconGlyph".to_owned(),
                        escape: "\\u{f005}".to_owned(),
                    }),
                    svg: Some(SvgConst {
                        path_const: "STAR_PATH".to_owned(),
                        path_literal: "\"M0,0\"".to_owned(),
                        viewbox: Some(ViewboxConst {
                            const_name: "STAR_VIEWBOX".to_owned(),
                            literal: "(576.0, 512.0)".to_owned(),
                        }),
                        ctor: "crate::Icon::from_path(STAR_PATH, STAR_VIEWBOX.0, STAR_VIEWBOX.1)"
                            .to_owned(),
                    }),
                },
                // Glyph-only icon (no vector data): emits only the webfont function.
                IconEntry {
                    source_name: "glyph-only".to_owned(),
                    fn_name: "r#glyph_only".to_owned(),
                    glyph: Some(GlyphConst {
                        type_path: "IconGlyph".to_owned(),
                        escape: "\\u{f006}".to_owned(),
                    }),
                    svg: None,
                },
            ],
        };
        let out = render(&module);
        assert!(out.contains(
            "/// Font family name for solid icons.\npub const FONT_FAMILY: &str = \"Font Awesome 7 Free Solid\";\n"
        ));
        assert!(out.contains("#[cfg(feature = \"webfont\")]\npub use waterui_icon::IconGlyph;\n"));
        assert!(out.contains("/// SVG viewbox dimensions `(width, height)` for `star`.\npub const STAR_VIEWBOX: (f32, f32) = (576.0, 512.0);\n"));
        assert!(
            out.contains("    crate::Icon::from_path(STAR_PATH, STAR_VIEWBOX.0, STAR_VIEWBOX.1)\n")
        );
        assert!(out.contains(
            "#[cfg(all(feature = \"webfont\", not(feature = \"svg\")))]\n/// `star` icon.\n#[inline]\npub const fn r#star() -> crate::Icon {\n    crate::Icon::from_glyph(IconGlyph::new('\\u{f005}', FONT_FAMILY))\n}\n"
        ));
        assert!(out.contains(
            "#[cfg(feature = \"webfont\")]\n/// `glyph-only` icon.\n#[inline]\npub const fn r#glyph_only() -> crate::Icon {\n    crate::Icon::from_glyph(IconGlyph::new('\\u{f006}', FONT_FAMILY))\n}\n"
        ));
        assert!(!out.contains("GLYPH_ONLY_PATH"));
    }
}
