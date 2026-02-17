//! Contour chart GPU renderer.
//!
//! Renders isolines using the marching squares algorithm on GPU.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, wgpu};

use crate::animation::ChartAnimation;
use crate::data::{ContourData, DataBounds};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::ChartRenderer;
use crate::renderer::base::{
    MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer, write_uniform_buffer,
};

const PLOT_PADDING: f32 = 0.05;

/// GPU-accelerated contour chart renderer.
///
/// Renders isolines from a 2D scalar field using the marching squares algorithm.
/// Each contour level is drawn with a different color from the Viridis scale.
pub struct ContourRenderer {
    // Data
    data: ContourData,
    bounds: DataBounds,
    line_width: f32,

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    value_buffer: Option<wgpu::Buffer>,
    level_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,
    msaa_samples: u32,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl Default for ContourRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl ContourRenderer {
    /// Creates a new contour renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: ContourData::default(),
            bounds: DataBounds::default(),
            line_width: 2.0,
            pipeline: None,
            uniform_buffer: None,
            value_buffer: None,
            level_buffer: None,
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

    /// Sets the line width for contour lines.
    #[must_use]
    pub fn line_width(mut self, width: f32) -> Self {
        assert!(
            width.is_finite() && width > 0.0,
            "Contour line width must be > 0"
        );
        self.line_width = width;
        self
    }

    /// Sets the contour data directly.
    pub fn set_data(&mut self, data: ContourData) {
        self.bounds = DataBounds::new(0.0, data.cols as f32, 0.0, data.rows as f32);
        self.data = data;
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(include_str!("../shaders/contour.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Contour Shader",
            &shader_source,
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Contour Bind Group Layout"),
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
                        // Values (scalar field)
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
                        // Levels (contour thresholds)
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
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
                label: Some("Contour Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Contour Pipeline"),
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
}

impl GpuRenderer for ContourRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = ContourUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Contour Uniforms", &uniforms));

        // Create value buffer (scalar field)
        let initial_capacity = self.data.cell_count().max(16384);
        let initial_values: Vec<f32> = if self.data.values.is_empty() {
            vec![0.0; initial_capacity]
        } else {
            self.data.values.clone()
        };
        self.value_buffer = Some(create_storage_buffer(
            ctx,
            "Contour Values",
            &initial_values,
        ));

        // Create level buffer (contour thresholds)
        let initial_levels: Vec<f32> = if self.data.levels.is_empty() {
            vec![0.5]
        } else {
            self.data.levels.clone()
        };
        self.level_buffer = Some(create_storage_buffer(
            ctx,
            "Contour Levels",
            &initial_levels,
        ));

        // Create bind group
        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Contour Bind Group"),
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.level_buffer.as_ref().unwrap().as_entire_binding(),
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

        // Need at least 2x2 grid and some levels
        if self.data.rows < 2 || self.data.cols < 2 || self.data.levels.is_empty() {
            // Clear to transparent
            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Contour Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Contour Clear Pass"),
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
        let uniforms = ContourUniforms {
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
            style: glam::Vec4::new(self.line_width, self.data.levels.len() as f32, 0.0, 0.0),
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

        // Calculate instance count:
        // instances = num_levels * cells_per_level
        // cells_per_level = (rows - 1) * (cols - 1)
        let cells_per_level = (self.data.rows - 1) * (self.data.cols - 1);
        let instance_count = (self.data.levels.len() as u32) * cells_per_level;

        // Render contours
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Contour Encoder"),
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
                label: Some("Contour Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color_view,
                    resolve_target,
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
            // 12 vertices per instance max (2 line segments × 6 vertices each)
            pass.draw(0..12, 0..instance_count);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

/// Hit result value for contour chart (level index, value at point).
#[derive(Debug, Clone, Copy, Default)]
pub struct ContourHit {
    /// Contour level index.
    pub level: usize,
    /// Contour threshold value.
    pub threshold: f32,
}

impl ChartRenderer for ContourRenderer {
    type Data = ContourData;
    type DataValue = ContourHit;

    fn update_data(&mut self, data: &Self::Data, _device: &wgpu::Device, queue: &wgpu::Queue) {
        // Update data
        self.data = data.clone();
        self.bounds = DataBounds::new(0.0, data.cols as f32, 0.0, data.rows as f32);

        // Upload values to GPU
        if let Some(buffer) = &self.value_buffer {
            if !data.values.is_empty() {
                write_storage_buffer(queue, buffer, &data.values);
            }
        }

        // Upload levels to GPU
        if let Some(buffer) = &self.level_buffer {
            if !data.levels.is_empty() {
                write_storage_buffer(queue, buffer, &data.levels);
            }
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<ContourHit>> {
        if self.data.levels.is_empty() || self.data.rows < 2 || self.data.cols < 2 {
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

        // Find the value at this point by bilinear interpolation
        let fx = unzoomed_x * (self.data.cols - 1) as f32;
        let fy = unzoomed_y * (self.data.rows - 1) as f32;
        let col = (fx as u32).min(self.data.cols - 2);
        let row = (fy as u32).min(self.data.rows - 2);
        let tx = fx - col as f32;
        let ty = fy - row as f32;

        // Get corner values
        let v00 = self.data.get(row, col).unwrap_or(0.0);
        let v10 = self.data.get(row, col + 1).unwrap_or(0.0);
        let v01 = self.data.get(row + 1, col).unwrap_or(0.0);
        let v11 = self.data.get(row + 1, col + 1).unwrap_or(0.0);

        // Bilinear interpolation
        let value = v00 * (1.0 - tx) * (1.0 - ty)
            + v10 * tx * (1.0 - ty)
            + v01 * (1.0 - tx) * ty
            + v11 * tx * ty;

        // Find closest contour level
        let mut closest_level = 0;
        let mut closest_dist = f32::MAX;
        for (i, &level) in self.data.levels.iter().enumerate() {
            let dist = (value - level).abs();
            if dist < closest_dist {
                closest_dist = dist;
                closest_level = i;
            }
        }

        Some(HitResult {
            series: 0,
            index: closest_level,
            value: ContourHit {
                level: closest_level,
                threshold: self.data.levels[closest_level],
            },
            screen_position: point,
        })
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

/// Contour-specific uniforms.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct ContourUniforms {
    /// Viewport: [width, height, 1/width, 1/height].
    viewport: glam::Vec4,
    /// Grid: [rows, cols, min_value, max_value].
    grid: glam::Vec4,
    /// Animation: [time, progress, easing, entry_active].
    animation: glam::Vec4,
    /// Style: [line_width, num_levels, 0, 0].
    style: glam::Vec4,
    /// Zoom/Pan: [scale, offset_x, offset_y, 0].
    zoom_pan: glam::Vec4,
}
