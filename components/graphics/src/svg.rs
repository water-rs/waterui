//! SVG component for vector graphics rendering.
//!
//! This module provides `Svg`, a view for rendering SVG content.
//!
//! # Rendering Modes
//!
//! ## Native Rendering (default)
//!
//! By default, `Svg` is a raw view that passes SVG data to native backends:
//! - Apple: CAShapeLayer with CGPath
//! - Android: VectorDrawable or android.graphics.Path
//!
//! ## Rust-side Rendering (with `svg-render` feature)
//!
//! With the `svg-render` feature enabled, you can use `Svg::render()` to create
//! a `GpuSurface` that renders the SVG using `resvg` in Rust. This avoids
//! native SVG parsing issues and provides consistent rendering across platforms.
//!
//! ```ignore
//! // Rust-side rendering with resvg
//! Svg::new(svg_content).render()
//! ```

use crate::color::Color;
use waterui_core::raw_view;
use waterui_str::Str;

/// A native view for rendering SVG content.
///
/// The SVG data can be either:
/// - Full SVG markup (parsed by native backend)
/// - Path data only (d attribute from SVG path element)
///
/// Native backends render using platform-native vector graphics for
/// optimal performance and quality.
///
/// # Example
///
/// ```ignore
/// // From SVG path data (most common for icons)
/// Svg::from_path("M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z", 24.0, 24.0)
///
/// // With tint color
/// Svg::from_path("M10 20v-6h4v6h5v-8h3L12 3 2 12h3v8z", 24.0, 24.0)
///     .tint(Color::BLUE)
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
        }
    }

    /// Creates an SVG from path data with explicit dimensions.
    ///
    /// This is the recommended constructor for icon SVGs where the
    /// path data comes from the `d` attribute of an SVG path element.
    ///
    /// # Arguments
    ///
    /// * `path_data` - The SVG path data (d attribute)
    /// * `width` - Intrinsic width (typically from viewBox)
    /// * `height` - Intrinsic height (typically from viewBox)
    #[must_use]
    pub fn from_path(path_data: impl Into<Str>, width: f32, height: f32) -> Self {
        Self {
            content: path_data.into(),
            width: Some(width),
            height: Some(height),
            tint: None,
        }
    }

    /// Sets the tint color for the SVG.
    ///
    /// When set, the SVG is rendered as a solid color mask, ignoring
    /// any fill/stroke colors in the original SVG. This is ideal for
    /// monochrome icons.
    #[must_use]
    pub fn tint(mut self, color: impl Into<Color>) -> Self {
        self.tint = Some(color.into());
        self
    }

    /// Sets explicit dimensions for the SVG.
    ///
    /// These dimensions define the intrinsic size and aspect ratio
    /// of the SVG content.
    #[must_use]
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    /// Converts this SVG to a GPU-rendered view using resvg.
    ///
    /// Instead of passing SVG data to native backends, this creates a
    /// `GpuSurface` that renders the SVG in Rust using resvg. This provides:
    /// - Consistent rendering across all platforms
    /// - No reliance on potentially buggy native SVG parsers
    /// - Automatic re-rendering on resize for crisp display
    ///
    /// # Note
    ///
    /// The `tint` modifier is not applied in Rust-side rendering.
    /// To colorize an SVG, set fill/stroke colors in the SVG content directly.
    ///
    /// # Feature
    ///
    /// Requires the `svg-render` feature to be enabled.
    ///
    /// # Example
    ///
    /// ```ignore
    /// Svg::new(svg_content).render()
    /// ```
    #[cfg(feature = "svg-render")]
    #[must_use]
    pub fn render(self) -> crate::GpuSurface {
        use crate::{GpuSurface, SvgRenderer};

        // Build full SVG document if we only have path data
        let svg_content = if let (Some(width), Some(height)) = (self.width, self.height) {
            // Check if content looks like path data (starts with M, m, or is just commands)
            let content = self.content.as_str();
            if content.trim_start().starts_with('<') {
                // Already full SVG markup
                self.content.to_string()
            } else {
                // Path data only - wrap in SVG document
                alloc::format!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}"><path d="{}"/></svg>"#,
                    width, height, content
                )
            }
        } else {
            // Assume full SVG content
            self.content.to_string()
        };

        GpuSurface::new(SvgRenderer::new(&svg_content))
    }
}

// Svg is content-sized by default (uses intrinsic dimensions)
raw_view!(Svg);
