//! Markdown blockquote bars, dividers, and inline code take their colours from
//! theme tokens rather than fixed colour literals.
#![cfg(feature = "flow-markdown")]

use waterui::ViewExt as _;
use waterui::color::Srgb;
use waterui::theme::{ColorScheme, ColorSettings, Theme};
use waterui::widget::RichText;
use waterui_testing::{TestStyle, ui};

fn render_markdown(markdown: &'static str, theme: Theme) -> waterui_testing::Snapshot {
    let mut app = ui()
        .viewport(360, 180)
        .theme(TestStyle::new())
        .install(theme)
        .mount_offscreen(move || RichText::from_markdown(markdown).padding_with(12.0));
    app.snapshot()
}

fn base_colors(border: Srgb, surface_variant: Srgb) -> ColorSettings {
    ColorSettings::new()
        .background(Srgb::new_u8(250, 250, 250))
        .surface(Srgb::new_u8(245, 245, 245))
        .foreground(Srgb::new_u8(20, 20, 20))
        .border(border)
        .surface_variant(surface_variant)
}

#[test]
fn markdown_quote_and_inline_code_change_between_light_and_dark() {
    const DOC: &str = "> Quoted line\n\nUse `cargo test` here.\n\n---";

    let light = render_markdown(DOC, Theme::new().color_scheme(ColorScheme::Light));
    let dark = render_markdown(DOC, Theme::new().color_scheme(ColorScheme::Dark));

    let metrics = light
        .compare_flip(&dark)
        .expect("light and dark snapshots share viewport dimensions")
        .metrics();
    assert!(
        metrics.flip_mean > 0.05,
        "expected markdown output to change between light and dark themes, got flip_mean {}",
        metrics.flip_mean
    );
}

#[test]
fn blockquote_bar_follows_border_token() {
    const DOC: &str = "> Quoted line";

    let neutral_surface_variant = Srgb::new_u8(232, 234, 237);
    let light_border = render_markdown(
        DOC,
        Theme::new().colors(base_colors(
            Srgb::new_u8(200, 200, 200),
            neutral_surface_variant,
        )),
    );
    let dark_border = render_markdown(
        DOC,
        Theme::new().colors(base_colors(
            Srgb::new_u8(20, 90, 210),
            neutral_surface_variant,
        )),
    );

    let metrics = light_border
        .compare_flip(&dark_border)
        .expect("blockquote snapshots share viewport dimensions")
        .metrics();
    assert!(
        metrics.flip_max > 0.1,
        "expected blockquote bar to change with the Border token, got flip_max {}",
        metrics.flip_max
    );
}

#[test]
fn inline_code_background_follows_surface_variant_token() {
    const DOC: &str = "Use `cargo test` here.";

    let neutral_border = Srgb::new_u8(160, 160, 160);
    let light_code = render_markdown(
        DOC,
        Theme::new().colors(base_colors(neutral_border, Srgb::new_u8(240, 240, 240))),
    );
    let dark_code = render_markdown(
        DOC,
        Theme::new().colors(base_colors(neutral_border, Srgb::new_u8(40, 60, 120))),
    );

    let metrics = light_code
        .compare_flip(&dark_code)
        .expect("inline code snapshots share viewport dimensions")
        .metrics();
    assert!(
        metrics.flip_max > 0.1,
        "expected inline code background to change with the SurfaceVariant token, got flip_max {}",
        metrics.flip_max
    );
}
