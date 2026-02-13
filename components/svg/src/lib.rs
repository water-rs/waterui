//! SVG rendering for WaterUI.
//!
//! This crate provides `Svg`, a view for rendering SVG content using GPU-accelerated
//! rendering via `GpuSurface`.
//!
//! # Backends
//!
//! - **CPU backend** (`cpu` feature, default): Uses resvg to rasterize SVGs
//!   to a texture which is then blitted to the GPU. Simple and widely compatible.
//!
//! - **GPU backend** (`vello` feature): Uses Vello for direct GPU vector rendering.
//!   Potentially better quality and performance, but requires a git dependency.

#![allow(clippy::multiple_crate_versions)]

extern crate alloc;

#[cfg(feature = "cpu")]
mod cpu_renderer;
#[cfg(feature = "vello-backend")]
mod vello_renderer;

use waterui_core::Signal;
use waterui_core::resolve::Resolvable;
use waterui_core::{Environment, View};
use waterui_graphics::GpuSurface;
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
pub struct Svg {
    /// SVG content (path data or full SVG markup).
    pub content: Str,
    /// Intrinsic width for aspect ratio.
    pub width: Option<f32>,
    /// Intrinsic height for aspect ratio.
    pub height: Option<f32>,
    /// Optional tint color (for monochrome icons).
    pub tint: Option<Color>,
    /// Whether to render as stroke (outline) rather than fill.
    pub stroke: bool,
}

impl Svg {
    /// Creates an SVG from raw SVG markup or path data.
    ///
    /// For icons, prefer `from_path` which provides explicit dimensions.
    #[must_use]
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
    #[must_use]
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
    #[must_use]
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
    #[must_use]
    pub fn tint(mut self, color: impl Into<Color>) -> Self {
        self.tint = Some(color.into());
        self
    }

    /// Sets explicit dimensions for the SVG.
    #[must_use]
    pub fn size(mut self, width: f32, height: f32) -> Self {
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
                // Already full SVG markup
                self.content.to_string()
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

    /// Creates a GpuSurface renderer for this SVG with the given color.
    #[cfg(feature = "cpu")]
    fn to_gpu_surface(&self, color: &str) -> GpuSurface {
        let svg_content = self.build_svg_content(color);
        GpuSurface::new(cpu_renderer::SvgRenderer::new(&svg_content)).on_demand()
    }

    #[cfg(all(feature = "vello-backend", not(feature = "cpu")))]
    fn to_gpu_surface(&self, color: &str) -> GpuSurface {
        let svg_content = self.build_svg_content(color);
        GpuSurface::new(vello_renderer::VelloSvgRenderer::new(&svg_content)).on_demand()
    }

    /// Format a ResolvedColor as an SVG-compatible hex string.
    ///
    /// Converts from linear RGB to sRGB and formats as #rrggbb.
    fn resolved_color_to_svg_hex(
        color: &waterui_graphics::color::ResolvedColor,
    ) -> alloc::string::String {
        let srgb = color.to_srgb();
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let r = (srgb.red * 255.0).clamp(0.0, 255.0) as u8;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let g = (srgb.green * 255.0).clamp(0.0, 255.0) as u8;
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let b = (srgb.blue * 255.0).clamp(0.0, 255.0) as u8;
        alloc::format!("#{r:02x}{g:02x}{b:02x}")
    }
}

impl View for Svg {
    fn body(self, env: &Environment) -> impl View {
        // Get the color to use: explicit tint or default to white (for dark themes)
        let color_hex = if let Some(tint) = &self.tint {
            // Use explicit tint color
            let resolved = tint.resolve(env).get();
            Svg::resolved_color_to_svg_hex(&resolved)
        } else {
            // Try to get foreground color from environment, fallback to white
            env.query::<waterui_graphics::color::ForegroundColor, waterui_graphics::color::ResolvedColor>()
                .map(|sig| {
                    let fg = sig.get();
                    Svg::resolved_color_to_svg_hex(&fg)
                })
                .unwrap_or_else(|| "#ffffff".into())
        };

        let surface = self.to_gpu_surface(&color_hex);
        // Apply frame with intrinsic dimensions if available
        match (self.width, self.height) {
            (Some(w), Some(h)) => Frame::new(surface).width(w).height(h),
            _ => Frame::new(surface),
        }
    }
}

// Re-export renderers for advanced usage
#[cfg(feature = "cpu")]
pub use cpu_renderer::SvgRenderer;
#[cfg(feature = "vello-backend")]
pub use vello_renderer::VelloSvgRenderer;
