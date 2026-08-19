use crate::{BarcodeMaskEffect, BarcodeRenderer, BarcodeSymbology};
use core::fmt;
use nami::{Computed, SignalExt as _, signal::IntoComputed};
use waterui_core::{
    AnyView, Environment, Str, View,
    accessibility::{AccessibilityLabel, AccessibilityRole},
    flatten_signal,
    layout::UnitPoint,
    metadata::IgnorableMetadata,
};
use waterui_graphics::{GpuSurface, GpuView, ViewEffect, color::Color};

/// Fill style for dark barcode modules.
///
/// Colors are kept as the unresolved [`Color`] type so that barcode views
/// participate in the same theme/HDR resolution flow as the rest of the
/// component library. The renderer observes those resolved colors for its
/// entire lifetime.
#[derive(Clone, Debug)]
pub enum BarcodeFill {
    /// Solid color fill.
    Solid(Computed<Color>),
    /// Linear gradient fill in normalized barcode-space coordinates.
    LinearGradient {
        /// Gradient start color.
        start_color: Computed<Color>,
        /// Gradient end color.
        end_color: Computed<Color>,
        /// Normalized gradient start point in barcode space.
        start_point: UnitPoint,
        /// Normalized gradient end point in barcode space.
        end_point: UnitPoint,
    },
}

impl BarcodeFill {
    /// Creates a reactive solid module fill.
    #[must_use]
    pub fn solid(color: impl IntoComputed<Color>) -> Self {
        Self::Solid(color.into_computed())
    }

    /// Creates a reactive linear-gradient module fill.
    #[must_use]
    pub fn linear_gradient(
        start_color: impl IntoComputed<Color>,
        end_color: impl IntoComputed<Color>,
        start_point: impl Into<UnitPoint>,
        end_point: impl Into<UnitPoint>,
    ) -> Self {
        Self::LinearGradient {
            start_color: start_color.into_computed(),
            end_color: end_color.into_computed(),
            start_point: start_point.into(),
            end_point: end_point.into(),
        }
    }
}

impl Default for BarcodeFill {
    fn default() -> Self {
        Self::solid(Color::from(waterui_graphics::color::Srgb::BLACK))
    }
}

fn default_light_color() -> Computed<Color> {
    Computed::constant(Color::from(waterui_graphics::color::Srgb::WHITE))
}

fn barcode_label(symbology: BarcodeSymbology, content: &Computed<Str>) -> AccessibilityLabel {
    let kind = match symbology {
        BarcodeSymbology::Qr => "QR code",
        BarcodeSymbology::Code128 => "Code 128 barcode",
    };
    AccessibilityLabel::new(content.map(move |content| Str::from(format!("{kind}: {content}"))))
}

fn apply_barcode_semantics(
    env: &Environment,
    symbology: BarcodeSymbology,
    content: &Computed<Str>,
    view: impl View,
) -> AnyView {
    let mut view = AnyView::new(view);
    if env.get::<AccessibilityRole>().is_none() {
        view = AnyView::new(IgnorableMetadata::new(view, AccessibilityRole::Image));
    }
    if env.get::<AccessibilityLabel>().is_none() {
        view = AnyView::new(IgnorableMetadata::new(
            view,
            barcode_label(symbology, content),
        ));
    }
    view
}

/// A view that renders a barcode.
///
/// `Barcode` is a high-performance, GPU-accelerated view that renders
/// QR/linear barcodes via fragment shader rasterization from a packed matrix
/// buffer.
///
/// # Example
///
/// ```rust
/// use waterui_barcode::Barcode;
///
/// Barcode::qr("https://waterui.dev");
/// Barcode::code128("HELLO-WATERUI");
/// ```
#[derive(Clone, Debug)]
pub struct Barcode {
    symbology: BarcodeSymbology,
    content: Computed<Str>,
    fill: BarcodeFill,
    light_color: Computed<Color>,
}

/// Barcode view filled by arbitrary GPU content.
pub struct BarcodeGpuFill<V: GpuView> {
    symbology: BarcodeSymbology,
    content: Computed<Str>,
    fill: V,
    light_color: Computed<Color>,
}

impl<V: GpuView> fmt::Debug for BarcodeGpuFill<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BarcodeGpuFill").finish_non_exhaustive()
    }
}

impl Barcode {
    /// Creates a QR code view.
    ///
    /// Content is a signal: a rotating token or live pairing URL re-encodes
    /// the matrix in place without rebuilding the view.
    ///
    /// # Panics
    ///
    /// The view panics while rendering when a content value exceeds QR
    /// capacity. Pre-validate runtime user input with [`BarcodeSource::qr`].
    ///
    /// [`BarcodeSource::qr`]: crate::BarcodeSource::qr
    pub fn qr(content: impl IntoComputed<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Qr,
            content: content.into_computed(),
            fill: BarcodeFill::default(),
            light_color: default_light_color(),
        }
    }

    /// Creates a Code128 barcode view.
    ///
    /// Content is a signal: a changing order id re-encodes the matrix in
    /// place without rebuilding the view.
    ///
    /// # Panics
    ///
    /// The view panics while rendering when a content value contains
    /// characters Code128 cannot represent. Pre-validate runtime user input
    /// with [`BarcodeSource::code128`].
    ///
    /// [`BarcodeSource::code128`]: crate::BarcodeSource::code128
    pub fn code128(content: impl IntoComputed<Str>) -> Self {
        Self {
            symbology: BarcodeSymbology::Code128,
            content: content.into_computed(),
            fill: BarcodeFill::default(),
            light_color: default_light_color(),
        }
    }

    /// Sets a solid dark module color.
    #[must_use]
    pub fn dark_color(mut self, color: impl IntoComputed<Color>) -> Self {
        self.fill = BarcodeFill::solid(color);
        self
    }

    /// Sets a linear gradient fill for dark modules.
    ///
    /// Gradient coordinates are normalized to the barcode square via
    /// [`UnitPoint`]: `UnitPoint::TOP_LEADING` = top-left,
    /// `UnitPoint::BOTTOM_TRAILING` = bottom-right.
    #[must_use]
    pub fn linear_gradient(
        mut self,
        start_color: impl IntoComputed<Color>,
        end_color: impl IntoComputed<Color>,
        start_point: impl Into<UnitPoint>,
        end_point: impl Into<UnitPoint>,
    ) -> Self {
        self.fill = BarcodeFill::linear_gradient(start_color, end_color, start_point, end_point);
        self
    }

    /// Sets the light module/background color.
    #[must_use]
    pub fn light_color(mut self, color: impl IntoComputed<Color>) -> Self {
        self.light_color = color.into_computed();
        self
    }

    /// Fills dark modules using arbitrary GPU-rendered content.
    ///
    /// Any type implementing `GpuView` can be passed directly.
    #[must_use]
    pub fn fill_gpu<V: GpuView>(self, fill: V) -> BarcodeGpuFill<V> {
        BarcodeGpuFill {
            symbology: self.symbology,
            content: self.content,
            fill,
            light_color: self.light_color,
        }
    }
}

impl<V: GpuView> BarcodeGpuFill<V> {
    /// Sets light module/background color for the masked barcode output.
    #[must_use]
    pub fn light_color(mut self, color: impl IntoComputed<Color>) -> Self {
        self.light_color = color.into_computed();
        self
    }
}

impl View for Barcode {
    fn body(self, env: &Environment) -> impl View {
        let Self {
            symbology,
            content,
            fill,
            light_color,
        } = self;
        let renderer = BarcodeRenderer::reactive(symbology, content.clone())
            .with_fill(fill)
            .with_light_color(light_color);
        apply_barcode_semantics(env, symbology, &content, GpuSurface::new(renderer))
    }
}

impl<V: GpuView> View for BarcodeGpuFill<V> {
    fn body(self, env: &Environment) -> impl View {
        let environment = env.clone();
        let resolved_light = flatten_signal(
            self.light_color
                .map(move |color| color.resolve(&environment)),
        );
        let effect = BarcodeMaskEffect::reactive(self.symbology, self.content.clone(), resolved_light);
        let fill_surface = GpuSurface::new(self.fill);
        apply_barcode_semantics(
            env,
            self.symbology,
            &self.content,
            ViewEffect::new(fill_surface, effect),
        )
    }
}

/// Free-function entry point for a QR code view.
///
/// See [`Barcode::qr`] for content semantics and panic conditions.
#[must_use]
pub fn qr_code(content: impl IntoComputed<Str>) -> Barcode {
    Barcode::qr(content)
}

/// Free-function entry point for a Code128 barcode view.
///
/// See [`Barcode::code128`] for content semantics and panic conditions.
#[must_use]
pub fn code128(content: impl IntoComputed<Str>) -> Barcode {
    Barcode::code128(content)
}
