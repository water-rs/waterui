#[cfg(feature = "wgpu")]
use alloc::sync::Arc;
#[cfg(feature = "wgpu")]
use core::cell::RefCell;

#[cfg(feature = "wgpu")]
use waterui_core::{Environment, View};

#[cfg(feature = "wgpu")]
use crate::{
    RendererWgpuSurface,
    renderer_view::{RendererSurface, RendererView},
};

#[cfg(feature = "wgpu")]
#[derive(Clone)]
/// A view that renders WGSL shader content via WGPU.
pub struct Shader {
    source: Arc<str>,
    vertex_entry: Arc<str>,
    fragment_entry: Arc<str>,
    width: f32,
    height: f32,
}

#[cfg(feature = "wgpu")]
impl core::fmt::Debug for Shader {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Shader")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("vertex_entry", &self.vertex_entry)
            .field("fragment_entry", &self.fragment_entry)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "wgpu")]
impl Shader {
    /// Creates a shader from WGSL source code. The shader must contain the entry points
    /// `vs_main` and `fs_main` by default. Use [`vertex_entry`](Self::vertex_entry) and
    /// [`fragment_entry`](Self::fragment_entry) to customise them.
    pub fn from_wgsl(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            vertex_entry: Arc::from("vs_main"),
            fragment_entry: Arc::from("fs_main"),
            width: 100.0,
            height: 100.0,
        }
    }

    /// Overrides the vertex entry point name.
    #[must_use]
    pub fn vertex_entry(mut self, entry: impl Into<Arc<str>>) -> Self {
        self.vertex_entry = entry.into();
        self
    }

    /// Overrides the fragment entry point name.
    #[must_use]
    pub fn fragment_entry(mut self, entry: impl Into<Arc<str>>) -> Self {
        self.fragment_entry = entry.into();
        self
    }

    /// Sets the shader view width.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the shader view height.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}

#[cfg(feature = "wgpu")]
impl View for Shader {
    fn body(self, _env: &Environment) -> impl View {
        let Self {
            source,
            vertex_entry,
            fragment_entry,
            width,
            height,
        } = self;

        let pipeline_cache = RefCell::new(None::<PipelineState>);

        RendererView::new(move |surface| match surface {
            RendererSurface::Cpu(mut cpu_surface) => {
                cpu_surface.pixels_mut().fill(0);
            }
            RendererSurface::Wgpu(gpu_surface) => {
                render_shader(
                    &gpu_surface,
                    &source,
                    &vertex_entry,
                    &fragment_entry,
                    &pipeline_cache,
                );
            }
        })
        .width(width)
        .height(height)
    }
}

#[cfg(feature = "wgpu")]
struct PipelineState {
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
}

#[cfg(feature = "wgpu")]
fn render_shader(
    surface: &RendererWgpuSurface<'_>,
    source: &Arc<str>,
    vertex_entry: &Arc<str>,
    fragment_entry: &Arc<str>,
    cache: &RefCell<Option<PipelineState>>,
) {
    let mut cache_mut = cache.borrow_mut();
    if cache_mut
        .as_ref()
        .is_none_or(|state| state.format != surface.format)
    {
        let pipeline = build_pipeline(
            surface.device,
            surface.format,
            source,
            vertex_entry,
            fragment_entry,
        );
        *cache_mut = Some(PipelineState {
            pipeline,
            format: surface.format,
        });
    }

    let Some(state) = cache_mut.as_ref() else {
        return;
    };

    let mut encoder = surface
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("waterui-shader-encoder"),
        });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("waterui-shader-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface.target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&state.pipeline);
        pass.draw(0..3, 0..1);
    }
    surface.queue.submit(Some(encoder.finish()));
}

#[cfg(feature = "wgpu")]
fn build_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    source: &Arc<str>,
    vertex_entry: &Arc<str>,
    fragment_entry: &Arc<str>,
) -> wgpu::RenderPipeline {
    let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("waterui-shader-module"),
        source: wgpu::ShaderSource::Wgsl(source.as_ref().into()),
    });

    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("waterui-shader-layout"),
        bind_group_layouts: &[],
        push_constant_ranges: &[],
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("waterui-shader-pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader_module,
            entry_point: Some(vertex_entry.as_ref()),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &shader_module,
            entry_point: Some(fragment_entry.as_ref()),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        multiview: None,
        cache: None,
    })
}

#[cfg(not(feature = "wgpu"))]
#[derive(Debug, Clone, Copy)]
pub struct Shader;

#[cfg(not(feature = "wgpu"))]
impl Shader {
    #[allow(unused_variables)]
    pub fn from_wgsl<T>(source: T) -> Self {
        panic!("Enable the `wgpu` feature on `waterui-graphics` to use Shader views.");
    }
}
