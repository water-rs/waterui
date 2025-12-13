//! Canvas view for 2D vector graphics rendering.
//!
//! `Canvas` provides an easy-to-use API for drawing 2D graphics using Vello.
//! It renders at full GPU speed while exposing a simple, declarative interface.
//!
//! # Example
//!
//! ```ignore
//! use waterui::graphics::Canvas;
//! use waterui::prelude::*;
//!
//! Canvas::new(|ctx: &mut DrawingContext| {
//!     // Fill a rectangle
//!     let rect = Rect::from_size(Size::new(200.0, 150.0));
//!     ctx.set_fill_style(Color::red());
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

use crate::conversions::{rect_to_kurbo, resolved_color_to_peniko};
use crate::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};
use crate::gradient::{ConicGradient, LinearGradient, RadialGradient};
use crate::image::CanvasImage;
use crate::path::Path;
use crate::state::{DrawingState, FillStyle, LineCap, LineJoin, StrokeStyle};
use waterui_core::layout::{Point, Rect};

// Internal imports for rendering (not exposed to users)
use vello::{kurbo, peniko};

// For signal operations
use nami::Signal;

/// A canvas for 2D vector graphics rendering.
///
/// Canvas provides a simple callback-based API where you receive a
/// [`DrawingContext`] to draw shapes, paths, and text.
pub struct Canvas {
    inner: GpuSurface,
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
    ///     ctx.fill(Circle::new((50.0, 50.0), 25.0), Color::RED);
    /// })
    /// ```
    #[must_use]
    pub fn new<F>(draw: F) -> Self
    where
        F: FnMut(&mut DrawingContext) + Send + 'static,
    {
        Self {
            inner: GpuSurface::new(CanvasRenderer::new(draw)),
        }
    }
}

impl waterui_core::View for Canvas {
    fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
        self.inner
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
    scene: &'a mut vello::Scene,
    env: &'a waterui_core::Environment,
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
    /// Returns the size of the canvas as a `kurbo::Size`.
    #[must_use]
    pub fn size(&self) -> kurbo::Size {
        kurbo::Size::new(f64::from(self.width), f64::from(self.height))
    }

    /// Returns the center point of the canvas.
    #[must_use]
    pub fn center(&self) -> kurbo::Point {
        kurbo::Point::new(f64::from(self.width) / 2.0, f64::from(self.height) / 2.0)
    }

    /// Fills a shape with a color.
    ///
    /// # Example
    ///
    /// ```ignore
    /// ctx.fill(Circle::new((100.0, 100.0), 50.0), Color::RED);
    /// ```
    pub fn fill(&mut self, shape: impl kurbo::Shape, color: peniko::Color) {
        self.scene.fill(
            self.current_state.fill_rule,
            kurbo::Affine::IDENTITY,
            color,
            None,
            &shape,
        );
    }

    /// Fills a shape with a brush (gradient, pattern, etc).
    pub fn fill_brush(&mut self, shape: impl kurbo::Shape, brush: &peniko::Brush) {
        self.scene.fill(
            self.current_state.fill_rule,
            kurbo::Affine::IDENTITY,
            brush,
            None,
            &shape,
        );
    }

    /// Fills a shape with a color and custom transform.
    pub fn fill_with_transform(
        &mut self,
        shape: impl kurbo::Shape,
        color: peniko::Color,
        transform: kurbo::Affine,
    ) {
        self.scene
            .fill(self.current_state.fill_rule, transform, color, None, &shape);
    }

    /// Strokes a shape with a color and line width.
    ///
    /// # Example
    ///
    /// ```ignore
    /// ctx.stroke(Rect::new(10.0, 10.0, 100.0, 80.0), Color::BLUE, 2.0);
    /// ```
    pub fn stroke(&mut self, shape: impl kurbo::Shape, color: peniko::Color, width: f64) {
        let stroke = kurbo::Stroke::new(width);
        self.scene
            .stroke(&stroke, kurbo::Affine::IDENTITY, color, None, &shape);
    }

    /// Strokes a shape with a brush and line width.
    pub fn stroke_brush(&mut self, shape: impl kurbo::Shape, brush: &peniko::Brush, width: f64) {
        let stroke = kurbo::Stroke::new(width);
        self.scene
            .stroke(&stroke, kurbo::Affine::IDENTITY, brush, None, &shape);
    }

    /// Strokes a shape with custom stroke style.
    pub fn stroke_with_style(
        &mut self,
        shape: impl kurbo::Shape,
        color: peniko::Color,
        stroke: &kurbo::Stroke,
    ) {
        self.scene
            .stroke(stroke, kurbo::Affine::IDENTITY, color, None, &shape);
    }

    /// Strokes a shape with custom stroke style and transform.
    pub fn stroke_with_transform(
        &mut self,
        shape: impl kurbo::Shape,
        color: peniko::Color,
        stroke: &kurbo::Stroke,
        transform: kurbo::Affine,
    ) {
        self.scene.stroke(stroke, transform, color, None, &shape);
    }

    /// Pushes a clip layer. All subsequent drawing will be clipped to the shape.
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_clip(&mut self, clip: impl kurbo::Shape) {
        self.scene.push_clip_layer(kurbo::Affine::IDENTITY, &clip);
    }

    /// Pushes a layer with alpha (opacity).
    ///
    /// Call [`pop_layer`](Self::pop_layer) when done drawing in this layer.
    pub fn push_alpha(&mut self, alpha: f32, bounds: impl kurbo::Shape) {
        self.scene.push_layer(
            peniko::BlendMode::default(),
            alpha,
            kurbo::Affine::IDENTITY,
            &bounds,
        );
    }

    /// Pops the current layer.
    pub fn pop_layer(&mut self) {
        self.scene.pop_layer();
    }

    /// Access the underlying Vello scene for advanced operations.
    ///
    /// Use this when you need features not exposed by the simplified API.
    #[must_use]
    pub const fn scene(&mut self) -> &mut vello::Scene {
        self.scene
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
        self.current_state.transform = self.current_state.transform * translation;
    }

    /// Rotates the current transform by the given angle (in radians).
    ///
    /// Positive angles rotate clockwise.
    pub fn rotate(&mut self, angle: f32) {
        let rotation = kurbo::Affine::rotate(f64::from(angle));
        self.current_state.transform = self.current_state.transform * rotation;
    }

    /// Scales the current transform by (x, y).
    ///
    /// Values less than 1.0 shrink, greater than 1.0 enlarge.
    pub fn scale(&mut self, x: f32, y: f32) {
        let scale = kurbo::Affine::scale_non_uniform(f64::from(x), f64::from(y));
        self.current_state.transform = self.current_state.transform * scale;
    }

    /// Applies an arbitrary affine transform.
    ///
    /// The transform is specified as a 2x3 matrix: [a, b, c, d, e, f]
    /// which represents the matrix [[a, c, e], [b, d, f], [0, 0, 1]].
    pub fn transform(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) {
        let affine = kurbo::Affine::new([
            f64::from(a),
            f64::from(b),
            f64::from(c),
            f64::from(d),
            f64::from(e),
            f64::from(f),
        ]);
        self.current_state.transform = self.current_state.transform * affine;
    }

    /// Replaces the current transform with the specified matrix.
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
    pub fn reset_transform(&mut self) {
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
        let brush = self.resolve_fill_style();
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            &brush,
            None,
            path.inner(),
        );
    }

    /// Strokes a path with the current stroke style and line properties.
    pub fn stroke_path(&mut self, path: &Path) {
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        self.scene.stroke(
            &stroke,
            self.current_state.transform,
            &brush,
            None,
            path.inner(),
        );
    }

    // ========================================================================
    // Rectangle Convenience Methods (Phase 3)
    // ========================================================================

    /// Fills a rectangle with the current fill style.
    pub fn fill_rect(&mut self, rect: Rect) {
        let kurbo_rect = rect_to_kurbo(rect);
        let brush = self.resolve_fill_style();
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            &brush,
            None,
            &kurbo_rect,
        );
    }

    /// Strokes a rectangle with the current stroke style.
    pub fn stroke_rect(&mut self, rect: Rect) {
        let kurbo_rect = rect_to_kurbo(rect);
        let brush = self.resolve_stroke_style();
        let stroke = self.current_state.build_stroke();
        self.scene.stroke(
            &stroke,
            self.current_state.transform,
            &brush,
            None,
            &kurbo_rect,
        );
    }

    /// Clears a rectangle to transparent black.
    pub fn clear_rect(&mut self, rect: Rect) {
        let kurbo_rect = rect_to_kurbo(rect);
        let transparent = peniko::Color::TRANSPARENT;
        self.scene.fill(
            self.current_state.fill_rule,
            self.current_state.transform,
            transparent,
            None,
            &kurbo_rect,
        );
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
    pub fn set_line_width(&mut self, width: f32) {
        self.current_state.line_width = width;
    }

    /// Sets the line cap style (how stroke endpoints are drawn).
    pub fn set_line_cap(&mut self, cap: LineCap) {
        self.current_state.line_cap = cap;
    }

    /// Sets the line join style (how stroke corners are drawn).
    pub fn set_line_join(&mut self, join: LineJoin) {
        self.current_state.line_join = join;
    }

    /// Sets the miter limit for miter line joins.
    pub fn set_miter_limit(&mut self, limit: f32) {
        self.current_state.miter_limit = limit;
    }

    /// Sets the line dash pattern.
    ///
    /// Pass an empty vector to disable dashing.
    pub fn set_line_dash(&mut self, segments: Vec<f32>) {
        self.current_state.line_dash = segments;
    }

    /// Sets the line dash offset (where the dash pattern starts).
    pub fn set_line_dash_offset(&mut self, offset: f32) {
        self.current_state.line_dash_offset = offset;
    }

    /// Sets the global alpha (opacity) for all drawing operations.
    ///
    /// Values range from 0.0 (transparent) to 1.0 (opaque).
    pub fn set_global_alpha(&mut self, alpha: f32) {
        self.current_state.global_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Sets the blend mode for compositing operations.
    ///
    /// This controls how new shapes are blended with existing content.
    pub fn set_blend_mode(&mut self, mode: peniko::BlendMode) {
        self.current_state.blend_mode = mode;
    }

    /// Sets the shadow blur radius.
    ///
    /// A blur value of 0 means sharp shadows, higher values create softer shadows.
    pub fn set_shadow_blur(&mut self, blur: f32) {
        self.current_state.shadow_blur = blur.max(0.0);
    }

    /// Sets the shadow color.
    pub fn set_shadow_color(&mut self, color: impl Into<waterui_color::Color>) {
        self.current_state.shadow_color = color.into();
    }

    /// Sets the shadow offset in the x and y directions.
    ///
    /// # Arguments
    /// * `x` - Horizontal offset (positive = right)
    /// * `y` - Vertical offset (positive = down)
    pub fn set_shadow_offset(&mut self, x: f32, y: f32) {
        self.current_state.shadow_offset_x = x;
        self.current_state.shadow_offset_y = y;
    }

    /// Sets the fill rule for determining the interior of shapes.
    ///
    /// # Arguments
    /// * `rule` - The fill rule to use (`NonZero` or `EvenOdd`)
    ///
    /// NonZero (default): A point is inside the path if a ray from the point crosses a non-zero net number of path segments.
    /// EvenOdd: A point is inside the path if a ray from the point crosses an odd number of path segments.
    pub fn set_fill_rule(&mut self, rule: peniko::Fill) {
        self.current_state.fill_rule = rule;
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
    /// gradient.add_color_stop(0.0, Color::red());
    /// gradient.add_color_stop(1.0, Color::blue());
    /// ctx.set_fill_style(gradient);
    /// ```
    #[must_use]
    pub fn create_linear_gradient(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> LinearGradient {
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
    /// gradient.add_color_stop(0.0, Color::white());
    /// gradient.add_color_stop(1.0, Color::black());
    /// ctx.set_fill_style(gradient);
    /// ```
    #[must_use]
    pub fn create_radial_gradient(
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
    /// gradient.add_color_stop(0.0, Color::red());
    /// gradient.add_color_stop(0.5, Color::green());
    /// gradient.add_color_stop(1.0, Color::blue());
    /// ctx.set_fill_style(gradient);
    /// ```
    #[must_use]
    pub fn create_conic_gradient(&self, start_angle: f32, x: f32, y: f32) -> ConicGradient {
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
        // Calculate transform to scale image to destination rectangle
        let scale_x = f64::from(dest.size().width) / f64::from(image.width());
        let scale_y = f64::from(dest.size().height) / f64::from(image.height());

        // Create transform: translate to dest position, then scale
        let image_transform = kurbo::Affine::translate((
            f64::from(dest.origin().x),
            f64::from(dest.origin().y),
        )) * kurbo::Affine::scale_non_uniform(scale_x, scale_y);

        // Compose with current transform
        let final_transform = self.current_state.transform * image_transform;

        // Wrap ImageData in ImageBrush
        let image_brush = peniko::ImageBrush::new(image.inner().clone());

        // Draw image using vello
        self.scene.draw_image(&image_brush, final_transform);
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
        // Use push_clip_layer with clip to render only the source rectangle
        // Calculate transform for the sub-rectangle

        // First, translate to negate the source offset
        let src_offset = kurbo::Affine::translate((
            -f64::from(src.origin().x),
            -f64::from(src.origin().y),
        ));

        // Then scale from source size to destination size
        let scale_x = f64::from(dest.size().width) / f64::from(src.size().width);
        let scale_y = f64::from(dest.size().height) / f64::from(src.size().height);
        let scale = kurbo::Affine::scale_non_uniform(scale_x, scale_y);

        // Finally, translate to destination position
        let dest_offset = kurbo::Affine::translate((
            f64::from(dest.origin().x),
            f64::from(dest.origin().y),
        ));

        // Compose transforms: src_offset -> scale -> dest_offset
        let image_transform = src_offset * scale * dest_offset;

        // Compose with current transform
        let final_transform = self.current_state.transform * image_transform;

        // Create clip rectangle at destination
        let clip_rect = rect_to_kurbo(dest);

        // Push a clipped layer, draw the image, then pop
        self.scene
            .push_clip_layer(self.current_state.transform, &clip_rect);

        // Wrap ImageData in ImageBrush
        let image_brush = peniko::ImageBrush::new(image.inner().clone());

        self.scene.draw_image(&image_brush, final_transform);

        self.scene.pop_layer();
    }

    // ========================================================================
    // Internal Helper Methods
    // ========================================================================

    /// Resolves the current fill style to a peniko brush.
    fn resolve_fill_style(&self) -> peniko::Brush {
        match &self.current_state.fill_style {
            FillStyle::Color(color) => {
                // Resolve the color in the current environment
                let computed = color.resolve(self.env);
                // Get the current value from the computed signal
                let resolved = computed.get();
                let peniko_color = resolved_color_to_peniko(resolved);
                peniko_color.into()
            }
            FillStyle::LinearGradient(gradient) => gradient.build(self.env),
            FillStyle::RadialGradient(gradient) => gradient.build(self.env),
            FillStyle::ConicGradient(gradient) => gradient.build(self.env),
        }
    }

    /// Resolves the current stroke style to a peniko brush.
    fn resolve_stroke_style(&self) -> peniko::Brush {
        match &self.current_state.stroke_style {
            StrokeStyle::Color(color) => {
                // Resolve the color in the current environment
                let computed = color.resolve(self.env);
                // Get the current value from the computed signal
                let resolved = computed.get();
                let peniko_color = resolved_color_to_peniko(resolved);
                peniko_color.into()
            }
            StrokeStyle::LinearGradient(gradient) => gradient.build(self.env),
            StrokeStyle::RadialGradient(gradient) => gradient.build(self.env),
            StrokeStyle::ConicGradient(gradient) => gradient.build(self.env),
        }
    }
}

/// Internal renderer that bridges Canvas to `GpuSurface`.
struct CanvasRenderer<F> {
    draw_fn: F,
    scene: vello::Scene,
    renderer: Option<vello::Renderer>,
    /// Intermediate texture for Vello (Rgba8Unorm format required by Vello)
    intermediate_texture: Option<wgpu::Texture>,
    intermediate_view: Option<wgpu::TextureView>,
    /// Blit pipeline for copying intermediate texture to target (handles HDR surfaces)
    blit_pipeline: Option<wgpu::RenderPipeline>,
    blit_bind_group_layout: Option<wgpu::BindGroupLayout>,
    blit_sampler: Option<wgpu::Sampler>,
    /// Current intermediate texture dimensions
    intermediate_size: (u32, u32),
}

impl<F> CanvasRenderer<F> {
    fn new(draw_fn: F) -> Self {
        Self {
            draw_fn,
            scene: vello::Scene::new(),
            renderer: None,
            intermediate_texture: None,
            intermediate_view: None,
            blit_pipeline: None,
            blit_bind_group_layout: None,
            blit_sampler: None,
            intermediate_size: (0, 0),
        }
    }
}

impl<F> GpuRenderer for CanvasRenderer<F>
where
    F: FnMut(&mut DrawingContext) + Send + 'static,
{
    fn setup(&mut self, ctx: &GpuContext) {
        let renderer = vello::Renderer::new(
            ctx.device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::area_only(),
                num_init_threads: std::num::NonZeroUsize::new(1), // Single thread on macOS
                pipeline_cache: None,
            },
        )
        .expect("Failed to create Vello renderer");
        self.renderer = Some(renderer);

        // Create blit pipeline for copying from Rgba8Unorm to target format
        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Canvas Blit Shader"),
                source: wgpu::ShaderSource::Wgsl(BLIT_SHADER.into()),
            });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Canvas Blit Bind Group Layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Canvas Blit Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Canvas Blit Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            });

        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Canvas Blit Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        self.blit_pipeline = Some(pipeline);
        self.blit_bind_group_layout = Some(bind_group_layout);
        self.blit_sampler = Some(sampler);
    }

    fn resize(&mut self, width: u32, height: u32) {
        // Mark that we need to recreate the intermediate texture
        self.intermediate_size = (0, 0);
        let _ = (width, height);
    }

    fn render(&mut self, frame: &GpuFrame) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        // Recreate intermediate texture if size changed
        if self.intermediate_size != (frame.width, frame.height) {
            let texture = frame.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Canvas Intermediate Texture"),
                size: wgpu::Extent3d {
                    width: frame.width,
                    height: frame.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // Vello requires Rgba8Unorm format
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.intermediate_texture = Some(texture);
            self.intermediate_view = Some(view);
            self.intermediate_size = (frame.width, frame.height);
        }

        let Some(intermediate_view) = &self.intermediate_view else {
            return;
        };

        // Clear and rebuild scene
        self.scene.reset();

        // Create drawing context and invoke user's draw function
        #[allow(clippy::cast_precision_loss)]
        let mut ctx = DrawingContext {
            scene: &mut self.scene,
            env: &waterui_core::Environment::new(), // TODO: Thread actual environment in future phase
            width: frame.width as f32,
            height: frame.height as f32,
            state_stack: Vec::new(),
            current_state: DrawingState::default(),
        };
        (self.draw_fn)(&mut ctx);

        // Render the scene to intermediate texture (Rgba8Unorm)
        let params = vello::RenderParams {
            base_color: peniko::Color::TRANSPARENT,
            width: frame.width,
            height: frame.height,
            antialiasing_method: vello::AaConfig::Area,
        };

        renderer
            .render_to_texture(
                frame.device,
                frame.queue,
                &self.scene,
                intermediate_view,
                &params,
            )
            .expect("Failed to render Vello scene");

        // Blit from intermediate texture to target (may be HDR format)
        let Some(pipeline) = &self.blit_pipeline else {
            return;
        };
        let Some(bind_group_layout) = &self.blit_bind_group_layout else {
            return;
        };
        let Some(sampler) = &self.blit_sampler else {
            return;
        };

        let bind_group = frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Canvas Blit Bind Group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Canvas Blit Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Canvas Blit Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..6, 0..1);
        }

        frame.queue.submit(std::iter::once(encoder.finish()));
    }
}

/// WGSL shader for blitting from Rgba8Unorm to target format
const BLIT_SHADER: &str = r"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Full-screen triangle pair
    var positions = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
    );
    var tex_coords = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 0.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
    );

    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.tex_coord = tex_coords[vertex_index];
    return output;
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_source, s_source, in.tex_coord);
}
";
