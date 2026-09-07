//! The [`QrCode`] view and the scene content behind it.

use alloc::rc::Rc;
use alloc::string::{String, ToString as _};

use kurbo::{Affine, BezPath, Rect, Shape as _};
use nami::SignalExt as _;
use nami::signal::IntoComputed;
use nami::watcher::BoxWatcherGuard;
use peniko::{Brush, Fill};
use waterui_core::layout::Size;
use waterui_core::resolve::Resolvable as _;
use waterui_core::{Computed, Environment, Signal as _, View};
use waterui_graphics::color::{Color, ForegroundColor, ResolvedColor, SurfaceColor, signal_color};
use waterui_graphics::{Scene2D, SceneContent, SceneInvalidator, SceneView, invalidate_on_change};
use waterui_layout::frame::Frame;
use waterui_str::Str;

use crate::matrix::{ErrorCorrection, QrMatrix};

/// The quiet zone ISO/IEC 18004 requires around a symbol, in modules.
///
/// Four modules of clear space on every side is what tells a decoder where the
/// symbol ends; a code pressed against neighbouring content is a code that is
/// found late or not at all. It is drawn as part of the code rather than left
/// to the caller for exactly that reason — a required margin that has to be
/// remembered is a margin that gets forgotten.
pub const DEFAULT_QUIET_ZONE: u8 = 4;

/// The natural edge of one module, in points.
///
/// A QR code has no resolution of its own: it is a grid of squares that can be
/// drawn at any size, so what settles its natural size is the smallest size it
/// can still be read at. A camera needs several device pixels per module to
/// resolve one, and four points a module is the smallest edge that leaves that
/// margin on a non-HiDPI display. A layout that names a size wins over this,
/// exactly as it does over an image's pixel dimensions.
pub const DEFAULT_MODULE_SIZE: f32 = 4.0;

/// A QR code drawn from its module grid.
///
/// The payload is a signal, so a code bound to state re-encodes and redraws
/// without its subtree being rebuilt. The two colours default to the theme's
/// (see [`Self::module_color`]), and the quiet zone is part of the drawing.
///
/// ```
/// use waterui_qr::{ErrorCorrection, QrCode};
///
/// let ticket = QrCode::new("https://waterui.dev")
///     .correction(ErrorCorrection::High)
///     .module_size(6.0);
/// ```
#[derive(Debug, Clone)]
pub struct QrCode {
    payload: Computed<Str>,
    correction: ErrorCorrection,
    quiet_zone: u8,
    module_size: f32,
    module_color: Option<Color>,
    background_color: Option<Color>,
}

impl QrCode {
    /// A code encoding `payload`.
    ///
    /// The payload takes a signal, so a code bound to state follows it.
    #[must_use]
    pub fn new(payload: impl IntoComputed<Str>) -> Self {
        Self {
            payload: payload.into_computed(),
            correction: ErrorCorrection::default(),
            quiet_zone: DEFAULT_QUIET_ZONE,
            module_size: DEFAULT_MODULE_SIZE,
            module_color: None,
            background_color: None,
        }
    }

    /// Sets how much of the symbol may be lost and still decode.
    ///
    /// A stronger level needs a larger grid for the same payload, so this
    /// changes the code's natural size.
    #[must_use]
    pub const fn correction(mut self, correction: ErrorCorrection) -> Self {
        self.correction = correction;
        self
    }

    /// Sets the clear space around the symbol, in modules.
    ///
    /// [`DEFAULT_QUIET_ZONE`] is what the specification requires and what a
    /// code is drawn with unless it sits on a ground that is already clear for
    /// some other reason.
    #[must_use]
    pub const fn quiet_zone(mut self, modules: u8) -> Self {
        self.quiet_zone = modules;
        self
    }

    /// Sets the edge of one module in points, which is what the code's natural
    /// size is made of.
    ///
    /// # Panics
    ///
    /// Panics if `points` is not finite and positive, which is a caller bug
    /// rather than bad input.
    #[must_use]
    pub fn module_size(mut self, points: f32) -> Self {
        assert!(
            points.is_finite() && points > 0.0,
            "QR module size must be finite and positive, got {points}"
        );
        self.module_size = points;
        self
    }

    /// Overrides the colour the dark modules are drawn in.
    ///
    /// Left alone, the two colours come from the theme — but not verbatim. A QR
    /// code is a machine-readable target, so its colours are functional rather
    /// than decorative: a decoder looks for dark modules on a light ground, and
    /// the detectors in wide use (`quirc`, `ZXing`, and the `rqrr` this crate's
    /// round-trip test decodes with) do not search for the reflectance-reversed
    /// form at all. Drawing `Foreground` on `Surface` verbatim would therefore
    /// hand a dark-mode application a code that looks right and does not scan.
    /// So the default pair is the theme's `Foreground` and `Surface` with the
    /// *darker* of the two always drawing the modules: a themed application
    /// still gets its own near-black and near-white rather than a hardcoded
    /// `#000` on `#fff`, and the polarity a decoder needs survives the colour
    /// scheme.
    ///
    /// Naming either colour hands that judgement back to the caller: the
    /// colours are then used exactly as given, in the order given.
    #[must_use]
    pub fn module_color(mut self, color: impl IntoComputed<Color>) -> Self {
        self.module_color = Some(signal_color(color));
        self
    }

    /// Overrides the colour behind the modules, quiet zone included.
    ///
    /// See [`Self::module_color`] for what the colours default to and why
    /// naming one changes it.
    #[must_use]
    pub fn background_color(mut self, color: impl IntoComputed<Color>) -> Self {
        self.background_color = Some(signal_color(color));
        self
    }

    /// The colours this code draws with, resolved against `env`.
    fn colors(&self, env: &Environment) -> (Computed<ResolvedColor>, Computed<ResolvedColor>) {
        match (&self.module_color, &self.background_color) {
            (Some(modules), Some(background)) => (modules.resolve(env), background.resolve(env)),
            (Some(modules), None) => (modules.resolve(env), SurfaceColor.resolve(env).computed()),
            (None, Some(background)) => (
                ForegroundColor.resolve(env).computed(),
                background.resolve(env),
            ),
            (None, None) => {
                let pair = ForegroundColor
                    .resolve(env)
                    .zip(&SurfaceColor.resolve(env).computed());
                (
                    pair.map(|(foreground, surface)| darker(foreground, surface))
                        .computed(),
                    pair.map(|(foreground, surface)| lighter(foreground, surface))
                        .computed(),
                )
            }
        }
    }
}

/// A QR code encoding `payload`.
///
/// The ergonomic entry point; [`QrCode::new`] is the same constructor and
/// carries the attributes that shape the code.
///
/// ```
/// use waterui_qr::qr_code;
///
/// let code = qr_code("https://waterui.dev");
/// ```
#[must_use]
pub fn qr_code(payload: impl IntoComputed<Str>) -> QrCode {
    QrCode::new(payload)
}

/// The darker of two colours, compared by perceptual lightness.
fn darker(first: ResolvedColor, second: ResolvedColor) -> ResolvedColor {
    if first.to_oklch().lightness <= second.to_oklch().lightness {
        first
    } else {
        second
    }
}

/// The lighter of two colours, compared by perceptual lightness.
fn lighter(first: ResolvedColor, second: ResolvedColor) -> ResolvedColor {
    if first.to_oklch().lightness <= second.to_oklch().lightness {
        second
    } else {
        first
    }
}

/// Scene content that draws one QR code.
///
/// [`QrCode`] wraps this. A backend that already owns a scene can draw a code
/// into it directly rather than going through a view.
pub struct QrContent {
    payload: Computed<Str>,
    correction: ErrorCorrection,
    quiet_zone: u8,
    module_size: f32,
    modules: Computed<ResolvedColor>,
    background: Computed<ResolvedColor>,
    /// The payload last encoded and what it encoded to, `None` when it did not
    /// encode. Measuring and drawing both need the grid, and a container probes
    /// a child several times in one pass, so the answer is kept. A plain
    /// `RefCell` is the right cell for it: layout is single-threaded by
    /// contract and runs on the thread that draws.
    encoded: core::cell::RefCell<Option<(Str, Option<Rc<QrMatrix>>)>>,
    invalidator: Option<SceneInvalidator>,
    /// Keeps the watchers installed by [`SceneContent::set_invalidator`] alive.
    /// Dropping them stops asking for redraws, which is what clearing the
    /// invalidator means.
    guards: alloc::vec::Vec<BoxWatcherGuard>,
}

impl QrContent {
    /// Scene content drawing `payload` at `correction`.
    ///
    /// `module_size` is only the natural size the content reports; the code is
    /// drawn to fill whatever box layout hands it.
    ///
    /// # Panics
    ///
    /// Panics if `module_size` is not finite and positive.
    #[must_use]
    pub fn new(
        payload: impl IntoComputed<Str>,
        correction: ErrorCorrection,
        quiet_zone: u8,
        module_size: f32,
        modules: impl IntoComputed<ResolvedColor>,
        background: impl IntoComputed<ResolvedColor>,
    ) -> Self {
        assert!(
            module_size.is_finite() && module_size > 0.0,
            "QR module size must be finite and positive, got {module_size}"
        );
        Self {
            payload: payload.into_computed(),
            correction,
            quiet_zone,
            module_size,
            modules: modules.into_computed(),
            background: background.into_computed(),
            encoded: core::cell::RefCell::new(None),
            invalidator: None,
            guards: alloc::vec::Vec::new(),
        }
    }

    /// The grid `payload` encodes to, encoded once and kept.
    ///
    /// `None` means the payload does not fit a symbol at this correction level.
    /// There is deliberately nothing to fall back to: a code drawn from a
    /// truncated payload sends whoever scans it somewhere else.
    fn matrix(&self, payload: &Str) -> Option<Rc<QrMatrix>> {
        let mut encoded = self.encoded.borrow_mut();
        if let Some((cached, matrix)) = encoded.as_ref()
            && cached == payload
        {
            return matrix.clone();
        }

        let matrix = match QrMatrix::encode(payload.as_str(), self.correction) {
            Ok(matrix) => Some(Rc::new(matrix)),
            Err(error) => {
                tracing::error!(%error, %payload, "could not encode QR code");
                None
            }
        };
        *encoded = Some((payload.clone(), matrix.clone()));
        matrix
    }

    /// The whole grid, symbol plus quiet zone on both sides, in modules.
    fn side_in_modules(&self, matrix: &QrMatrix) -> usize {
        matrix.width() + 2 * usize::from(self.quiet_zone)
    }
}

impl core::fmt::Debug for QrContent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("QrContent")
            .field("correction", &self.correction)
            .field("quiet_zone", &self.quiet_zone)
            .field("module_size", &self.module_size)
            .finish_non_exhaustive()
    }
}

impl SceneContent for QrContent {
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
            return false;
        }

        // The ground covers the whole box rather than just the symbol's own
        // quiet zone: the code is opaque by definition — a decoder reads
        // reflectance, and whatever shows through modules that are supposed to
        // be light is damage — and any box larger than the snapped grid below
        // leaves its remainder as extra clear space, which is the one thing
        // around a symbol that may grow freely.
        let box_rect = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Brush::Solid(self.background.get().to_peniko()),
            None,
            &box_rect.to_path(0.0),
        );

        let payload = self.payload.get();
        let Some(matrix) = self.matrix(&payload) else {
            return false;
        };
        let side = self.side_in_modules(&matrix);
        let Some(grid) = Grid::fit(width, height, side) else {
            tracing::error!(
                width,
                height,
                side,
                "a {side}-module QR grid does not fit a {width}x{height} box at one unit a module"
            );
            return false;
        };

        let mut path = BezPath::new();
        let quiet = grid.module * f64::from(self.quiet_zone);
        let mut top = grid.y + quiet;
        for row in matrix.rows() {
            let bottom = top + grid.module;
            // Each row is walked as runs of dark modules so a run becomes one
            // rectangle instead of one per module: a version-40 symbol is
            // 31 329 modules, and a fill of that many separate squares is a
            // scene nobody needs to build. Coordinates advance by whole
            // multiples of an integer module edge from an integer origin, so
            // every rectangle in the path is exactly aligned with its
            // neighbours and adjacent runs share an edge instead of leaving a
            // hairline of ground between them.
            let mut x = grid.x + quiet;
            let mut run: Option<f64> = None;
            for &dark in row {
                if dark {
                    run = run.or(Some(x));
                } else if let Some(left) = run.take() {
                    path.extend(Rect::new(left, top, x, bottom).path_elements(0.0));
                }
                x += grid.module;
            }
            if let Some(left) = run.take() {
                path.extend(Rect::new(left, top, x, bottom).path_elements(0.0));
            }
            top = bottom;
        }

        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            &Brush::Solid(self.modules.get().to_peniko()),
            None,
            &path,
        );
        false
    }

    /// The whole grid — symbol plus quiet zone — at this content's module size.
    ///
    /// Square, because a symbol is, so a container that names one axis derives
    /// the other from it instead of stretching the code into something no
    /// decoder will look at. `None` when the payload does not encode, which is
    /// not a size.
    fn intrinsic_size(&self) -> Option<Size> {
        let matrix = self.matrix(&self.payload.get())?;
        // A symbol is at most 177 modules on a side and the quiet zone at most
        // 255 on each, so the total is well inside `u16` and exact in `f32`.
        let side = u16::try_from(self.side_in_modules(&matrix)).ok()?;
        let points = f32::from(side) * self.module_size;
        Some(Size::new(points, points))
    }

    /// What the code encodes.
    ///
    /// A QR code reaches the screen as an anonymous field of squares, so this
    /// node is the only place its content can be announced at all — and unlike
    /// a decorative drawing, a code that cannot be read by anything but a
    /// camera is a dead end for whoever cannot point one at it. It is read
    /// fresh on every emission, and the payload watcher installed by
    /// [`Self::set_invalidator`] is what schedules the frame that re-emits it.
    fn accessibility_label(&self) -> Option<String> {
        Some(self.payload.get().to_string())
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        // The code is drawn from three signals read in `build_scene`, so a
        // surface that is never told one of them changed keeps presenting the
        // code it drew last — a stale payload, or a code that stayed light-on-
        // dark through a switch to the light theme. Watching them schedules the
        // frame that redraws; this is scene invalidation, not a subtree
        // rebuild, so the content instance and its cached grid survive it.
        self.guards = invalidator
            .as_ref()
            .map_or_else(alloc::vec::Vec::new, |invalidator| {
                alloc::vec![
                    invalidate_on_change(invalidator, &self.payload),
                    invalidate_on_change(invalidator, &self.modules),
                    invalidate_on_change(invalidator, &self.background),
                ]
            });
        self.invalidator = invalidator;
    }
}

/// Where the symbol sits inside the box layout handed the code, snapped so
/// every module edge lands on a whole unit of that box's coordinate space.
///
/// Snapping is what keeps a code scannable rather than merely tidy. A module
/// boundary that falls mid-pixel is antialiased into a grey seam, and a
/// binarizer then has to guess which side of the boundary that seam belongs to;
/// guess wrong often enough — and at small sizes it is wrong on most rows — and
/// the symbol carries more damage than its error correction was sized for. With
/// an integer module edge and an integer origin, every boundary is exact and
/// nothing is antialiased at all.
///
/// The space being snapped to is the one `build_scene` is handed: the surface's
/// own pixel grid where the scene owns a `GpuSurface`, and the window's logical
/// grid — which is the device pixel grid at every integer scale factor — where
/// a self-drawn backend merges the scene into its own.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Grid {
    /// Left edge of the drawn block, on a whole unit.
    x: f64,
    /// Top edge of the drawn block, on a whole unit.
    y: f64,
    /// One module's edge: a whole number of units, at least one.
    module: f64,
}

impl Grid {
    /// The largest snapped grid of `side` modules that fits `width` by
    /// `height`, centred in it.
    ///
    /// `None` when the box does not hold one unit per module, which is a code
    /// that could only be drawn by putting module boundaries inside pixels —
    /// exactly the drawing this type exists to avoid.
    fn fit(width: f32, height: f32, side: usize) -> Option<Self> {
        let modules = f64::from(u32::try_from(side).ok()?);
        let module = (f64::from(width.min(height)) / modules).floor();
        if module < 1.0 {
            return None;
        }
        let drawn = module * modules;
        Some(Self {
            x: ((f64::from(width) - drawn) / 2.0).floor(),
            y: ((f64::from(height) - drawn) / 2.0).floor(),
            module,
        })
    }
}

impl View for QrCode {
    /// # Panics
    ///
    /// Panics when the environment carries no theme, since the code's default
    /// colours are the theme's `Foreground` and `Surface`. Every backend
    /// installs them; a bare [`Environment`] does not.
    fn body(self, env: &Environment) -> impl View {
        let (modules, background) = self.colors(env);
        let content = QrContent::new(
            self.payload,
            self.correction,
            self.quiet_zone,
            self.module_size,
            modules,
            background,
        );

        // The code's accessibility node is the leaf's own: `QrContent` answers
        // `SceneContent::accessibility_label` with the payload, which the
        // backend offers as the node's name when the application named nothing
        // itself. It cannot be attached here as `.a11y_label(…)` metadata —
        // that is nearest-consumer and would beat the application's own label
        // instead of yielding to it.
        Frame::new(SceneView::new(content))
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::Cell;

    use nami::Binding;
    use waterui_core::layout::Size;
    use waterui_graphics::color::{ResolvedColor, Srgb};
    use waterui_graphics::{SceneContent as _, SceneInvalidator};
    use waterui_str::Str;

    use super::{DEFAULT_MODULE_SIZE, DEFAULT_QUIET_ZONE, Grid, QrContent, darker, lighter};
    use crate::matrix::{ErrorCorrection, QrMatrix};

    const PAYLOAD: &str = "https://waterui.dev";

    fn content(payload: &Binding<Str>) -> QrContent {
        QrContent::new(
            payload.clone(),
            ErrorCorrection::Medium,
            DEFAULT_QUIET_ZONE,
            DEFAULT_MODULE_SIZE,
            ResolvedColor::from_srgb(Srgb::BLACK),
            ResolvedColor::from_srgb(Srgb::WHITE),
        )
    }

    /// A [`SceneInvalidator`] that counts the frames it was asked for.
    fn counting_invalidator() -> (SceneInvalidator, Rc<Cell<usize>>) {
        let redraws = Rc::new(Cell::new(0_usize));
        let counted = Rc::clone(&redraws);
        let invalidator: SceneInvalidator = Rc::new(move || counted.set(counted.get() + 1));
        (invalidator, redraws)
    }

    #[test]
    fn the_natural_size_is_the_grid_and_its_quiet_zone() {
        let content = content(&Binding::container(Str::from_static(PAYLOAD)));
        let width = QrMatrix::encode(PAYLOAD, ErrorCorrection::Medium)
            .expect("the payload fits")
            .width();

        let modules = u16::try_from(width + 2 * usize::from(DEFAULT_QUIET_ZONE))
            .expect("a symbol and its quiet zone are far inside u16");
        let side = f32::from(modules) * DEFAULT_MODULE_SIZE;
        assert_eq!(content.intrinsic_size(), Some(Size::new(side, side)));
    }

    /// A payload no symbol holds has no grid, and therefore no size — rather
    /// than a size for a code that cannot be drawn.
    #[test]
    fn a_payload_that_does_not_encode_has_no_natural_size() {
        let payload = Str::from(alloc::string::String::from_iter(core::iter::repeat_n(
            'W', 4096,
        )));
        let content = content(&Binding::container(payload));
        assert_eq!(content.intrinsic_size(), None);
    }

    #[test]
    fn the_accessibility_node_carries_the_payload() {
        let payload = Binding::container(Str::from_static(PAYLOAD));
        let content = content(&payload);
        assert_eq!(content.accessibility_label().as_deref(), Some(PAYLOAD));

        payload.set(Str::from_static("https://example.invalid"));
        assert_eq!(
            content.accessibility_label().as_deref(),
            Some("https://example.invalid"),
            "the node must follow the payload signal, not freeze at what it was built with"
        );
    }

    #[test]
    fn changing_the_payload_asks_for_a_frame() {
        let payload = Binding::container(Str::from_static(PAYLOAD));
        let mut content = content(&payload);
        let (invalidator, redraws) = counting_invalidator();

        content.set_invalidator(Some(invalidator));
        let installed = redraws.get();

        payload.set(Str::from_static("https://example.invalid"));

        assert!(
            redraws.get() > installed,
            "a code bound to state must schedule a frame when its payload changes"
        );
    }

    /// A theme switch changes only the colours, and a surface that is not told
    /// about it keeps presenting a code in the old scheme's polarity.
    #[test]
    fn changing_a_colour_asks_for_a_frame() {
        let modules = Binding::container(ResolvedColor::from_srgb(Srgb::BLACK));
        let mut content = QrContent::new(
            Str::from_static(PAYLOAD),
            ErrorCorrection::Medium,
            DEFAULT_QUIET_ZONE,
            DEFAULT_MODULE_SIZE,
            modules.clone(),
            ResolvedColor::from_srgb(Srgb::WHITE),
        );
        let (invalidator, redraws) = counting_invalidator();

        content.set_invalidator(Some(invalidator));
        let installed = redraws.get();

        modules.set(ResolvedColor::from_srgb(Srgb::new(0.1, 0.1, 0.1)));

        assert!(redraws.get() > installed);
    }

    #[test]
    fn clearing_the_invalidator_stops_the_frames() {
        let payload = Binding::container(Str::from_static(PAYLOAD));
        let mut content = content(&payload);
        let (invalidator, redraws) = counting_invalidator();

        content.set_invalidator(Some(invalidator));
        content.set_invalidator(None);
        let cleared = redraws.get();

        payload.set(Str::from_static("https://example.invalid"));

        assert_eq!(
            redraws.get(),
            cleared,
            "a surface that took its invalidator back must stop being asked for frames"
        );
    }

    /// The grid's whole promise is that its coordinates are exact whole
    /// numbers, so exact comparison is the assertion; an epsilon would pass on
    /// precisely the drawing this type exists to rule out.
    #[expect(
        clippy::float_cmp,
        reason = "a snapped grid's coordinates are exact integers by construction"
    )]
    #[test]
    fn every_module_edge_lands_on_a_whole_unit() {
        // 29 modules (a version-1 symbol with its quiet zone) in a box that is
        // not a multiple of it: 200 / 29 is 6.89, so the grid must take 6.
        let grid = Grid::fit(200.0, 200.0, 29).expect("29 modules fit 200 units");
        assert_eq!(grid.module, 6.0);
        assert_eq!(grid.x, 13.0, "the 26 leftover units are split evenly");
        assert_eq!(grid.y, 13.0);

        let edges: Vec<f64> = (0..=29)
            .map(|index| grid.module.mul_add(f64::from(index), grid.x))
            .collect();
        assert!(
            edges.iter().all(|edge| edge.fract() == 0.0),
            "a module edge inside a pixel is a grey seam a binarizer has to guess at"
        );
        assert_eq!(
            *edges.last().expect("the grid has edges"),
            grid.x + 174.0,
            "the drawn block is the module edge times the module count, exactly"
        );
    }

    /// The box is the code's, not the symbol's: a non-square box centres the
    /// grid on both axes and leaves the rest as clear space.
    #[expect(
        clippy::float_cmp,
        reason = "a snapped grid's coordinates are exact integers by construction"
    )]
    #[test]
    fn a_non_square_box_is_sized_by_its_shorter_side() {
        let grid = Grid::fit(300.0, 200.0, 29).expect("29 modules fit the shorter side");
        assert_eq!(grid.module, 6.0);
        assert_eq!(grid.x, 63.0);
        assert_eq!(grid.y, 13.0);
    }

    /// Below one unit a module there is no drawing that is not a guess, so
    /// there is no grid.
    #[test]
    fn a_box_too_small_for_one_unit_a_module_has_no_grid() {
        assert_eq!(Grid::fit(28.0, 28.0, 29), None);
        assert!(Grid::fit(29.0, 29.0, 29).is_some());
    }

    #[test]
    fn the_darker_colour_is_picked_by_perceptual_lightness() {
        let black = ResolvedColor::from_srgb(Srgb::BLACK);
        let white = ResolvedColor::from_srgb(Srgb::WHITE);

        // Compared as the paint each resolves to, which is what the drawing
        // actually uses and what makes the two colours distinguishable without
        // comparing floats.
        assert_eq!(darker(black, white).to_peniko(), black.to_peniko());
        assert_eq!(darker(white, black).to_peniko(), black.to_peniko());
        assert_eq!(lighter(black, white).to_peniko(), white.to_peniko());
        assert_eq!(lighter(white, black).to_peniko(), white.to_peniko());
    }
}
