//! Heatmap chart GPU renderer.
//!
//! Renders a 2D matrix as colored cells using a color scale.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::{wgpu, GpuContext, GpuFrame, GpuRenderer};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, HeatmapData};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::base::{
    create_storage_buffer, create_uniform_buffer, shader_with_common,
    write_storage_buffer, write_uniform_buffer,
};
use crate::renderer::ChartRenderer;

/// GPU-accelerated heatmap renderer.
///
/// Renders a grid of cells colored by value using the Viridis color scale.
pub struct HeatmapRenderer {
    // Data
    data: HeatmapData,
    bounds: DataBounds,

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    value_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl Default for HeatmapRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl HeatmapRenderer {
    /// Creates a new heatmap renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: HeatmapData::default(),
            bounds: DataBounds::default(),
            pipeline: None,
            uniform_buffer: None,
            value_buffer: None,
            bind_group: None,
            animation: ChartAnimation::default(),
            needs_redraw: false,
            zoom_pan: ZoomPanState::new(),
        }
    }

    /// Resets zoom and pan to default state.
    pub fn reset_zoom_pan(&mut self) {
        self.zoom_pan.reset();
    }

    /// Returns the current zoom scale.
    #[must_use]
    pub fn zoom_scale(&self) -> f32 {
        self.zoom_pan.scale
    }

    /// Sets the heatmap data directly.
    pub fn set_data(&mut self, data: HeatmapData) {
        self.bounds = DataBounds::new(0.0, data.cols as f32, 0.0, data.rows as f32);
        self.data = data;
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        };
        let shader_source = shader_with_common(include_str!("../shaders/heatmap.wgsl"));
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Heatmap Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Heatmap Bind Group Layout"),
                    entries: &[
                        // Uniforms
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // Values
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::VERTEX,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Heatmap Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Heatmap Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.surface_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: ctx.pipeline_cache,
            })
    }
}

impl GpuRenderer for HeatmapRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = HeatmapUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Heatmap Uniforms", &uniforms));

        // Create value buffer
        let initial_capacity = self.data.cell_count().max(64);
        let initial_values: Vec<f32> = if self.data.values.is_empty() {
            vec![0.0; initial_capacity]
        } else {
            self.data.values.clone()
        };

        self.value_buffer = Some(create_storage_buffer(ctx, "Heatmap Values", &initial_values));

        // Create bind group
        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Heatmap Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.value_buffer.as_ref().unwrap().as_entire_binding(),
                },
            ],
        }));

        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            return;
        };

        // Update zoom/pan state from gesture input
        self.zoom_pan
            .update(&frame.gesture, frame.width as f32, frame.height as f32);

        let cell_count = self.data.cell_count();

        if cell_count == 0 {
            // Clear to transparent
            let mut encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Heatmap Clear Encoder"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Heatmap Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                });
            }
            frame.queue.submit([encoder.finish()]);
            return;
        }

        // Update uniforms
        let uniforms = HeatmapUniforms {
            viewport: glam::Vec4::new(
                frame.width as f32,
                frame.height as f32,
                1.0 / frame.width as f32,
                1.0 / frame.height as f32,
            ),
            grid: glam::Vec4::new(
                self.data.rows as f32,
                self.data.cols as f32,
                self.data.min_value,
                self.data.max_value,
            ),
            animation: glam::Vec4::new(
                self.animation.time,
                self.animation.progress,
                self.animation.easing as f32,
                if self.animation.entry_active > 0 { 1.0 } else { 0.0 },
            ),
            pointer: if let Some((x, y)) = frame.pointer_normalized() {
                glam::Vec4::new(x, y, if frame.pointer.hit.is_some() { 1.0 } else { 0.0 }, 0.0)
            } else {
                glam::Vec4::new(-1.0, -1.0, 0.0, 0.0)
            },
            zoom_pan: glam::Vec4::new(self.zoom_pan.scale, self.zoom_pan.offset.x, self.zoom_pan.offset.y, 0.0),
        };
        write_uniform_buffer(frame.queue, self.uniform_buffer.as_ref().unwrap(), &uniforms);

        // Render heatmap
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Heatmap Encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Heatmap Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });

            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            // 6 vertices per cell (2 triangles)
            pass.draw(0..6, 0..cell_count as u32);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

/// Hit result value for heatmap (row, col, value).
#[derive(Debug, Clone, Copy, Default)]
pub struct HeatmapHit {
    /// Row index.
    pub row: u32,
    /// Column index.
    pub col: u32,
    /// Cell value.
    pub value: f32,
}

impl ChartRenderer for HeatmapRenderer {
    type Data = HeatmapData;
    type DataValue = HeatmapHit;

    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
        // Update data
        self.data = data.clone();
        self.bounds = DataBounds::new(0.0, data.cols as f32, 0.0, data.rows as f32);

        // Upload values to GPU
        if let Some(buffer) = &self.value_buffer {
            if !data.values.is_empty() {
                write_storage_buffer(queue, buffer, &data.values);
            }
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<HeatmapHit>> {
        if self.data.cell_count() == 0 {
            return None;
        }

        // Convert screen point to chart coordinates
        let chart_x = (point.x - viewport.x) / viewport.width;
        let chart_y = (point.y - viewport.y) / viewport.height;

        if chart_x < 0.0 || chart_x > 1.0 || chart_y < 0.0 || chart_y > 1.0 {
            return None;
        }

        // Calculate cell indices
        let col = (chart_x * self.data.cols as f32) as u32;
        let row = (chart_y * self.data.rows as f32) as u32;

        if row < self.data.rows && col < self.data.cols {
            let value = self.data.get(row, col).unwrap_or(0.0);
            return Some(HitResult {
                series: 0,
                index: (row * self.data.cols + col) as usize,
                value: HeatmapHit { row, col, value },
                screen_position: point,
            });
        }

        None
    }

    fn data_bounds(&self) -> DataBounds {
        self.bounds
    }

    fn data_count(&self) -> usize {
        self.data.cell_count()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// Heatmap-specific uniforms.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct HeatmapUniforms {
    /// Viewport: [width, height, 1/width, 1/height].
    viewport: glam::Vec4,
    /// Grid: [rows, cols, min_value, max_value].
    grid: glam::Vec4,
    /// Animation: [time, progress, easing, entry_active].
    animation: glam::Vec4,
    /// Pointer: [x, y, pressed, 0].
    pointer: glam::Vec4,
    /// Zoom/Pan: [scale, offset_x, offset_y, 0].
    zoom_pan: glam::Vec4,
}
