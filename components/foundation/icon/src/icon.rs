//! Unified [`Icon`] view and shared [`IconSize`] environment channel.

use waterui_core::plugin::Plugin;
use waterui_core::{Computed, Environment, IntoSignalF32, SignalExt as _, View};
#[cfg(feature = "svg")]
use waterui_core::{Str, anyview::AnyView};
#[cfg(feature = "svg")]
use waterui_layout::frame::Frame;
#[cfg(feature = "svg")]
use waterui_svg::Svg;

use crate::IconGlyph;

/// Default logical icon size in points when no [`IconSize`] plugin is installed.
pub const DEFAULT_ICON_SIZE: f32 = 24.0;

/// Shared icon-size channel installed in the [`Environment`].
///
/// Icons resolve their size from the nearest enclosing [`IconSize`] in the
/// environment, falling back to [`DEFAULT_ICON_SIZE`] (24 pt) when none is
/// installed.
#[derive(Debug, Clone)]
pub struct IconSize(Computed<f32>);

impl IconSize {
    /// Creates an [`IconSize`] channel from a constant or reactive `f32` signal.
    #[must_use]
    pub fn new(size: impl IntoSignalF32) -> Self {
        Self(size.into_signal_f32().computed())
    }

    /// Returns the reactive icon size signal in points.
    #[must_use]
    pub const fn size(&self) -> &Computed<f32> {
        &self.0
    }
}

impl Default for IconSize {
    fn default() -> Self {
        Self::new(DEFAULT_ICON_SIZE)
    }
}

impl Plugin for IconSize {}

#[derive(Debug, Clone)]
enum IconBacking {
    Glyph(IconGlyph),
    #[cfg(feature = "svg")]
    Svg {
        svg: Svg,
        viewbox: Option<(f32, f32)>,
    },
}

/// A portable UI icon returned by icon pack crates.
///
/// Every icon pack returns `Icon` from its `snake_case` constructor functions;
/// whether an icon draws from an SVG path or a webfont [`IconGlyph`] is
/// controlled by pack Cargo features rather than by distinct public types.
///
/// Colour comes from `.foreground(...)`, and size comes from [`IconSize`]
/// (or `.icon_size(...)`).
#[derive(Debug, Clone)]
#[must_use = "an `Icon` does nothing unless it is rendered as part of a view"]
pub struct Icon {
    backing: IconBacking,
}

impl Icon {
    /// Creates an [`Icon`] backed by a webfont [`IconGlyph`].
    pub const fn from_glyph(glyph: IconGlyph) -> Self {
        Self {
            backing: IconBacking::Glyph(glyph),
        }
    }

    /// Creates an [`Icon`] wrapping an [`Svg`].
    #[cfg(feature = "svg")]
    pub const fn from_svg(svg: Svg) -> Self {
        Self {
            backing: IconBacking::Svg { svg, viewbox: None },
        }
    }

    /// Creates an [`Icon`] from filled SVG path data and its viewbox dimensions.
    #[cfg(feature = "svg")]
    pub fn from_path(path_data: impl Into<Str>, width: f32, height: f32) -> Self {
        Self {
            backing: IconBacking::Svg {
                svg: Svg::from_path(path_data, width, height),
                viewbox: Some((width, height)),
            },
        }
    }

    /// Renders a path-backed [`Icon`] as strokes instead of filled shapes.
    #[cfg(feature = "svg")]
    pub fn stroked(self) -> Self {
        Self {
            backing: match self.backing {
                IconBacking::Glyph(glyph) => IconBacking::Glyph(glyph),
                IconBacking::Svg { svg, viewbox } => IconBacking::Svg {
                    svg: svg.stroked(),
                    viewbox,
                },
            },
        }
    }
}

impl From<IconGlyph> for Icon {
    fn from(glyph: IconGlyph) -> Self {
        Self::from_glyph(glyph)
    }
}

#[cfg(feature = "svg")]
impl From<Svg> for Icon {
    fn from(svg: Svg) -> Self {
        Self::from_svg(svg)
    }
}

#[cfg(feature = "svg")]
fn fit_viewbox(viewbox: Option<(f32, f32)>, size: f32) -> (f32, f32) {
    let Some((vw, vh)) = viewbox else {
        return (size, size);
    };
    let longest = vw.max(vh);
    if longest <= 0.0 || !longest.is_finite() {
        return (size, size);
    }
    let scale = size / longest;
    (vw * scale, vh * scale)
}

#[cfg(feature = "svg")]
impl View for Icon {
    fn body(self, env: &Environment) -> impl View {
        match self.backing {
            IconBacking::Glyph(glyph) => AnyView::new(glyph),
            IconBacking::Svg { svg, viewbox } => {
                if let Some(icon_size) = env.get::<IconSize>() {
                    let width = icon_size.size().map(move |s| fit_viewbox(viewbox, s).0);
                    let height = icon_size.size().map(move |s| fit_viewbox(viewbox, s).1);
                    return AnyView::new(Frame::new(svg).width(width).height(height));
                }
                if let Some((vw, vh)) = viewbox
                    && (vw > DEFAULT_ICON_SIZE || vh > DEFAULT_ICON_SIZE)
                {
                    let (w, h) = fit_viewbox(Some((vw, vh)), DEFAULT_ICON_SIZE);
                    return AnyView::new(
                        Frame::new(svg)
                            .ideal_width(w)
                            .ideal_height(h)
                            .max_width(w)
                            .max_height(h),
                    );
                }
                AnyView::new(svg)
            }
        }
    }
}

#[cfg(not(feature = "svg"))]
impl View for Icon {
    fn body(self, _env: &Environment) -> impl View {
        match self.backing {
            IconBacking::Glyph(glyph) => glyph,
        }
    }
}
