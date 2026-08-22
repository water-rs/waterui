#![cfg_attr(
    test,
    allow(
        clippy::float_cmp,
        reason = "tests assert exact canvas geometry values"
    )
)]
//! Canvas view for 2D vector graphics rendering.
//!
//! `Canvas` provides an easy-to-use API for drawing 2D graphics using Vello.
//! It renders at full GPU speed while exposing a simple, declarative interface.
//!
//! # Example
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui_canvas::{Canvas, DrawingContext};
//! use waterui_graphics::color::Srgb;
//!
//! # fn scene() -> Canvas {
//! Canvas::new(|ctx: &mut DrawingContext| {
//!     // Fill a rectangle
//!     let rect = Rect::from_size(Size::new(200.0, 150.0));
//!     ctx.set_fill_style(Srgb::new(1.0, 0.0, 0.0));
//!     ctx.fill_rect(rect);
//!
//!     // Draw with transforms
//!     ctx.save();
//!     ctx.translate(100.0, 100.0);
//!     ctx.rotate(0.785); // 45 degrees
//!     ctx.fill_rect(Rect::from_size(Size::new(50.0, 50.0)));
//!     ctx.restore();
//! })
//! # }
//! ```

extern crate alloc;

pub mod state;

/// Path construction API for Canvas.
pub mod path;

/// Conversion utilities between WaterUI and Vello types.
mod conversions;

/// Gradient builders for Canvas.
pub mod gradient;

/// Image loading and handling for Canvas.
pub mod image;

/// Text rendering support for Canvas.
pub mod text;

use core::fmt;
pub use path::Path;

pub use state::{LineCap, LineJoin};

pub use state::FillRule;

pub use gradient::{ConicGradient, LinearGradient, RadialGradient};

pub use image::{CanvasImage, ImageError};

pub use text::{FontSpec, FontStyle, FontWeight, TextMetrics};

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::Any;
use core::cell::Cell;

use nami::Signal;
use nami::signal::IntoSignal;
use waterui_core::IntoSignalF32;
use waterui_core::layout::{Affine2, Point, Rect, Size, StretchAxis};

fn affine2_to_kurbo(t: Affine2) -> kurbo::Affine {
    kurbo::Affine::new([
        f64::from(t.a),
        f64::from(t.b),
        f64::from(t.c),
        f64::from(t.d),
        f64::from(t.e),
        f64::from(t.f),
    ])
}

// Internal imports for rendering (not exposed to users)
use kurbo::Shape as _;

use crate::conversions::{point_to_kurbo, rect_to_kurbo, resolved_color_to_peniko};
use crate::state::{DrawingState, FillStyle, StrokeStyle};
use waterui_graphics::{Glyph, GlyphRun, Scene2D, SceneContent, SceneInvalidator, SceneView};

/// A canvas for 2D vector graphics rendering.
///
/// Canvas provides a simple callback-based API where you receive a
/// [`DrawingContext`] to draw shapes, paths, and text.
pub struct Canvas {
    draw_fn: Box<dyn FnMut(&mut DrawingContext)>,
}

impl fmt::Debug for Canvas {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Canvas").finish_non_exhaustive()
    }
}

impl Canvas {
    /// Creates a new canvas with a drawing callback.
    ///
    /// The callback is invoked each frame with a [`DrawingContext`] that
    /// provides methods for drawing shapes, paths, and more.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_canvas::Canvas;
    /// # fn dot() -> Canvas {
    /// Canvas::new(|ctx| {
    ///     ctx.set_fill_style(waterui_graphics::color::Srgb::new_u8(242, 140, 168));
    ///     ctx.fill_circle(Point::new(50.0, 50.0), 25.0);
    /// })
    /// # }
    /// ```
    #[must_use]
    pub fn new<F>(draw: F) -> Self
    where
        F: FnMut(&mut DrawingContext) + 'static,
    {
        Self {
            draw_fn: Box::new(draw),
        }
    }

    /// Creates a canvas whose drawing callback receives the current value of a signal.
    ///
    /// The canvas tracks the signal precisely and redraws when that signal changes,
    /// without rebuilding the `Canvas` view itself.
    #[must_use]
    pub fn with_signal<S, D, F>(signal: S, mut draw: F) -> Self
    where
        S: Signal<Output = D> + 'static,
        S::Guard: 'static,
        D: 'static,
        F: FnMut(&mut DrawingContext, D) + 'static,
    {
        Self::new(move |ctx| {
            ctx.track_signal(&signal);
            draw(ctx, signal.get());
        })
    }
}

impl waterui_core::View for Canvas {
    fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
        SceneView::new(CanvasContent {
            draw_fn: self.draw_fn,
            invalidator: None,
            pending_redraw: Rc::new(Cell::new(false)),
            active_guards: Vec::new(),
            text_engine: TextEngine::default(),
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[derive(Default)]
struct TextEngine {
    font_cx: Option<parley::FontContext>,
    layout_cx: Option<parley::LayoutContext>,
}

impl TextEngine {
    fn font_cx(&mut self) -> &mut parley::FontContext {
        self.font_cx.get_or_insert_with(parley::FontContext::new)
    }
}

struct ReactiveFrameState<'a> {
    pending_redraw: Rc<Cell<bool>>,
    invalidator: Option<SceneInvalidator>,
    guards: &'a mut Vec<Box<dyn Any>>,
}

/// Drawable resource for unified Canvas resource rendering.
#[derive(Debug, Clone, Copy)]
pub enum CanvasResource<'a> {
    /// Raster image resource.
    Image(&'a CanvasImage),
    /// Plain text resource.
    Text(&'a str),
}

impl<'a> From<&'a CanvasImage> for CanvasResource<'a> {
    fn from(value: &'a CanvasImage) -> Self {
        Self::Image(value)
    }
}

impl<'a> From<&'a str> for CanvasResource<'a> {
    fn from(value: &'a str) -> Self {
        Self::Text(value)
    }
}

impl<'a> From<&'a String> for CanvasResource<'a> {
    fn from(value: &'a String) -> Self {
        Self::Text(value.as_str())
    }
}

/// Context for drawing 2D graphics.
///
/// This is passed to your drawing callback each frame. Use it to draw
/// shapes, paths, text, and images.
///
/// The context maintains a state stack for transforms, styles, and other
/// drawing properties. Use `save()` and `restore()` to push and pop state.
pub struct DrawingContext<'a> {
    scene: &'a mut dyn Scene2D,
    /// Width of the canvas in pixels.
    pub width: f32,
    /// Height of the canvas in pixels.
    pub height: f32,
    /// State stack for save/restore operations.
    state_stack: Vec<DrawingState>,
    /// Current drawing state.
    current_state: DrawingState,
    reactive: ReactiveFrameState<'a>,
    text_engine: &'a mut TextEngine,
    requested_next_frame: bool,
}

impl fmt::Debug for DrawingContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DrawingContext")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl DrawingContext<'_> {
    /// Returns the size of the canvas.
    #[must_use]
    pub const fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    /// Returns the center point of the canvas.
    #[must_use]
    pub fn center(&self) -> Point {
        Point::new(self.width / 2.0, self.height / 2.0)
    }

    /// Requests another frame after the current one completes.
    pub const fn request_next_frame(&mut self) {
        self.requested_next_frame = true;
    }

    fn track_signal<S>(&mut self, signal: &S)
    where
        S: Signal + 'static,
        S::Guard: 'static,
    {
        let pending_redraw = Rc::clone(&self.reactive.pending_redraw);
        let invalidator = self.reactive.invalidator.clone();
        let guard = signal.watch(move |_| {
            pending_redraw.set(true);
            if let Some(invalidator) = &invalidator {
                invalidator();
            }
        });
        self.reactive.guards.push(Box::new(guard));
    }

    fn resolve_signal<T, S>(&mut self, value: S) -> T
    where
        T: 'static,
        S: IntoSignal<T>,
        S::Signal: Signal<Output = T> + 'static,
        <S::Signal as Signal>::Guard: 'static,
    {
        let signal = value.into_signal();
        self.track_signal(&signal);
        signal.get()
    }

    fn resolve_f32(&mut self, value: impl IntoSignalF32) -> f32 {
        let signal = value.into_signal_f32();
        self.track_signal(&signal);
        let resolved = signal.get();
        assert!(
            resolved.is_finite(),
            "Canvas f32 signal resolved to a non-finite value"
        );
        resolved
    }

    /// Pushes a clip layer, clipping subsequent drawing to the given rectangle.
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_clip_rect(&mut self, rect: impl IntoSignal<Rect>) {
        let rect = self.resolve_signal(rect);
        let kurbo_rect = rect_to_kurbo(rect);
        let clip_path = kurbo_rect.to_path(0.1);
        self.scene.push_clip_layer(
            self.current_state.fill_rule,
            self.current_state.transform,
            &clip_path,
        );
    }

    /// Pushes a clip layer, clipping subsequent drawing to the given path.
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_clip_path(&mut self, path: &Path) {
        self.scene.push_clip_layer(
            self.current_state.fill_rule,
            self.current_state.transform,
            path.inner(),
        );
    }

    /// Pushes a layer with alpha (opacity), clipping content to the given rectangle.
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_alpha_rect(&mut self, alpha: impl IntoSignalF32, rect: impl IntoSignal<Rect>) {
        let alpha = self.resolve_f32(alpha);
        let rect = self.resolve_signal(rect);
        let kurbo_rect = rect_to_kurbo(rect);
        let clip_path = kurbo_rect.to_path(0.1);
        self.scene.push_layer(
            self.current_state.fill_rule,
            self.current_state.blend_mode,
            alpha.clamp(0.0, 1.0),
            self.current_state.transform,
            &clip_path,
        );
    }

    /// Pushes a layer with alpha (opacity), clipping content to the given path.
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_alpha_path(&mut self, alpha: impl IntoSignalF32, path: &Path) {
        let alpha = self.resolve_f32(alpha);
        self.scene.push_layer(
            self.current_state.fill_rule,
            self.current_state.blend_mode,
            alpha.clamp(0.0, 1.0),
            self.current_state.transform,
            path.inner(),
        );
    }

    /// Pops the current layer.
    pub fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    // ========================================================================
    // Unified Resource Drawing
    // ========================================================================

    /// Draws an image/text resource at a point.
    pub fn draw_resource<'a>(
        &mut self,
        resource: impl Into<CanvasResource<'a>>,
        pos: impl IntoSignal<Point>,
    ) {
        match resource.into() {
            CanvasResource::Image(image) => self.draw_image(image, pos),
            CanvasResource::Text(text) => self.draw_text(text, pos),
        }
    }

    /// Draws an image/text resource within a rectangle.
    ///
    /// Image is scaled to the rectangle.
    /// Text is wrapped to rectangle width and clipped to the rectangle bounds.
    pub fn draw_resource_in<'a>(
        &mut self,
        resource: impl Into<CanvasResource<'a>>,
        rect: impl IntoSignal<Rect>,
    ) {
        match resource.into() {
            CanvasResource::Image(image) => self.draw_image_scaled(image, rect),
            CanvasResource::Text(text) => self.draw_text_in_rect(text, rect),
        }
    }

    // ========================================================================
    // State Management (Phase 1)
    // ========================================================================

    /// Saves the current drawing state to the stack.
    ///
    /// This saves transforms, styles, line properties, and other state.
    /// Call `restore()` to pop the saved state.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui_canvas::DrawingContext;
    /// # fn draw(ctx: &mut DrawingContext<'_>) {
    /// ctx.save();
    /// ctx.translate(100.0, 50.0);
    /// ctx.rotate(0.785);
    /// // ... draw with transform ...
    /// ctx.restore(); // Back to original state
    /// # }
    /// ```
    pub fn save(&mut self) {
        self.state_stack.push(self.current_state.clone());
    }

    /// Restores the most recently saved drawing state from the stack.
    ///
    /// If there's no saved state, this does nothing.
    pub fn restore(&mut self) {
        if let Some(state) = self.state_stack.pop() {
            self.current_state = state;
        }
    }

    // ========================================================================
    // Transform Helpers (Phase 1)
    // ========================================================================

    /// Translates the current transform by (x, y).
    ///
    /// This affects all subsequent drawing operations until `restore()`.
    pub fn translate(&mut self, x: impl IntoSignalF32, y: impl IntoSignalF32) {
        let x = self.resolve_f32(x);
        let y = self.resolve_f32(y);
        let translation = kurbo::Affine::translate((f64::from(x), f64::from(y)));
        self.current_state.transform *= translation;
    }

    /// Rotates the current transform by the given angle (in radians).
    ///
    /// Positive angles rotate clockwise.
    pub fn rotate(&mut self, angle: impl IntoSignalF32) {
        let angle = self.resolve_f32(angle);
        let rotation = kurbo::Affine::rotate(f64::from(angle));
        self.current_state.transform *= rotation;
    }

    /// Scales the current transform by (x, y).
    ///
    /// Values less than 1.0 shrink, greater than 1.0 enlarge.
    pub fn scale(&mut self, x: impl IntoSignalF32, y: impl IntoSignalF32) {
        let x = self.resolve_f32(x);
        let y = self.resolve_f32(y);
        let scale = kurbo::Affine::scale_non_uniform(f64::from(x), f64::from(y));
        self.current_state.transform *= scale;
    }

    /// Applies an arbitrary affine transform.
    ///
    /// Accepts any value convertible into [`Affine2`], including the type
    /// itself, `[f32; 6]`, or one of the helper constructors
    /// `Affine2::translate(...)`, `::scale(...)`, `::rotate(...)`. The
    /// supplied transform is multiplied onto the current transform.
    pub fn transform(&mut self, transform: impl Into<Affine2>) {
        let affine = affine2_to_kurbo(transform.into());
        self.current_state.transform *= affine;
    }

    /// Replaces the current transform with the specified matrix.
    ///
    /// Accepts any value convertible into [`Affine2`].
    pub fn set_transform(&mut self, transform: impl Into<Affine2>) {
        self.current_state.transform = affine2_to_kurbo(transform.into());
    }

    /// Resets the transform to the identity matrix.
    pub const fn reset_transform(&mut self) {
        self.current_state.transform = kurbo::Affine::IDENTITY;
    }

    // ========================================================================
    // Path Drawing (Phase 1)
    // ========================================================================

    /// Creates a new empty path.
    ///
    /// Use the returned `Path` to build complex shapes, then draw it with
    /// `fill_path()` or `stroke_path()`.
    #[must_use]
    pub fn begin_path(&self) -> Path {
        Path::new()
    }

    /// Fills a path with the current fill style.
    pub fn fill_path(&mut self, path: &Path) {
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let brush = self.resolve_fill_style();
        let pushed_alpha = self.push_global_alpha_layer_if_needed(path.inner());
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            &brush,
            None,
            path.inner(),
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Strokes a path with the current stroke style and line properties.
    pub fn stroke_path(&mut self, path: &Path) {
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        let pushed_alpha = self.push_global_alpha_layer_if_needed(path.inner());
        self.scene.stroke(
            &stroke,
            self.current_state.transform,
            &brush,
            None,
            path.inner(),
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    // ========================================================================
    // Rectangle Convenience Methods (Phase 3)
    // ========================================================================

    /// Fills a rectangle with the current fill style.
    pub fn fill_rect(&mut self, rect: impl IntoSignal<Rect>) {
        let rect = self.resolve_signal(rect);
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let kurbo_rect = rect_to_kurbo(rect);
        let shape_path = kurbo_rect.to_path(0.1);
        let brush = self.resolve_fill_style();
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            &brush,
            None,
            &shape_path,
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Strokes a rectangle with the current stroke style.
    pub fn stroke_rect(&mut self, rect: impl IntoSignal<Rect>) {
        let rect = self.resolve_signal(rect);
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let kurbo_rect = rect_to_kurbo(rect);
        let shape_path = kurbo_rect.to_path(0.1);
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene.stroke(
            &stroke,
            self.current_state.transform,
            &brush,
            None,
            &shape_path,
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Clears a rectangle to transparent black.
    pub fn clear_rect(&mut self, rect: impl IntoSignal<Rect>) {
        let rect = self.resolve_signal(rect);
        let kurbo_rect = rect_to_kurbo(rect);
        let shape_path = kurbo_rect.to_path(0.1);
        let brush: peniko::Brush = peniko::Color::TRANSPARENT.into();
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            &brush,
            None,
            &shape_path,
        );
    }

    // ========================================================================
    // Shape Convenience Methods
    // ========================================================================

    /// Fills a circle with the current fill style.
    pub fn fill_circle(&mut self, center: impl IntoSignal<Point>, radius: impl IntoSignalF32) {
        let center = self.resolve_signal(center);
        let radius = self.resolve_f32(radius);
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let brush = self.resolve_fill_style();
        let circle = kurbo::Circle::new(point_to_kurbo(center), f64::from(radius));
        let shape_path = circle.to_path(0.1);
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            &brush,
            None,
            &shape_path,
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Strokes a circle with the current stroke style.
    pub fn stroke_circle(&mut self, center: impl IntoSignal<Point>, radius: impl IntoSignalF32) {
        let center = self.resolve_signal(center);
        let radius = self.resolve_f32(radius);
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        let circle = kurbo::Circle::new(point_to_kurbo(center), f64::from(radius));
        let shape_path = circle.to_path(0.1);
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene.stroke(
            &stroke,
            self.current_state.transform,
            &brush,
            None,
            &shape_path,
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Strokes a line segment with the current stroke style.
    pub fn stroke_line(&mut self, start: impl IntoSignal<Point>, end: impl IntoSignal<Point>) {
        let start = self.resolve_signal(start);
        let end = self.resolve_signal(end);
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        let line = kurbo::Line::new(point_to_kurbo(start), point_to_kurbo(end));
        let shape_path = line.to_path(0.1);
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene.stroke(
            &stroke,
            self.current_state.transform,
            &brush,
            None,
            &shape_path,
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    // ========================================================================
    // Style Setters (Phase 1 & 4)
    // ========================================================================

    /// Sets the fill style (color or gradient).
    pub fn set_fill_style(&mut self, style: impl Into<FillStyle>) {
        self.current_state.fill_style = style.into();
    }

    /// Sets the stroke style (color or gradient).
    pub fn set_stroke_style(&mut self, style: impl Into<StrokeStyle>) {
        self.current_state.stroke_style = style.into();
    }

    /// Sets the line width for stroking operations.
    pub fn set_line_width(&mut self, width: impl IntoSignalF32) {
        let width = self.resolve_f32(width);
        self.current_state.line_width = width;
    }

    /// Sets the line cap style (how stroke endpoints are drawn).
    pub const fn set_line_cap(&mut self, cap: LineCap) {
        self.current_state.line_cap = cap;
    }

    /// Sets the line join style (how stroke corners are drawn).
    pub const fn set_line_join(&mut self, join: LineJoin) {
        self.current_state.line_join = join;
    }

    /// Sets the miter limit for miter line joins.
    pub fn set_miter_limit(&mut self, limit: impl IntoSignalF32) {
        let limit = self.resolve_f32(limit);
        self.current_state.miter_limit = limit;
    }

    /// Sets the line dash pattern.
    ///
    /// Pass an empty vector to disable dashing.
    pub fn set_line_dash(&mut self, segments: impl IntoSignal<Vec<f32>>) {
        let segments = self.resolve_signal(segments);
        self.current_state.line_dash = segments;
    }

    /// Sets the line dash offset (where the dash pattern starts).
    pub fn set_line_dash_offset(&mut self, offset: impl IntoSignalF32) {
        let offset = self.resolve_f32(offset);
        self.current_state.line_dash_offset = offset;
    }

    /// Sets the global alpha (opacity) for all drawing operations.
    ///
    /// Values range from 0.0 (transparent) to 1.0 (opaque).
    pub fn set_global_alpha(&mut self, alpha: impl IntoSignalF32) {
        let alpha = self.resolve_f32(alpha);
        self.current_state.global_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Sets the fill rule for determining the interior of shapes.
    ///
    /// # Arguments
    /// * `rule` - The fill rule to use (`NonZero` or `EvenOdd`)
    ///
    /// `NonZero` (default): A point is inside the path if a ray from the point crosses a non-zero net number of path segments.
    /// `EvenOdd`: A point is inside the path if a ray from the point crosses an odd number of path segments.
    pub const fn set_fill_rule(&mut self, rule: FillRule) {
        self.current_state.fill_rule = rule.to_peniko();
    }

    // ========================================================================
    // Gradient Creation Methods (Phase 2)
    // ========================================================================

    /// Creates a linear gradient from (x0, y0) to (x1, y1).
    ///
    /// Returns a `LinearGradient` builder. Add color stops with `add_color_stop()`,
    /// then use with `set_fill_style()` or `set_stroke_style()`.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui_canvas::DrawingContext;
    /// # fn draw(ctx: &mut DrawingContext<'_>) {
    /// let mut gradient = ctx.create_linear_gradient(0.0, 0.0, 100.0, 100.0);
    /// gradient.add_color_stop(0.0, waterui_graphics::color::Srgb::new(1.0, 0.0, 0.0));
    /// gradient.add_color_stop(1.0, waterui_graphics::color::Srgb::new(0.0, 0.0, 1.0));
    /// ctx.set_fill_style(gradient);
    /// # }
    /// ```
    #[must_use]
    pub const fn create_linear_gradient(
        &self,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    ) -> LinearGradient {
        LinearGradient::new(x0, y0, x1, y1)
    }

    /// Creates a radial gradient between two circles.
    ///
    /// # Arguments
    /// * `x0, y0` - Center of the start circle
    /// * `r0` - Radius of the start circle
    /// * `x1, y1` - Center of the end circle
    /// * `r1` - Radius of the end circle
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui_canvas::DrawingContext;
    /// # fn draw(ctx: &mut DrawingContext<'_>) {
    /// let mut gradient = ctx.create_radial_gradient(50.0, 50.0, 10.0, 50.0, 50.0, 50.0);
    /// gradient.add_color_stop(0.0, waterui_graphics::color::Srgb::new(1.0, 1.0, 1.0));
    /// gradient.add_color_stop(1.0, waterui_graphics::color::Srgb::new(0.0, 0.0, 0.0));
    /// ctx.set_fill_style(gradient);
    /// # }
    /// ```
    #[must_use]
    pub const fn create_radial_gradient(
        &self,
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
    ) -> RadialGradient {
        RadialGradient::new(x0, y0, r0, x1, y1, r1)
    }

    /// Creates a conic (sweep) gradient around a center point.
    ///
    /// # Arguments
    /// * `start_angle` - Starting angle in radians (0 = 3 o'clock)
    /// * `x, y` - Center point of the gradient
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui_canvas::DrawingContext;
    /// # fn draw(ctx: &mut DrawingContext<'_>) {
    /// let mut gradient = ctx.create_conic_gradient(0.0, 50.0, 50.0);
    /// gradient.add_color_stop(0.0, waterui_graphics::color::Srgb::new(1.0, 0.0, 0.0));
    /// gradient.add_color_stop(0.5, waterui_graphics::color::Srgb::new(0.0, 1.0, 0.0));
    /// gradient.add_color_stop(1.0, waterui_graphics::color::Srgb::new(0.0, 0.0, 1.0));
    /// ctx.set_fill_style(gradient);
    /// # }
    /// ```
    #[must_use]
    pub const fn create_conic_gradient(&self, start_angle: f32, x: f32, y: f32) -> ConicGradient {
        ConicGradient::new(start_angle, x, y)
    }

    // ========================================================================
    // Image Drawing Methods (Phase 6)
    // ========================================================================

    /// Draws an image at the specified position.
    ///
    /// The image is drawn at its natural size (1:1 pixel mapping).
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_canvas::{CanvasImage, DrawingContext, ImageError};
    /// # fn draw(ctx: &mut DrawingContext<'_>, png_data: &[u8]) -> Result<(), ImageError> {
    /// let image = CanvasImage::from_bytes(png_data)?;
    /// ctx.draw_image(&image, Point::new(10.0, 10.0));
    /// # Ok(())
    /// # }
    /// ```
    pub fn draw_image(&mut self, image: &CanvasImage, pos: impl IntoSignal<Point>) {
        let pos = self.resolve_signal(pos);
        let size = image.size();
        let dest_rect = Rect::new(pos, size);
        self.draw_image_scaled(image, dest_rect);
    }

    /// Draws an image scaled to fit the destination rectangle.
    ///
    /// # Arguments
    /// * `image` - The image to draw
    /// * `dest` - Destination rectangle (position and size)
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_canvas::{CanvasImage, DrawingContext, ImageError};
    /// # fn draw(ctx: &mut DrawingContext<'_>, png_data: &[u8]) -> Result<(), ImageError> {
    /// let image = CanvasImage::from_bytes(png_data)?;
    /// let dest = Rect::new(Point::zero(), Size::new(200.0, 150.0));
    /// ctx.draw_image_scaled(&image, dest);
    /// # Ok(())
    /// # }
    /// ```
    pub fn draw_image_scaled(&mut self, image: &CanvasImage, dest: impl IntoSignal<Rect>) {
        let dest = self.resolve_signal(dest);
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        // Calculate transform to scale image to destination rectangle
        let scale_x = f64::from(dest.size().width) / f64::from(image.width());
        let scale_y = f64::from(dest.size().height) / f64::from(image.height());

        // Create transform: translate to dest position, then scale
        let image_transform =
            kurbo::Affine::translate((f64::from(dest.origin().x), f64::from(dest.origin().y)))
                * kurbo::Affine::scale_non_uniform(scale_x, scale_y);

        // Compose with current transform
        let final_transform = self.current_state.transform * image_transform;

        // Wrap ImageData in ImageBrush
        let image_brush = peniko::ImageBrush::new(image.inner().clone());
        let dest_rect = rect_to_kurbo(dest);
        let dest_path = dest_rect.to_path(0.1);
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&dest_path);

        // Draw image using vello
        self.scene.draw_image(&image_brush, final_transform);
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Draws a sub-rectangle of an image, scaled to fit the destination.
    ///
    /// This allows drawing only part of an image (sprite sheet support).
    ///
    /// # Arguments
    /// * `image` - The source image
    /// * `src` - Source rectangle (which part of the image to draw)
    /// * `dest` - Destination rectangle (where and how large to draw)
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_canvas::{CanvasImage, DrawingContext, ImageError};
    /// # fn draw(ctx: &mut DrawingContext<'_>, png_data: &[u8]) -> Result<(), ImageError> {
    /// let sprite_sheet = CanvasImage::from_bytes(png_data)?;
    /// // Draw top-left 32x32 sprite at position (100, 100) scaled to 64x64
    /// let src = Rect::new(Point::zero(), Size::new(32.0, 32.0));
    /// let dest = Rect::new(Point::new(100.0, 100.0), Size::new(64.0, 64.0));
    /// ctx.draw_image_sub(&sprite_sheet, src, dest);
    /// # Ok(())
    /// # }
    /// ```
    pub fn draw_image_sub(
        &mut self,
        image: &CanvasImage,
        src: impl IntoSignal<Rect>,
        dest: impl IntoSignal<Rect>,
    ) {
        let src = self.resolve_signal(src);
        let dest = self.resolve_signal(dest);
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        // Use push_clip_layer with clip to render only the source rectangle
        // Calculate transform for the sub-rectangle

        // First, translate to negate the source offset
        let src_offset =
            kurbo::Affine::translate((-f64::from(src.origin().x), -f64::from(src.origin().y)));

        // Then scale from source size to destination size
        let scale_x = f64::from(dest.size().width) / f64::from(src.size().width);
        let scale_y = f64::from(dest.size().height) / f64::from(src.size().height);
        let scale = kurbo::Affine::scale_non_uniform(scale_x, scale_y);

        // Finally, translate to destination position
        let dest_offset =
            kurbo::Affine::translate((f64::from(dest.origin().x), f64::from(dest.origin().y)));

        // Compose transforms so a source point is first shifted by -src origin,
        // then scaled, then translated to the destination. kurbo's `Affine`
        // multiplication applies the RIGHT operand first, so the source offset
        // must sit rightmost.
        let image_transform = dest_offset * scale * src_offset;

        // Compose with current transform
        let final_transform = self.current_state.transform * image_transform;

        // Create clip rectangle at destination
        let clip_rect = rect_to_kurbo(dest);
        let clip_path = clip_rect.to_path(0.1);

        // Push a clipped layer, draw the image, then pop
        self.scene.push_clip_layer(
            self.current_state.fill_rule,
            self.current_state.transform,
            &clip_path,
        );
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&clip_path);

        // Wrap ImageData in ImageBrush
        let image_brush = peniko::ImageBrush::new(image.inner().clone());

        self.scene.draw_image(&image_brush, final_transform);

        self.pop_global_alpha_layer_if_needed(pushed_alpha);
        self.scene.pop_layer();
    }

    // ========================================================================
    // Text Rendering Methods (Phase 5)
    // ========================================================================

    /// Sets the font for text rendering.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui_canvas::{DrawingContext, FontSpec, FontWeight};
    /// # fn draw(ctx: &mut DrawingContext<'_>) {
    /// ctx.set_font(FontSpec::new("Arial", 24.0).with_weight(FontWeight::Bold));
    /// # }
    /// ```
    pub fn set_font(&mut self, font: FontSpec) {
        self.current_state.font = font;
    }

    /// Measures the given text with the current font.
    ///
    /// Uses the same shaping pipeline as [`draw_text`](Self::draw_text), so the
    /// metrics match what drawing produces exactly.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use waterui::prelude::*;
    /// # use waterui_canvas::DrawingContext;
    /// # fn draw(ctx: &mut DrawingContext<'_>) {
    /// let metrics = ctx.measure_text("Hello World");
    /// // Centre the string horizontally in a 320pt-wide canvas.
    /// ctx.draw_text("Hello World", Point::new((320.0 - metrics.width) / 2.0, 40.0));
    /// # }
    /// ```
    #[must_use]
    pub fn measure_text(&mut self, text: &str) -> TextMetrics {
        if text.is_empty() {
            return TextMetrics::new(0.0, 0.0);
        }

        let layout = self.build_text_layout(text, None);
        TextMetrics::new(layout.full_width(), layout.height())
    }

    /// Draws filled text at the specified position.
    pub fn draw_text(&mut self, text: &str, pos: impl IntoSignal<Point>) {
        if text.is_empty() || self.skip_draw_for_zero_alpha() {
            return;
        }

        let pos = self.resolve_signal(pos);
        let layout = self.build_text_layout(text, None);
        self.draw_text_layout(&layout, pos);
    }

    /// Draws text inside a rectangle.
    ///
    /// The text layout is width-constrained and clipped to rectangle bounds.
    pub fn draw_text_in_rect(&mut self, text: &str, rect: impl IntoSignal<Rect>) {
        if text.is_empty() || self.skip_draw_for_zero_alpha() {
            return;
        }
        let rect = self.resolve_signal(rect);
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let layout = self.build_text_layout(text, Some(rect.width()));
        let clip_path = rect_to_kurbo(rect).to_path(0.1);
        self.scene.push_clip_layer(
            self.current_state.fill_rule,
            self.current_state.transform,
            &clip_path,
        );
        self.draw_text_layout(&layout, rect.origin());
        self.scene.pop_layer();
    }

    /// Fills text at the specified position.
    pub fn fill_text(&mut self, text: &str, pos: impl IntoSignal<Point>) {
        self.draw_text(text, pos);
    }

    /// Strokes text at the specified position.
    pub fn stroke_text(&mut self, text: &str, pos: impl IntoSignal<Point>) {
        if text.is_empty() || self.skip_draw_for_zero_alpha() {
            return;
        }

        let pos = self.resolve_signal(pos);
        let layout = self.build_text_layout(text, None);
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        self.draw_text_layout_with_style(&layout, pos, &brush, (&stroke).into());
    }

    // ========================================================================
    // Internal Helper Methods
    // ========================================================================

    /// Resolves the current fill style to a peniko brush.
    fn resolve_fill_style(&self) -> peniko::Brush {
        match &self.current_state.fill_style {
            FillStyle::Color(color) => {
                let peniko_color = resolved_color_to_peniko(*color);
                peniko_color.into()
            }
            FillStyle::LinearGradient(gradient) => gradient.build(),
            FillStyle::RadialGradient(gradient) => gradient.build(),
            FillStyle::ConicGradient(gradient) => gradient.build(),
        }
    }

    /// Resolves the current stroke style to a peniko brush.
    fn resolve_stroke_style(&self) -> peniko::Brush {
        match &self.current_state.stroke_style {
            StrokeStyle::Color(color) => {
                let peniko_color = resolved_color_to_peniko(*color);
                peniko_color.into()
            }
            StrokeStyle::LinearGradient(gradient) => gradient.build(),
            StrokeStyle::RadialGradient(gradient) => gradient.build(),
            StrokeStyle::ConicGradient(gradient) => gradient.build(),
        }
    }

    #[inline]
    const fn normalized_global_alpha(&self) -> f32 {
        self.current_state.global_alpha.clamp(0.0, 1.0)
    }

    #[inline]
    fn skip_draw_for_zero_alpha(&self) -> bool {
        self.normalized_global_alpha() <= 0.0
    }

    fn push_global_alpha_layer_if_needed(&mut self, clip_shape: &kurbo::BezPath) -> bool {
        let alpha = self.normalized_global_alpha();
        if alpha >= 1.0 {
            return false;
        }
        self.scene.push_layer(
            self.current_state.fill_rule,
            self.current_state.blend_mode,
            alpha,
            self.current_state.transform,
            clip_shape,
        );
        true
    }

    #[inline]
    fn pop_global_alpha_layer_if_needed(&mut self, pushed: bool) {
        if pushed {
            self.scene.pop_layer();
        }
    }

    fn build_text_layout(&mut self, text: &str, max_width: Option<f32>) -> parley::Layout<[u8; 4]> {
        build_text_layout_with_engine(self.text_engine, &self.current_state.font, text, max_width)
    }

    fn draw_text_layout(&mut self, layout: &parley::Layout<[u8; 4]>, origin: Point) {
        let brush = self.resolve_fill_style();
        self.draw_text_layout_with_style(layout, origin, &brush, peniko::Fill::NonZero.into());
    }

    fn draw_text_layout_with_style(
        &mut self,
        layout: &parley::Layout<[u8; 4]>,
        origin: Point,
        brush: &peniko::Brush,
        style: peniko::StyleRef<'_>,
    ) {
        if layout.is_empty() {
            return;
        }

        let alpha = self.normalized_global_alpha();
        let transform = self.current_state.transform
            * kurbo::Affine::translate((f64::from(origin.x), f64::from(origin.y)));
        let mut glyphs = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    let run = glyph_run.run();
                    let mut run_x = glyph_run.offset();
                    let run_y = glyph_run.baseline();
                    glyphs.clear();
                    glyphs.extend(glyph_run.glyphs().map(|glyph| {
                        let positioned = Glyph {
                            id: glyph.id,
                            x: run_x + glyph.x,
                            y: run_y - glyph.y,
                        };
                        run_x += glyph.advance;
                        positioned
                    }));

                    self.scene.draw_glyph_run(&GlyphRun {
                        font: run.font(),
                        font_size: run.font_size(),
                        normalized_coords: run.normalized_coords(),
                        transform,
                        brush,
                        brush_alpha: alpha,
                        style,
                        glyphs: &glyphs,
                    });
                }
            }
        }
    }
}

fn build_text_layout_with_engine(
    text_engine: &mut TextEngine,
    font: &FontSpec,
    text: &str,
    max_width: Option<f32>,
) -> parley::Layout<[u8; 4]> {
    let family = font.family.trim().to_owned();
    let mut layout_cx = text_engine.layout_cx.take().unwrap_or_default();
    let mut builder = layout_cx.ranged_builder(text_engine.font_cx(), text, 1.0, true);
    builder.push_default(parley::StyleProperty::Brush([0, 0, 0, 255]));
    builder.push_default(parley::StyleProperty::FontSize(font.size));
    builder.push_default(parley::StyleProperty::FontWeight(parley_font_weight(
        font.weight,
    )));
    builder.push_default(parley::StyleProperty::FontStyle(parley_font_style(
        font.style,
    )));
    if !family.is_empty() {
        builder.push_default(parley::StyleProperty::FontFamily(
            parley::FontFamily::named(&family),
        ));
    }

    let mut layout = builder.build(text);
    text_engine.layout_cx = Some(layout_cx);
    layout.break_all_lines(max_width);
    layout.align(
        parley::Alignment::Start,
        parley::AlignmentOptions::default(),
    );
    layout
}

fn parley_font_weight(weight: FontWeight) -> parley::FontWeight {
    parley::FontWeight::new(f32::from(weight.value()))
}

const fn parley_font_style(style: FontStyle) -> parley::FontStyle {
    match style {
        FontStyle::Normal => parley::FontStyle::Normal,
        FontStyle::Italic => parley::FontStyle::Italic,
        FontStyle::Oblique => parley::FontStyle::Oblique(None),
    }
}

struct CanvasContent {
    draw_fn: Box<dyn FnMut(&mut DrawingContext)>,
    invalidator: Option<SceneInvalidator>,
    pending_redraw: Rc<Cell<bool>>,
    active_guards: Vec<Box<dyn Any>>,
    text_engine: TextEngine,
}

impl SceneContent for CanvasContent {
    #[allow(clippy::cast_precision_loss)]
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) -> bool {
        self.active_guards.clear();
        self.pending_redraw.set(false);
        let mut ctx = DrawingContext {
            scene,
            width,
            height,
            state_stack: Vec::new(),
            current_state: DrawingState::new(),
            reactive: ReactiveFrameState {
                pending_redraw: Rc::clone(&self.pending_redraw),
                invalidator: self.invalidator.clone(),
                guards: &mut self.active_guards,
            },
            text_engine: &mut self.text_engine,
            requested_next_frame: false,
        };
        (self.draw_fn)(&mut ctx);
        let requested_next_frame = ctx.requested_next_frame;
        requested_next_frame || self.pending_redraw.replace(false)
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.invalidator = invalidator;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestScene {
        stroked_glyph_run: bool,
        image_transforms: Vec<kurbo::Affine>,
    }

    impl TestScene {
        const fn new() -> Self {
            Self {
                stroked_glyph_run: false,
                image_transforms: Vec::new(),
            }
        }
    }

    impl Scene2D for TestScene {
        fn fill(
            &mut self,
            _fill: peniko::Fill,
            _transform: kurbo::Affine,
            _brush: &peniko::Brush,
            _brush_transform: Option<kurbo::Affine>,
            _shape: &kurbo::BezPath,
        ) {
        }

        fn stroke(
            &mut self,
            _stroke: &kurbo::Stroke,
            _transform: kurbo::Affine,
            _brush: &peniko::Brush,
            _brush_transform: Option<kurbo::Affine>,
            _shape: &kurbo::BezPath,
        ) {
        }

        fn push_layer(
            &mut self,
            _fill: peniko::Fill,
            _blend: peniko::BlendMode,
            _alpha: f32,
            _transform: kurbo::Affine,
            _clip: &kurbo::BezPath,
        ) {
        }

        fn push_clip_layer(
            &mut self,
            _fill: peniko::Fill,
            _transform: kurbo::Affine,
            _clip: &kurbo::BezPath,
        ) {
        }

        fn pop_layer(&mut self) {}

        fn draw_image(&mut self, _image: &peniko::ImageBrush, transform: kurbo::Affine) {
            self.image_transforms.push(transform);
        }

        fn draw_glyph_run(&mut self, run: &waterui_graphics::GlyphRun<'_>) {
            if matches!(run.style, peniko::StyleRef::Stroke(_)) {
                self.stroked_glyph_run = true;
            }
        }

        fn append_vello_scene(&mut self, _scene: &vello::Scene, _transform: Option<kurbo::Affine>) {
        }

        fn reset(&mut self) {}
    }

    /// Draws every kind of command a scene carries: solid and gradient fills,
    /// a stroke, a clipped layer, and a run of text.
    fn draw_every_command(ctx: &mut DrawingContext<'_>) {
        ctx.set_fill_style(waterui_graphics::color::Srgb::new(0.09, 0.10, 0.15));
        ctx.fill_rect(Rect::from_size(Size::new(ctx.width, ctx.height)));

        let mut gradient = ctx.create_linear_gradient(24.0, 24.0, 216.0, 120.0);
        gradient.add_color_stop(0.0, waterui_graphics::color::Srgb::new(0.95, 0.55, 0.66));
        gradient.add_color_stop(1.0, waterui_graphics::color::Srgb::new(0.35, 0.63, 0.94));
        ctx.set_fill_style(gradient);
        ctx.fill_rect(Rect::new(Point::new(24.0, 24.0), Size::new(192.0, 96.0)));

        ctx.set_stroke_style(waterui_graphics::color::Srgb::new(1.0, 0.84, 0.32));
        ctx.set_line_width(6.0);
        ctx.stroke_circle(Point::new(300.0, 72.0), 40.0);

        ctx.push_clip_rect(Rect::new(Point::new(24.0, 140.0), Size::new(150.0, 60.0)));
        ctx.set_fill_style(waterui_graphics::color::Srgb::new(0.40, 0.85, 0.60));
        ctx.fill_circle(Point::new(96.0, 170.0), 60.0);
        ctx.pop_layer();

        ctx.set_fill_style(waterui_graphics::color::Srgb::new(0.98, 0.98, 1.0));
        ctx.set_font(FontSpec::new("", 28.0));
        ctx.fill_text("Scene engines", Point::new(200.0, 170.0));
    }

    /// Renders the same drawing through both scene engines.
    ///
    /// This machine's GPU runs the compute pipeline, so it can run either
    /// engine; a device without indirect execution can only run the hybrid one,
    /// and this is how the two are compared where both are available. The PNGs
    /// are for looking at — the engines rasterize differently, so nothing about
    /// them is asserted pixel-wise.
    #[test]
    fn both_scene_engines_render_a_canvas() {
        use waterui_graphics::shared_context::SceneEngine;
        use waterui_graphics::{GpuRuntime, OffscreenRenderConfig, OffscreenSize};

        let directory = std::path::Path::new("/tmp/waterui_scene_engines");
        std::fs::create_dir_all(directory).expect("output directory must be creatable");
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("scene engine comparison requires a working GPU runtime");
        let size = OffscreenSize::try_from_pixels(400, 220).expect("test size must be valid");

        for (engine, name) in [
            (SceneEngine::Classic, "classic"),
            (SceneEngine::Hybrid, "hybrid"),
        ] {
            let surface = SceneView::new(CanvasContent {
                draw_fn: Box::new(draw_every_command),
                invalidator: None,
                pending_redraw: Rc::new(Cell::new(false)),
                active_guards: Vec::new(),
                text_engine: TextEngine::default(),
            })
            .into_gpu_surface();
            let config = OffscreenRenderConfig::new(size)
                .format(wgpu::TextureFormat::Rgba8Unorm)
                .scene_engine(engine);
            let mut env = waterui_core::Environment::new();
            let output = pollster::block_on(surface.render_offscreen(&runtime, config, &mut env))
                .expect("offscreen render should succeed");
            output
                .save_png(directory.join(alloc::format!("{name}.png")))
                .expect("png should be written");
        }
    }

    #[test]
    fn measure_text_uses_real_layout_metrics() {
        let mut scene = TestScene::new();
        let mut text_engine = TextEngine::default();
        let mut guards = Vec::new();
        let mut ctx = DrawingContext {
            scene: &mut scene,
            width: 640.0,
            height: 480.0,
            state_stack: Vec::new(),
            current_state: DrawingState::new(),
            reactive: ReactiveFrameState {
                pending_redraw: Rc::new(Cell::new(false)),
                invalidator: None,
                guards: &mut guards,
            },
            text_engine: &mut text_engine,
            requested_next_frame: false,
        };

        let metrics = ctx.measure_text("Hello, canvas");
        assert!(metrics.width > 0.0, "expected positive text width");
        assert!(metrics.height > 0.0, "expected positive text height");
        assert_eq!(ctx.measure_text("").width, 0.0);
        assert_eq!(ctx.measure_text("").height, 0.0);
    }

    #[test]
    fn draw_image_sub_maps_source_rect_onto_destination_rect() {
        let mut scene = TestScene::new();
        let mut text_engine = TextEngine::default();
        let mut guards = Vec::new();

        // A non-origin source rect and a non-unit scale: the sprite at
        // (32, 16)..(64, 48) must land exactly on (100, 100)..(164, 164).
        let src = Rect::new(Point::new(32.0, 16.0), Size::new(32.0, 32.0));
        let dest = Rect::new(Point::new(100.0, 100.0), Size::new(64.0, 64.0));

        {
            let mut ctx = DrawingContext {
                scene: &mut scene,
                width: 640.0,
                height: 480.0,
                state_stack: Vec::new(),
                current_state: DrawingState::new(),
                reactive: ReactiveFrameState {
                    pending_redraw: Rc::new(Cell::new(false)),
                    invalidator: None,
                    guards: &mut guards,
                },
                text_engine: &mut text_engine,
                requested_next_frame: false,
            };
            let pixels = alloc::vec![0u8; 128 * 128 * 4];
            let image = CanvasImage::from_rgba_pixels(128, 128, &pixels)
                .expect("test image must be constructible");
            ctx.draw_image_sub(&image, src, dest);
        }

        let transform = *scene
            .image_transforms
            .first()
            .expect("draw_image_sub must emit exactly one image draw");
        let src_origin = transform * kurbo::Point::new(32.0, 16.0);
        let src_corner = transform * kurbo::Point::new(64.0, 48.0);
        assert!(
            (src_origin.x - 100.0).abs() < 1e-6 && (src_origin.y - 100.0).abs() < 1e-6,
            "source origin must map to destination origin, got {src_origin:?}"
        );
        assert!(
            (src_corner.x - 164.0).abs() < 1e-6 && (src_corner.y - 164.0).abs() < 1e-6,
            "source corner must map to destination corner, got {src_corner:?}"
        );
    }

    #[test]
    fn stroke_text_appends_scene_commands() {
        let mut scene = TestScene::new();
        let mut text_engine = TextEngine::default();
        let mut guards = Vec::new();

        {
            let mut ctx = DrawingContext {
                scene: &mut scene,
                width: 640.0,
                height: 480.0,
                state_stack: Vec::new(),
                current_state: DrawingState::new(),
                reactive: ReactiveFrameState {
                    pending_redraw: Rc::new(Cell::new(false)),
                    invalidator: None,
                    guards: &mut guards,
                },
                text_engine: &mut text_engine,
                requested_next_frame: false,
            };
            ctx.stroke_text("Stroke", Point::new(24.0, 32.0));
        }

        assert!(
            scene.stroked_glyph_run,
            "expected stroke_text to draw a stroked glyph run"
        );
    }
}
