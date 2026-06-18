//! SVG rendering for `WaterUI`.
//!
//! This crate provides `Svg`, a view for rendering SVG content using GPU-accelerated
//! rendering via `SceneView`.
//!
//! SVG rendering is backed by Vello for direct GPU vector rendering.

extern crate alloc;

mod scene_renderer;
mod vello_renderer;

use waterui_core::{AnyView, Computed, Environment, Signal, SignalExt, View};
use waterui_graphics::SceneView;
use waterui_graphics::color::Color;
use waterui_layout::frame::Frame;
use waterui_str::Str;

/// A view for rendering SVG content using GPU-accelerated rendering.
///
/// The SVG data can be either:
/// - Full SVG markup
/// - Path data only (d attribute from SVG path element)
///
/// # Example
///
/// ```ignore
/// // From SVG path data (most common for icons)
/// Svg::from_path("M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z", 24.0, 24.0)
///
/// // Stroke-based icons (like Lucide)
/// Svg::from_stroke_path("M3 12h18M3 6h18M3 18h18", 24.0, 24.0)
/// ```
#[derive(Debug, Clone)]
#[must_use = "an `Svg` does nothing unless it is rendered as part of a view"]
pub struct Svg {
    /// SVG content (path data or full SVG markup).
    pub(crate) content: Str,
    /// Intrinsic width for aspect ratio.
    pub(crate) width: Option<f32>,
    /// Intrinsic height for aspect ratio.
    pub(crate) height: Option<f32>,
    /// Optional tint color (for monochrome icons).
    pub(crate) tint: Option<Color>,
    /// Whether to render as stroke (outline) rather than fill.
    pub(crate) stroke: bool,
}

impl Svg {
    /// Creates an SVG from raw SVG markup or path data.
    ///
    /// For icons, prefer `from_path` which provides explicit dimensions.
    pub fn new(content: impl Into<Str>) -> Self {
        Self {
            content: content.into(),
            width: None,
            height: None,
            tint: None,
            stroke: false,
        }
    }

    /// Creates an SVG from path data with explicit dimensions.
    ///
    /// This is the recommended constructor for filled icon SVGs where the
    /// path data comes from the `d` attribute of an SVG path element.
    pub fn from_path(path_data: impl Into<Str>, width: f32, height: f32) -> Self {
        Self {
            content: path_data.into(),
            width: Some(width),
            height: Some(height),
            tint: None,
            stroke: false,
        }
    }

    /// Creates an SVG from path data rendered as strokes (outlines).
    ///
    /// This is for stroke-based icon sets like Lucide where the path
    /// represents the outline of the icon, not a filled shape.
    ///
    /// The stroke uses:
    /// - `stroke-width: 2`
    /// - `stroke-linecap: round`
    /// - `stroke-linejoin: round`
    /// - `fill: none`
    pub fn from_stroke_path(path_data: impl Into<Str>, width: f32, height: f32) -> Self {
        Self {
            content: path_data.into(),
            width: Some(width),
            height: Some(height),
            tint: None,
            stroke: true,
        }
    }

    /// Sets the tint color for the SVG.
    pub fn tint(mut self, color: impl Into<Color>) -> Self {
        self.tint = Some(color.into());
        self
    }

    /// Sets explicit dimensions for the SVG.
    pub const fn size(mut self, width: f32, height: f32) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    /// Build full SVG document from path data if needed.
    ///
    /// The `color` parameter specifies the fill or stroke color for the SVG.
    fn build_svg_content(&self, color: &str) -> alloc::string::String {
        if let (Some(width), Some(height)) = (self.width, self.height) {
            let content = self.content.as_str();
            if content.trim_start().starts_with('<') {
                // Full SVG markup: support tint only for currentColor-driven assets.
                if content.contains("currentColor") {
                    content.replace("currentColor", color)
                } else {
                    self.content.to_string()
                }
            } else if self.stroke {
                // Stroke-based path (for outline icons like Lucide)
                alloc::format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" fill="none" stroke="{color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="{content}"/></svg>"#
                )
            } else {
                // Filled path data - wrap in SVG document
                alloc::format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" fill="{color}"><path d="{content}"/></svg>"#
                )
            }
        } else {
            // Assume full SVG content
            self.content.to_string()
        }
    }

    /// Creates a `SceneView` renderer for this SVG with the given color.
    fn to_scene_view(&self, color: &str) -> SceneView {
        let svg_content = self.build_svg_content(color);
        SceneView::new(scene_renderer::SvgSceneContent::new(&svg_content))
    }

    /// Creates a reactive `SceneView` renderer for this SVG.
    fn to_reactive_scene_view<S>(&self, color_signal: S) -> impl View
    where
        S: Signal<Output = alloc::string::String> + 'static,
        S::Guard: 'static,
    {
        let svg_template = self.build_svg_content(vello_renderer::SVG_COLOR_PLACEHOLDER);
        SceneView::new(scene_renderer::ReactiveSvgSceneContent::new(
            svg_template,
            color_signal,
        ))
    }

    /// Wraps a view in a frame, preserving optional intrinsic size.
    fn frame_view(&self, view: impl View) -> AnyView {
        match (self.width, self.height) {
            (Some(w), Some(h)) => AnyView::new(Frame::new(view).width(w).height(h)),
            _ => AnyView::new(Frame::new(view)),
        }
    }

    /// Creates a framed SVG scene view for the given color.
    fn to_framed_scene_view(&self, color: &str) -> AnyView {
        self.frame_view(self.to_scene_view(color))
    }

    /// Creates a framed reactive SVG scene view.
    fn to_reactive_framed_scene_view<S>(&self, color_signal: S) -> AnyView
    where
        S: Signal<Output = alloc::string::String> + 'static,
        S::Guard: 'static,
    {
        self.frame_view(self.to_reactive_scene_view(color_signal))
    }

    /// Format a `ResolvedColor` as an SVG-compatible color string.
    ///
    /// Converts from linear RGB to sRGB.
    /// - Opaque colors are emitted as `#rrggbb`.
    /// - Translucent colors are emitted as `rgba(r,g,b,a)`.
    fn resolved_color_to_svg_color(
        color: &waterui_graphics::color::ResolvedColor,
    ) -> alloc::string::String {
        let srgb = color.to_srgb();
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let r = (srgb.red * 255.0).clamp(0.0, 255.0) as u8;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let g = (srgb.green * 255.0).clamp(0.0, 255.0) as u8;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let b = (srgb.blue * 255.0).clamp(0.0, 255.0) as u8;
        let alpha = color.opacity.clamp(0.0, 1.0);
        if alpha >= 0.999_999 {
            alloc::format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            alloc::format!("rgba({r},{g},{b},{alpha:.6})")
        }
    }
}

impl View for Svg {
    fn body(self, env: &Environment) -> impl View {
        if let Some(tint) = self.tint.clone() {
            let color_signal = tint
                .resolve(env)
                .map(|resolved| Self::resolved_color_to_svg_color(&resolved));
            return self.to_reactive_framed_scene_view(color_signal);
        }

        if let Some(color_signal) = env
            .query::<
                waterui_graphics::color::ForegroundColor,
                Computed<waterui_graphics::color::ResolvedColor>,
            >()
            .cloned()
        {
            return self.to_reactive_framed_scene_view(
                color_signal.map(|resolved| Self::resolved_color_to_svg_color(&resolved)),
            );
        }

        self.to_framed_scene_view("#000000")
    }
}

// Re-export renderer for advanced usage.
pub use vello_renderer::VelloSvgRenderer;
