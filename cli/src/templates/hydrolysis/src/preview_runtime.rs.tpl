//! Hydrolysis preview runtime for {{ ctx.app_display_name }}.

use std::{env, fs, path::PathBuf};

use crate::preview_symbol;
use hydrolysis::HydrolysisViewRenderer;
use waterui_preview::{RenderResult, RenderResultExt as _, RenderSize, ViewRenderer};

pub(crate) fn run() {
    let output_path = PathBuf::from(required_env(preview_symbol::PREVIEW_OUTPUT_ENV));
    let width = parse_dimension(preview_symbol::PREVIEW_WIDTH_ENV);
    let height = parse_dimension(preview_symbol::PREVIEW_HEIGHT_ENV);

    let view = preview_symbol::load_preview_view();
    let renderer = ViewRenderer::new(HydrolysisViewRenderer::with_environment(
        preview_symbol::install_preview_theme,
    ));
    let mut render = pollster::block_on(renderer.render(view, RenderSize::new(width, height)));
    flatten_alpha_over_white(&mut render);
    let png_data = render
        .into_png()
        .unwrap_or_else(|error| panic!("hydrolysis preview: failed to encode PNG: {error}"));

    fs::write(&output_path, png_data).unwrap_or_else(|error| {
        panic!(
            "hydrolysis preview: failed to write `{}`: {error}",
            output_path.display()
        )
    });
}

fn required_env(name: &str) -> String {
    env::var(name)
        .unwrap_or_else(|error| panic!("hydrolysis preview: missing environment variable `{name}`: {error}"))
}

fn parse_dimension(name: &str) -> f32 {
    let raw = required_env(name);
    raw.parse::<f32>()
        .unwrap_or_else(|error| panic!("hydrolysis preview: invalid `{name}` value `{raw}`: {error}"))
}

fn flatten_alpha_over_white(render: &mut RenderResult) {
    for pixel in render.rgba_data.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        let inv_alpha = 255_u16
            .checked_sub(alpha)
            .expect("preview alpha channel must be <= 255");
        for channel in &mut pixel[..3] {
            let source = u16::from(*channel);
            let blended = source
                .checked_mul(alpha)
                .and_then(|value| value.checked_add(255_u16.checked_mul(inv_alpha)?))
                .and_then(|value| value.checked_add(127))
                .map(|value| value / 255)
                .expect("preview RGB blending overflowed");
            *channel = u8::try_from(blended).expect("preview RGB channel must fit into u8");
        }
        pixel[3] = 255;
    }
}
