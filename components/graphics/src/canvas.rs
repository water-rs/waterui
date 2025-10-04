use crate::{context::GraphicsContext, wgpu_view::WgpuView};
use std::cell::RefCell;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene, peniko::Color};
use waterui_core::{Environment, View};

// A type alias for the drawing closure that populates the GraphicsContext.
type DrawCallback = Box<dyn Fn(&mut GraphicsContext)>;

/// A high-level 2D vector graphics canvas view.
///
/// This component provides a user-friendly API for 2D drawing. Internally, it
/// uses Vello to render the graphics and leverages the primitive `WgpuView`
/// to display the result.
pub struct Canvas {
    /// The closure that performs the drawing operations.
    content: DrawCallback,
    /// The desired width of the canvas.
    width: f32,
    /// The desired height of the canvas,
    height: f32,
}

// Implement Debug manually because closures don't implement it.
impl std::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Canvas")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Canvas {
    /// Creates a new Canvas view with a drawing closure.
    pub fn new(content: impl Fn(&mut GraphicsContext) + 'static) -> Self {
        Self {
            content: Box::new(content),
            width: 100.0,  // Default width
            height: 100.0, // Default height
        }
    }

    /// Sets the canvas width.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the canvas height.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}

/// Creates a new `Canvas` view with the specified drawing closure.
pub fn canvas(content: impl Fn(&mut GraphicsContext) + 'static) -> Canvas {
    Canvas::new(content)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
impl View for Canvas {
    fn body(self, env: &Environment) -> impl View {
        // The Canvas view transforms itself into a primitive WgpuView.
        // It constructs the `on_draw` closure which contains all the Vello
        // rendering logic.

        // Use RefCell to allow the renderer to be created once and reused across frames.
        let renderer: RefCell<Option<Renderer>> = RefCell::new(None);
        let env = env.clone();

        WgpuView::new(move |device, queue, target_view, format| {
            // Build the Vello scene from the user's drawing commands.
            let mut scene = Scene::new();
            let mut context = GraphicsContext {
                scene: &mut scene,
                env: &env,
            };
            (self.content)(&mut context);

            // Lazily create the Vello renderer.
            let mut renderer_ref = renderer.borrow_mut();
            if renderer_ref.is_none() {
                let options = RendererOptions {
                    surface_format: Some(format),
                    use_cpu: false,
                    antialiasing_support: AaSupport::all(),
                    num_init_threads: None,
                };
                let renderer =
                    Renderer::new(device, options).expect("failed to create Vello renderer");
                *renderer_ref = Some(renderer);
            }

            // Render the scene to the target texture provided by the WgpuView.
            if let Some(renderer) = renderer_ref.as_mut() {
                let width = self.width.max(1.0).round() as u32;
                let height = self.height.max(1.0).round() as u32;
                let render_params = RenderParams {
                    base_color: Color::WHITE,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                };
                renderer
                    .render_to_texture(device, queue, &scene, target_view, &render_params)
                    .expect("failed to render Vello scene");
            }
        })
        .width(self.width)
        .height(self.height)
    }
}
