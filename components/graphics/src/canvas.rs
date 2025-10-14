use alloc::sync::Arc;

use tiny_skia::{Color as TinyColor, FillRule, Paint, PathBuilder, PixmapMut, Stroke, Transform};
use waterui_color::ResolvedColor;
use waterui_core::{Environment, Signal, View};

use crate::{
    context::{GraphicsContext, RecordedCommand},
    renderer_view::{RendererBufferFormat, RendererCpuSurface, RendererSurface, RendererView},
    shape::{DrawStyle, Path, PathCommand},
};

/// A high-level 2D vector graphics canvas view.
///
/// This component records drawing commands into a [`GraphicsContext`] and renders them
/// into a [`RendererView`]'s CPU surface using `tiny-skia`. It is entirely independent of
/// any GPU technology, allowing backends to provide either CPU or GPU surfaces via the
/// unified renderer bridge.
pub struct Canvas {
    /// The closure that performs the drawing operations.
    content: Arc<dyn Fn(&mut GraphicsContext) + Send + Sync>,
    /// The desired width of the canvas.
    width: f32,
    /// The desired height of the canvas.
    height: f32,
}

impl core::fmt::Debug for Canvas {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Canvas")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl Canvas {
    /// Creates a new Canvas view with a drawing closure.
    pub fn new(content: impl Fn(&mut GraphicsContext) + Send + Sync + 'static) -> Self {
        Self {
            content: Arc::new(content),
            width: 100.0,
            height: 100.0,
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
pub fn canvas(content: impl Fn(&mut GraphicsContext) + Send + Sync + 'static) -> Canvas {
    Canvas::new(content)
}

impl View for Canvas {
    fn body(self, env: &Environment) -> impl View {
        let Self {
            content,
            width,
            height,
        } = self;
        let env = env.clone();

        RendererView::new(move |surface| {
            let mut context = GraphicsContext::new(&env);
            content.as_ref()(&mut context);
            let commands = context.into_commands();

            match surface {
                RendererSurface::Cpu(mut cpu_surface) => {
                    render_cpu(&mut cpu_surface, &env, &commands);
                }
                #[cfg(feature = "wgpu")]
                RendererSurface::Wgpu(mut gpu_surface) => {
                    // Canvas is a CPU renderer. If the backend only provides a GPU surface
                    // we simply clear it to transparent so the caller receives predictable
                    // output.
                    clear_gpu_surface(&mut gpu_surface);
                }
            }
        })
        .width(width)
        .height(height)
    }
}

fn render_cpu(
    surface: &mut RendererCpuSurface<'_>,
    env: &Environment,
    commands: &[RecordedCommand],
) {
    if surface.format != RendererBufferFormat::Rgba8888 {
        return;
    }

    if !surface.is_tightly_packed() {
        return;
    }

    if surface.width == 0 || surface.height == 0 {
        return;
    }

    let width = surface.width;
    let height = surface.height;
    if let Some(mut pixmap) = PixmapMut::from_bytes(surface.pixels_mut(), width, height) {
        pixmap.fill(TinyColor::TRANSPARENT);

        for command in commands {
            let Some(path) = build_path(&command.path) else {
                continue;
            };

            match &command.style {
                DrawStyle::Fill(color) => {
                    let mut paint = Paint::default();
                    paint.set_color(resolve_color(env, color));
                    pixmap.fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
                DrawStyle::Stroke(color, width) => {
                    let mut paint = Paint::default();
                    paint.set_color(resolve_color(env, color));
                    let mut stroke = Stroke::default();
                    stroke.width = *width;
                    pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
                }
            }
        }
    }
}

#[cfg(feature = "wgpu")]
fn clear_gpu_surface(surface: &mut crate::renderer_view::RendererWgpuSurface<'_>) {
    use wgpu::LoadOp;

    let mut encoder = surface
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui-canvas-gpu-clear"),
        });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("waterui-canvas-gpu-clear-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface.target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: true,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    surface.queue.submit(Some(encoder.finish()));
}

fn build_path(path: &Path) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();

    for command in &path.0 {
        match *command {
            PathCommand::MoveTo([x, y]) => builder.move_to(x, y),
            PathCommand::LineTo([x, y]) => builder.line_to(x, y),
            PathCommand::QuadTo([cx, cy], [x, y]) => builder.quad_to(cx, cy, x, y),
            PathCommand::CurveTo([cx1, cy1], [cx2, cy2], [x, y]) => {
                builder.cubic_to(cx1, cy1, cx2, cy2, x, y);
            }
            PathCommand::Close => builder.close(),
        }
    }

    builder.finish()
}

fn resolve_color(env: &Environment, color: &waterui_color::Color) -> TinyColor {
    let resolved = color.resolve(env).get();
    resolved_to_tiny(&resolved)
}

fn resolved_to_tiny(resolved: &ResolvedColor) -> TinyColor {
    let srgb = resolved.to_srgb();
    TinyColor::from_rgba(
        srgb.red.clamp(0.0, 1.0),
        srgb.green.clamp(0.0, 1.0),
        srgb.blue.clamp(0.0, 1.0),
        resolved.opacity.clamp(0.0, 1.0),
    )
    .unwrap_or(TinyColor::TRANSPARENT)
}
