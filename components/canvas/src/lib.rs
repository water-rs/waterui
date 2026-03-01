//! Canvas view for 2D vector graphics rendering.
//!
//! `Canvas` provides an easy-to-use API for drawing 2D graphics using Vello.
//! It renders at full GPU speed while exposing a simple, declarative interface.
//!
//! # Example
//!
//! ```ignore
//! use waterui_canvas::Canvas;
//! use waterui_graphics::color::Srgb;
//! use waterui::prelude::*;
//!
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
//! ```
//!
//! /// Drawing state management for Canvas.

#![allow(clippy::multiple_crate_versions)]

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

pub use path::Path;

pub use state::{LineCap, LineJoin};

pub use state::FillRule;

pub use gradient::{ConicGradient, LinearGradient, RadialGradient};

pub use image::{CanvasImage, ImageError};

pub use text::{FontSpec, FontStyle, FontWeight, TextMetrics};

use alloc::boxed::Box;

use waterui_core::layout::{Point, Rect, Size};

// Internal imports for rendering (not exposed to users)
use kurbo::{self, Shape as _};
use peniko;

use crate::conversions::{point_to_kurbo, rect_to_kurbo, resolved_color_to_peniko};
use crate::state::{DrawingState, FillStyle, StrokeStyle};
use waterui_graphics::{Scene2D, SceneContent, SceneView};

/// A canvas for 2D vector graphics rendering.
///
/// Canvas provides a simple callback-based API where you receive a
/// [`DrawingContext`] to draw shapes, paths, and text.
pub struct Canvas {
    draw_fn: Box<dyn FnMut(&mut DrawingContext) + Send>,
}

impl core::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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
    /// ```ignore
    /// Canvas::new(|ctx| {
    ///     ctx.set_fill_style(waterui_graphics::color::Srgb::new_u8(242, 140, 168));
    ///     ctx.fill_circle(Point::new(50.0, 50.0), 25.0);
    /// })
    /// ```
    #[must_use]
    pub fn new<F>(draw: F) -> Self
    where
        F: FnMut(&mut DrawingContext) + Send + 'static,
    {
        Self {
            draw_fn: Box::new(draw),
        }
    }
}

impl waterui_core::View for Canvas {
    fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
        SceneView::new(CanvasContent {
            draw_fn: self.draw_fn,
        })
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
}

impl core::fmt::Debug for DrawingContext<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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

    /// Pushes a clip layer, clipping subsequent drawing to the given rectangle.
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_clip_rect(&mut self, rect: Rect) {
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
    pub fn push_alpha_rect(&mut self, alpha: f32, rect: Rect) {
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
    pub fn push_alpha_path(&mut self, alpha: f32, path: &Path) {
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
    // State Management (Phase 1)
    // ========================================================================

    /// Saves the current drawing state to the stack.
    ///
    /// This saves transforms, styles, line properties, and other state.
    /// Call `restore()` to pop the saved state.
    ///
    /// # Example
    ///
    /// ```ignore
    /// ctx.save();
    /// ctx.translate(100.0, 50.0);
    /// ctx.rotate(0.785);
    /// // ... draw with transform ...
    /// ctx.restore(); // Back to original state
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
    pub fn translate(&mut self, x: f32, y: f32) {
        let translation = kurbo::Affine::translate((f64::from(x), f64::from(y)));
        self.current_state.transform *= translation;
    }

    /// Rotates the current transform by the given angle (in radians).
    ///
    /// Positive angles rotate clockwise.
    pub fn rotate(&mut self, angle: f32) {
        let rotation = kurbo::Affine::rotate(f64::from(angle));
        self.current_state.transform *= rotation;
    }

    /// Scales the current transform by (x, y).
    ///
    /// Values less than 1.0 shrink, greater than 1.0 enlarge.
    pub fn scale(&mut self, x: f32, y: f32) {
        let scale = kurbo::Affine::scale_non_uniform(f64::from(x), f64::from(y));
        self.current_state.transform *= scale;
    }

    /// Applies an arbitrary affine transform.
    ///
    /// The transform is specified as a 2x3 matrix: [a, b, c, d, e, f]
    /// which represents the matrix [[a, c, e], [b, d, f], [0, 0, 1]].
    #[allow(clippy::many_single_char_names)]
    pub fn transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        let affine = kurbo::Affine::new([
            f64::from(a),
            f64::from(b),
            f64::from(c),
            f64::from(d),
            f64::from(e),
            f64::from(f),
        ]);
        self.current_state.transform *= affine;
    }

    /// Replaces the current transform with the specified matrix.
    #[allow(clippy::many_single_char_names)]
    pub fn set_transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        self.current_state.transform = kurbo::Affine::new([
            f64::from(a),
            f64::from(b),
            f64::from(c),
            f64::from(d),
            f64::from(e),
            f64::from(f),
        ]);
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
        self.scene
            .stroke(&stroke, self.current_state.transform, &brush, path.inner());
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    // ========================================================================
    // Rectangle Convenience Methods (Phase 3)
    // ========================================================================

    /// Fills a rectangle with the current fill style.
    pub fn fill_rect(&mut self, rect: Rect) {
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
            &shape_path,
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Strokes a rectangle with the current stroke style.
    pub fn stroke_rect(&mut self, rect: Rect) {
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let kurbo_rect = rect_to_kurbo(rect);
        let shape_path = kurbo_rect.to_path(0.1);
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene
            .stroke(&stroke, self.current_state.transform, &brush, &shape_path);
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Clears a rectangle to transparent black.
    pub fn clear_rect(&mut self, rect: Rect) {
        let kurbo_rect = rect_to_kurbo(rect);
        let shape_path = kurbo_rect.to_path(0.1);
        let brush: peniko::Brush = peniko::Color::TRANSPARENT.into();
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            &brush,
            &shape_path,
        );
    }

    // ========================================================================
    // Shape Convenience Methods
    // ========================================================================

    /// Fills a circle with the current fill style.
    pub fn fill_circle(&mut self, center: Point, radius: f32) {
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
            &shape_path,
        );
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Strokes a circle with the current stroke style.
    pub fn stroke_circle(&mut self, center: Point, radius: f32) {
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        let circle = kurbo::Circle::new(point_to_kurbo(center), f64::from(radius));
        let shape_path = circle.to_path(0.1);
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene
            .stroke(&stroke, self.current_state.transform, &brush, &shape_path);
        self.pop_global_alpha_layer_if_needed(pushed_alpha);
    }

    /// Strokes a line segment with the current stroke style.
    pub fn stroke_line(&mut self, start: Point, end: Point) {
        if self.skip_draw_for_zero_alpha() {
            return;
        }
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        let line = kurbo::Line::new(point_to_kurbo(start), point_to_kurbo(end));
        let shape_path = line.to_path(0.1);
        let pushed_alpha = self.push_global_alpha_layer_if_needed(&shape_path);
        self.scene
            .stroke(&stroke, self.current_state.transform, &brush, &shape_path);
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
    pub const fn set_line_width(&mut self, width: f32) {
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
    pub const fn set_miter_limit(&mut self, limit: f32) {
        self.current_state.miter_limit = limit;
    }

    /// Sets the line dash pattern.
    ///
    /// Pass an empty vector to disable dashing.
    pub fn set_line_dash(&mut self, segments: Vec<f32>) {
        self.current_state.line_dash = segments;
    }

    /// Sets the line dash offset (where the dash pattern starts).
    pub const fn set_line_dash_offset(&mut self, offset: f32) {
        self.current_state.line_dash_offset = offset;
    }

    /// Sets the global alpha (opacity) for all drawing operations.
    ///
    /// Values range from 0.0 (transparent) to 1.0 (opaque).
    pub const fn set_global_alpha(&mut self, alpha: f32) {
        self.current_state.global_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Sets the shadow blur radius.
    ///
    /// A blur value of 0 means sharp shadows, higher values create softer shadows.
    pub const fn set_shadow_blur(&mut self, blur: f32) {
        self.current_state.shadow_blur = blur.max(0.0);
    }

    /// Sets the shadow color.
    pub fn set_shadow_color(&mut self, color: impl Into<waterui_graphics::color::ResolvedColor>) {
        self.current_state.shadow_color = color.into();
    }

    /// Sets the shadow offset in the x and y directions.
    ///
    /// # Arguments
    /// * `x` - Horizontal offset (positive = right)
    /// * `y` - Vertical offset (positive = down)
    pub const fn set_shadow_offset(&mut self, x: f32, y: f32) {
        self.current_state.shadow_offset_x = x;
        self.current_state.shadow_offset_y = y;
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
    /// ```ignore
    /// let mut gradient = ctx.create_linear_gradient(0.0, 0.0, 100.0, 100.0);
    /// gradient.add_color_stop(0.0, waterui_graphics::color::Srgb::new(1.0, 0.0, 0.0));
    /// gradient.add_color_stop(1.0, waterui_graphics::color::Srgb::new(0.0, 0.0, 1.0));
    /// ctx.set_fill_style(gradient);
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
    /// ```ignore
    /// let mut gradient = ctx.create_radial_gradient(50.0, 50.0, 10.0, 50.0, 50.0, 50.0);
    /// gradient.add_color_stop(0.0, waterui_graphics::color::Srgb::new(1.0, 1.0, 1.0));
    /// gradient.add_color_stop(1.0, waterui_graphics::color::Srgb::new(0.0, 0.0, 0.0));
    /// ctx.set_fill_style(gradient);
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
    /// ```ignore
    /// let mut gradient = ctx.create_conic_gradient(0.0, 50.0, 50.0);
    /// gradient.add_color_stop(0.0, waterui_graphics::color::Srgb::new(1.0, 0.0, 0.0));
    /// gradient.add_color_stop(0.5, waterui_graphics::color::Srgb::new(0.0, 1.0, 0.0));
    /// gradient.add_color_stop(1.0, waterui_graphics::color::Srgb::new(0.0, 0.0, 1.0));
    /// ctx.set_fill_style(gradient);
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
    /// ```ignore
    /// let image = CanvasImage::from_bytes(png_data)?;
    /// ctx.draw_image(&image, Point::new(10.0, 10.0));
    /// ```
    pub fn draw_image(&mut self, image: &CanvasImage, pos: Point) {
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
    /// ```ignore
    /// let image = CanvasImage::from_bytes(png_data)?;
    /// let dest = Rect::new(Point::ZERO, Size::new(200.0, 150.0));
    /// ctx.draw_image_scaled(&image, dest);
    /// ```
    pub fn draw_image_scaled(&mut self, image: &CanvasImage, dest: Rect) {
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
    /// ```ignore
    /// let sprite_sheet = CanvasImage::from_bytes(png_data)?;
    /// // Draw top-left 32x32 sprite at position (100, 100) scaled to 64x64
    /// let src = Rect::new(Point::ZERO, Size::new(32.0, 32.0));
    /// let dest = Rect::new(Point::new(100.0, 100.0), Size::new(64.0, 64.0));
    /// ctx.draw_image_sub(&sprite_sheet, src, dest);
    /// ```
    pub fn draw_image_sub(&mut self, image: &CanvasImage, src: Rect, dest: Rect) {
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

        // Compose transforms: src_offset -> scale -> dest_offset
        let image_transform = src_offset * scale * dest_offset;

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
    /// ```ignore
    /// ctx.set_font(FontSpec::new("Arial", 24.0).with_weight(FontWeight::Bold));
    /// ```
    pub fn set_font(&mut self, font: FontSpec) {
        self.current_state.font = font;
    }

    /// Measures the given text with the current font.
    ///
    /// Returns approximate text metrics. For more accurate measurements,
    /// use a dedicated text layout library.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let metrics = ctx.measure_text("Hello World");
    /// println!("Text width: {}", metrics.width);
    /// ```
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn measure_text(&self, text: &str) -> TextMetrics {
        // Simple approximation based on font size
        // In a real implementation, this would use Parley to layout the text
        let char_count = text.chars().count() as f32;
        let font_size = self.current_state.font.size;

        // Approximate width (assuming average character width is ~0.6 * font_size)
        let width = char_count * font_size * 0.6;
        let height = font_size;

        TextMetrics::new(width, height)
    }

    /// Fills text at the specified position.
    ///
    /// Note: This is a simplified implementation. Full text rendering with
    /// complex layouts, bidirectional text, and font fallbacks requires
    /// integration with Parley's text layout engine.
    ///
    /// # Example
    ///
    /// ```ignore
    /// ctx.fill_text("Hello World", Point::new(100.0, 100.0));
    /// ```
    pub fn fill_text(&mut self, _text: &str, _pos: Point) {
        // TODO: Implement full text rendering with Parley
        // This requires:
        // 1. Create a Parley layout with the text and current font
        // 2. Iterate through glyphs in the layout
        // 3. Use Scene::draw_glyphs() to render each glyph run
        // 4. Apply current fill style to glyphs

        tracing::warn!("fill_text is not yet fully implemented - requires Parley integration");
    }

    /// Strokes text at the specified position.
    ///
    /// Note: This is a simplified implementation. Stroke text requires
    /// converting glyphs to paths using skrifa.
    ///
    /// # Example
    ///
    /// ```ignore
    /// ctx.stroke_text("Hello World", Point::new(100.0, 100.0));
    /// ```
    pub fn stroke_text(&mut self, _text: &str, _pos: Point) {
        // TODO: Implement stroke text
        // This requires:
        // 1. Create a Parley layout with the text and current font
        // 2. For each glyph, use skrifa to convert it to a path
        // 3. Stroke each path with current stroke style

        tracing::warn!("stroke_text is not yet fully implemented - requires skrifa integration");
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
    fn normalized_global_alpha(&self) -> f32 {
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
}

struct CanvasContent {
    draw_fn: Box<dyn FnMut(&mut DrawingContext) + Send>,
}

impl SceneContent for CanvasContent {
    #[allow(clippy::cast_precision_loss)]
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, height: f32) {
        let mut ctx = DrawingContext {
            scene,
            width,
            height,
            state_stack: Vec::new(),
            current_state: DrawingState::new(),
        };
        (self.draw_fn)(&mut ctx);
    }
}
