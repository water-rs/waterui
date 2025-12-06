//! Canvas view for 2D vector graphics rendering.
//!
//! `Canvas` provides an easy-to-use API for drawing 2D graphics using Vello.
//! It renders at full GPU speed while exposing a simple, declarative interface.
//!
//! # Example
//!
//! ```ignore
//! use waterui::graphics::{Canvas, DrawingContext};
//! use waterui::graphics::kurbo::{Circle, Rect};
//! use waterui::graphics::peniko::Color;
//!
//! Canvas::new(|ctx: &mut DrawingContext| {
//!     // Fill a circle
//!     ctx.fill(
//!         Circle::new((100.0, 100.0), 50.0),
//!         Color::RED,
//!     );
//!
//!     // Stroke a rectangle
//!     ctx.stroke(
//!         Rect::new(10.0, 10.0, 200.0, 150.0),
//!         Color::BLUE,
//!         2.0,
//!     );
//! })
//! ```

use crate::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};

// Re-export vello types for user convenience
pub use vello::kurbo;
pub use vello::peniko;
pub use vello::peniko::Color;

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
pub struct DrawingContext<'a> {
    scene: &'a mut vello::Scene,
    /// Width of the canvas in pixels.
    pub width: f32,
    /// Height of the canvas in pixels.
    pub height: f32,
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
        kurbo::Point::new(
            f64::from(self.width) / 2.0,
            f64::from(self.height) / 2.0,
        )
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
            peniko::Fill::NonZero,
            kurbo::Affine::IDENTITY,
            color,
            None,
            &shape,
        );
    }

    /// Fills a shape with a brush (gradient, pattern, etc).
    pub fn fill_brush(&mut self, shape: impl kurbo::Shape, brush: &peniko::Brush) {
        self.scene.fill(
            peniko::Fill::NonZero,
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
        self.scene.fill(
            peniko::Fill::NonZero,
            transform,
            color,
            None,
            &shape,
        );
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
        self.scene.stroke(
            &stroke,
            kurbo::Affine::IDENTITY,
            color,
            None,
            &shape,
        );
    }

    /// Strokes a shape with a brush and line width.
    pub fn stroke_brush(
        &mut self,
        shape: impl kurbo::Shape,
        brush: &peniko::Brush,
        width: f64,
    ) {
        let stroke = kurbo::Stroke::new(width);
        self.scene.stroke(
            &stroke,
            kurbo::Affine::IDENTITY,
            brush,
            None,
            &shape,
        );
    }

    /// Strokes a shape with custom stroke style.
    pub fn stroke_with_style(
        &mut self,
        shape: impl kurbo::Shape,
        color: peniko::Color,
        stroke: &kurbo::Stroke,
    ) {
        self.scene.stroke(
            stroke,
            kurbo::Affine::IDENTITY,
            color,
            None,
            &shape,
        );
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
}

/// Internal renderer that bridges Canvas to `GpuSurface`.
struct CanvasRenderer<F> {
    draw_fn: F,
    scene: vello::Scene,
    renderer: Option<vello::Renderer>,
}

impl<F> CanvasRenderer<F> {
    fn new(draw_fn: F) -> Self {
        Self {
            draw_fn,
            scene: vello::Scene::new(),
            renderer: None,
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
    }

    fn render(&mut self, frame: &GpuFrame) {
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        // Clear and rebuild scene
        self.scene.reset();

        // Create drawing context and invoke user's draw function
        let mut ctx = DrawingContext {
            scene: &mut self.scene,
            width: frame.width as f32,
            height: frame.height as f32,
        };
        (self.draw_fn)(&mut ctx);

        // Render the scene
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
                &frame.view,
                &params,
            )
            .expect("Failed to render Vello scene");
    }
}
