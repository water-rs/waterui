//! Heatmap chart GPU renderer.
//!
//! Renders a 2D matrix as colored cells using a color scale.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::{GpuContext, GpuFrame, GpuView, wgpu};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, HeatmapData};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::ChartRenderer;
use crate::renderer::base::{
    MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer_with_growth, write_uniform_buffer,
};

const PLOT_PADDING: f32 = 0.05;

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
    msaa_target: Option<MsaaTarget>,
    msaa_samples: u32,

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
            msaa_target: None,
            msaa_samples: 1,
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
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(include_str!("../shaders/heatmap.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Heatmap Shader",
            &shader_source,
        );

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
                    module: shader.as_ref(),
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader.as_ref(),
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
                multisample: multisample_state(ctx.msaa_samples),
                multiview: None,
                cache: ctx.pipeline_cache,
            })
    }

    fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };
        let Some(value_buffer) = &self.value_buffer else {
            return;
        };

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Heatmap Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: value_buffer.as_entire_binding(),
                },
            ],
        }));
    }
}

impl GpuView for HeatmapRenderer {
    fn setup(&mut self, ctx: &GpuContext, _env: &mut waterui_core::Environment) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = HeatmapUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Heatmap Uniforms", &uniforms));

        // Create value buffer
        let initial_capacity = self.data.cell_count().max(16384);
        let initial_values: Vec<f32> = if self.data.values.is_empty() {
            vec![0.0; initial_capacity]
        } else {
            self.data.values.clone()
        };

        self.value_buffer = Some(create_storage_buffer(
            ctx,
            "Heatmap Values",
            &initial_values,
        ));

        self.rebuild_bind_group(ctx.device);

        async {}
    }

    fn render(&mut self, frame: &mut GpuFrame) {
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
            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Heatmap Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Heatmap Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &frame.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
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
                if self.animation.entry_active > 0 {
                    1.0
                } else {
                    0.0
                },
            ),
            pointer: if let Some((x, y)) = frame.pointer_normalized() {
                if let Some((px, py)) = super::unpad_normalized_point(x, y, PLOT_PADDING) {
                    let zoomed_y = 1.0 - py;
                    let zoomed_x = px;
                    let scale = self.zoom_pan.scale.max(0.001);
                    let unzoomed_x = (zoomed_x - 0.5 - self.zoom_pan.offset.x) / scale + 0.5;
                    let unzoomed_y = (zoomed_y - 0.5 - self.zoom_pan.offset.y) / scale + 0.5;
                    if (0.0..=1.0).contains(&unzoomed_x) && (0.0..=1.0).contains(&unzoomed_y) {
                        let padded_x = PLOT_PADDING + unzoomed_x * (1.0 - 2.0 * PLOT_PADDING);
                        let padded_y = PLOT_PADDING + unzoomed_y * (1.0 - 2.0 * PLOT_PADDING);
                        glam::Vec4::new(
                            padded_x,
                            padded_y,
                            if frame.pointer.hit.is_some() {
                                1.0
                            } else {
                                0.0
                            },
                            0.0,
                        )
                    } else {
                        glam::Vec4::new(-1.0, -1.0, 0.0, 0.0)
                    }
                } else {
                    glam::Vec4::new(-1.0, -1.0, 0.0, 0.0)
                }
            } else {
                glam::Vec4::new(-1.0, -1.0, 0.0, 0.0)
            },
            zoom_pan: glam::Vec4::new(
                self.zoom_pan.scale,
                self.zoom_pan.offset.x,
                self.zoom_pan.offset.y,
                0.0,
            ),
        };
        write_uniform_buffer(
            frame.queue,
            self.uniform_buffer.as_ref().unwrap(),
            &uniforms,
        );

        // Render heatmap
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Heatmap Encoder"),
            });

        {
            let (color_view, resolve_target) = msaa_attachment(
                &mut self.msaa_target,
                frame.device,
                frame.format,
                frame.width,
                frame.height,
                &frame.view,
                self.msaa_samples,
            );

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Heatmap Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    depth_slice: None,
                    resolve_target,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
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

    fn update_data(&mut self, data: &Self::Data, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Update data
        self.data = data.clone();
        self.bounds = DataBounds::new(0.0, data.cols as f32, 0.0, data.rows as f32);

        // Upload values to GPU
        let mut needs_rebind = false;
        if let Some(buffer) = self.value_buffer.as_mut() {
            if !data.values.is_empty() {
                needs_rebind |= write_storage_buffer_with_growth(
                    device,
                    queue,
                    buffer,
                    "Heatmap Values",
                    &data.values,
                );
            }
        }
        if needs_rebind {
            self.rebuild_bind_group(device);
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

        let (chart_x, chart_y) = super::chart_coords_from_viewport(viewport, point, PLOT_PADDING)?;
        let chart_y = 1.0 - chart_y;

        let scale = self.zoom_pan.scale.max(0.001);
        let unzoomed_x = (chart_x - 0.5 - self.zoom_pan.offset.x) / scale + 0.5;
        let unzoomed_y = (chart_y - 0.5 - self.zoom_pan.offset.y) / scale + 0.5;

        if !(0.0..=1.0).contains(&unzoomed_x) || !(0.0..=1.0).contains(&unzoomed_y) {
            return None;
        }

        // Calculate cell indices
        let col = (unzoomed_x * self.data.cols as f32) as u32;
        let row = (unzoomed_y * self.data.rows as f32) as u32;

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
