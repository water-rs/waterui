//! Shader registration and retained shader-paint textures.

use std::borrow::Cow;

use rustc_hash::FxHashMap;

use cherenkov::{RenderError, ResourceError, ShaderSource};
use std::sync::Arc;

/// One shader use; uniforms are hashed by their exact IEEE representation.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Key {
    pub shader: u64,
    pub uniforms: Vec<u32>,
    pub size: (u32, u32),
}

struct Entry {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    animated: bool,
}

impl Entry {
    /// Creates the module and pipeline of a user shader paint. Errors are
    /// reported through the caller's validation scope.
    fn new(device: &wgpu::Device, source: &ShaderSource) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shader paint"),
            source: wgpu::ShaderSource::Wgsl(Cow::Owned(module_text(source))),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader paint"),
            entries: &[uniform_binding(0, 16), uniform_binding(1, 256)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shader paint"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shader paint"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(FRAGMENT_ENTRY),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: super::TARGET_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            layout,
            animated: source.animated,
        }
    }
}

/// The fragment entry point a user shader paint defines.
const FRAGMENT_ENTRY: &str = "main";

/// The module a user shader paint compiles to: the colour and paint
/// preludes, then the user's fragment source.
fn module_text(source: &ShaderSource) -> String {
    format!(
        "{}\n{}\n{}",
        include_str!("color.wgsl"),
        include_str!("paint.wgsl"),
        source.source
    )
}

/// Validates a user shader paint on the caller thread: the composed module
/// parses, passes naga validation and defines the fragment entry point.
pub fn validate(source: &ShaderSource) -> Result<(), ResourceError> {
    let text = module_text(source);
    let module = super::shaders::validate_wgsl(&text)?;
    if module
        .entry_points
        .iter()
        .any(|entry| entry.stage == naga::ShaderStage::Fragment && entry.name == FRAGMENT_ENTRY)
    {
        Ok(())
    } else {
        Err(ResourceError::Shader(format!(
            "the shader paint does not define its fragment entry point `{FRAGMENT_ENTRY}`"
        )))
    }
}

pub struct Texture {
    pub image: super::GpuImage,
    globals: wgpu::Buffer,
    parameters: wgpu::Buffer,
    bindings: wgpu::BindGroup,
    rendered: bool,
}

#[derive(Default)]
pub struct Registry {
    entries: FxHashMap<u64, Entry>,
    registered: bool,
}

impl Registry {
    pub const fn has_registrations(&self) -> bool {
        self.registered
    }
    /// Registers shader `id` from a source [`validate`] accepted. The
    /// validation scope covers module and pipeline creation.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn add(
        &mut self,
        device: &wgpu::Device,
        id: u64,
        source: &ShaderSource,
    ) -> Result<(), ResourceError> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entry = Entry::new(device, source);
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(ResourceError::Shader(error.to_string()));
        }
        self.insert(id, entry);
        Ok(())
    }

    /// The browser variant of [`Registry::add`], awaiting the validation
    /// scope without blocking the JS event loop.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn add(
        &mut self,
        device: &wgpu::Device,
        id: u64,
        source: &ShaderSource,
    ) -> Result<(), ResourceError> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entry = Entry::new(device, source);
        if let Some(error) = scope.pop().await {
            return Err(ResourceError::Shader(error.to_string()));
        }
        self.insert(id, entry);
        Ok(())
    }

    fn insert(&mut self, id: u64, entry: Entry) {
        self.registered = true;
        self.entries.insert(id, entry);
    }

    pub fn remove(&mut self, id: u64) {
        self.entries.remove(&id);
    }

    pub fn animated(&self, key: &Key) -> bool {
        self.entries
            .get(&key.shader)
            .is_some_and(|pipeline| pipeline.animated)
    }

    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: &Arc<Key>,
        textures: &mut FxHashMap<Arc<Key>, Texture>,
        time: f32,
    ) -> Result<(), RenderError> {
        let pipeline = self
            .entries
            .get(&key.shader)
            .ok_or_else(|| RenderError::Render(format!("unregistered shader {}", key.shader)))?;
        if key.uniforms.len() > 64 {
            return Err(RenderError::Render(
                "shader paint accepts at most 64 uniform floats".into(),
            ));
        }
        let maximum = device.limits().max_texture_dimension_2d;
        if key.size.0 > maximum || key.size.1 > maximum {
            return Err(RenderError::Render(format!(
                "shader texture {:?} exceeds device limit {maximum}",
                key.size
            )));
        }
        let texture = textures.entry(Arc::clone(key)).or_insert_with(|| {
            let (texture, view) = super::create_target(
                device,
                "shader paint",
                key.size,
                super::TARGET_USAGES,
                super::TARGET_FORMAT,
            );
            crate::diag::create(
                device,
                "shader paint",
                u64::from(key.size.0)
                    * u64::from(key.size.1)
                    * super::texel_bytes(super::TARGET_FORMAT),
            );
            let globals = uniform_buffer(device, 16);
            let parameters = uniform_buffer(device, 256);
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shader paint"),
                layout: &pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: globals.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: parameters.as_entire_binding(),
                    },
                ],
            });
            Texture {
                image: super::GpuImage {
                    texture,
                    view,
                    width: key.size.0,
                    height: key.size.1,
                },
                globals,
                parameters,
                bindings,
                rendered: false,
            }
        });
        if texture.rendered && !pipeline.animated {
            return Ok(());
        }
        #[expect(clippy::cast_precision_loss, reason = "GPU texture dimensions fit f32")]
        let globals = [time, 0.0, key.size.0 as f32, key.size.1 as f32];
        queue.write_buffer(&texture.globals, 0, bytemuck::cast_slice(&globals));
        let mut parameters = [0u32; 64];
        parameters[..key.uniforms.len()].copy_from_slice(&key.uniforms);
        queue.write_buffer(&texture.parameters, 0, bytemuck::cast_slice(&parameters));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("shader paint"),
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture.image.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&pipeline.pipeline);
        pass.set_bind_group(0, &texture.bindings, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        queue.submit([encoder.finish()]);
        texture.rendered = true;
        Ok(())
    }
}

const fn uniform_binding(binding: u32, size: u64) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    }
}

fn uniform_buffer(device: &wgpu::Device, size: u64) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shader paint uniforms"),
        size,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    crate::diag::create(device, "shader paint uniforms", size);
    buffer
}
