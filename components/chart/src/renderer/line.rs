//! Line chart GPU renderer.
//!
//! Renders line charts using instanced rendering where each segment is a quad.
//! Supports animation interpolation between data states and area fills.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, wgpu};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, DataPoint};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::ChartRenderer;
use crate::renderer::base::{
    ChartUniforms, MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment,
    multisample_state, shader_with_common, write_storage_buffer, write_uniform_buffer,
};

const PLOT_PADDING: f32 = 0.1;

/// GPU-accelerated line chart renderer.
///
/// Uses instanced rendering to draw line segments efficiently.
/// Supports smooth animation between data states via double-buffering.
pub struct LineChartRenderer {
    // Data
    data: Vec<DataPoint>,
    bounds: DataBounds,
    line_color: [f32; 4],
    line_width: f32,
    show_fill: bool,
    fill_opacity: f32,

    // GPU resources (initialized on setup)
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    current_buffer: Option<wgpu::Buffer>,
    previous_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,
    msaa_samples: u32,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl Default for LineChartRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl LineChartRenderer {
    /// Creates a new line chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bounds: DataBounds::default(),
            line_color: [0.231, 0.510, 0.965, 1.0], // Blue #3B82F6
            line_width: 2.0,
            show_fill: false,
            fill_opacity: 0.3,
            pipeline: None,
            uniform_buffer: None,
            current_buffer: None,
            previous_buffer: None,
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

    /// Sets the line color from sRGB values.
    pub fn set_color(&mut self, color: Srgb) {
        self.line_color = [color.red, color.green, color.blue, 1.0];
    }

    /// Sets the line width in pixels.
    pub fn set_line_width(&mut self, width: f32) {
        assert!(width.is_finite() && width > 0.0, "Line width must be > 0");
        self.line_width = width;
    }

    /// Enables area fill below the line.
    pub fn set_fill(&mut self, show: bool, opacity: f32) {
        assert!(
            opacity.is_finite() && (0.0..=1.0).contains(&opacity),
            "Fill opacity must be in [0.0, 1.0]"
        );
        self.show_fill = show;
        self.fill_opacity = opacity;
    }

    /// Sets the data directly (for initial setup before GPU is ready).
    pub fn set_data(&mut self, data: Vec<DataPoint>) {
        self.bounds = DataBounds::from_points(&data);
        self.data = data;
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(include_str!("../shaders/line.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Line Chart Shader",
            &shader_source,
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Line Chart Bind Group Layout"),
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
                        // Current data
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // Previous data (for animation)
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
                label: Some("Line Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Line Chart Pipeline"),
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

impl GpuRenderer for LineChartRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer with line-specific uniforms
        let uniforms = LineUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Line Chart Uniforms", &uniforms));

        // Create data buffers with initial capacity
        let initial_capacity = self.data.len().max(16384);
        let initial_data: Vec<GpuLinePoint> = self
            .data
            .iter()
            .map(|p| GpuLinePoint { x: p.x, y: p.y })
            .collect();

        self.current_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Line Chart Current Data",
                &vec![GpuLinePoint::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Line Chart Current Data", &initial_data)
        });

        self.previous_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Line Chart Previous Data",
                &vec![GpuLinePoint::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Line Chart Previous Data", &initial_data)
        });

        // Create bind group
        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Line Chart Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.current_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.previous_buffer.as_ref().unwrap().as_entire_binding(),
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

        if self.data.len() < 2 {
            // Need at least 2 points to draw a line - clear to transparent
            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Line Chart Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Line Chart Clear Pass"),
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

        // Update zoom/pan state from gesture input
        self.zoom_pan
            .update(&frame.gesture, frame.width as f32, frame.height as f32);

        // Transform bounds based on zoom/pan state
        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);

        // Update uniforms with transformed bounds
        let base_uniforms = LineUniforms {
            chart: ChartUniforms {
                viewport: glam::Vec4::new(
                    frame.width as f32,
                    frame.height as f32,
                    1.0 / frame.width as f32,
                    1.0 / frame.height as f32,
                ),
                bounds: glam::Vec4::new(
                    visible_bounds.min_x,
                    visible_bounds.max_x,
                    visible_bounds.min_y,
                    visible_bounds.max_y,
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
                        glam::Vec4::new(
                            px,
                            py,
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
                },
            },
            line_color: glam::Vec4::from_array(self.line_color),
            line_width: self.line_width,
            show_fill: if self.show_fill { 1.0 } else { 0.0 },
            fill_opacity: self.fill_opacity,
            point_count: self.data.len() as f32,
            render_mode: 0.0,
        };

        // Render line segments
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Line Chart Encoder"),
            });

        let segment_count = (self.data.len() - 1) as u32;
        if self.show_fill {
            let mut fill_uniforms = base_uniforms;
            fill_uniforms.render_mode = 1.0;
            write_uniform_buffer(
                frame.queue,
                self.uniform_buffer.as_ref().unwrap(),
                &fill_uniforms,
            );

            let (color_view, resolve_target) = msaa_attachment(
                &mut self.msaa_target,
                frame.device,
                frame.format,
                frame.width,
                frame.height,
                &frame.view,
                self.msaa_samples,
            );

            {
                let mut fill_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Line Chart Fill Pass"),
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

                fill_pass.set_pipeline(pipeline);
                fill_pass.set_bind_group(0, bind_group, &[]);
                fill_pass.draw(0..6, 0..segment_count);
            }

            let line_uniforms = base_uniforms;
            write_uniform_buffer(
                frame.queue,
                self.uniform_buffer.as_ref().unwrap(),
                &line_uniforms,
            );

            let (color_view, resolve_target) = msaa_attachment(
                &mut self.msaa_target,
                frame.device,
                frame.format,
                frame.width,
                frame.height,
                &frame.view,
                self.msaa_samples,
            );
            {
                let mut line_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Line Chart Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: color_view,
                        resolve_target,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    ..Default::default()
                });

                line_pass.set_pipeline(pipeline);
                line_pass.set_bind_group(0, bind_group, &[]);
                line_pass.draw(0..6, 0..segment_count);
            }
        } else {
            write_uniform_buffer(
                frame.queue,
                self.uniform_buffer.as_ref().unwrap(),
                &base_uniforms,
            );

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
                label: Some("Line Chart Render Pass"),
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
            pass.draw(0..6, 0..segment_count);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl ChartRenderer for LineChartRenderer {
    type Data = Vec<DataPoint>;
    type DataValue = DataPoint;

    fn update_data(&mut self, data: &Self::Data, _device: &wgpu::Device, queue: &wgpu::Queue) {
        // Swap buffers for animation
        core::mem::swap(&mut self.current_buffer, &mut self.previous_buffer);

        // Update data
        self.data = data.clone();
        self.bounds = DataBounds::from_points(&self.data).with_padding(0.1);

        // Upload to GPU
        if let Some(buffer) = &self.current_buffer {
            let gpu_data: Vec<GpuLinePoint> = data
                .iter()
                .map(|p| GpuLinePoint { x: p.x, y: p.y })
                .collect();
            write_storage_buffer(queue, buffer, &gpu_data);
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<DataPoint>> {
        if self.data.len() < 2 {
            return None;
        }

        let (chart_x, chart_y) = super::chart_coords_from_viewport(viewport, point, PLOT_PADDING)?;
        let chart_y = 1.0 - chart_y;

        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);
        if visible_bounds.width() <= 0.0 || visible_bounds.height() <= 0.0 {
            return None;
        }

        // Find closest point on the line
        let denom = (1.0 - 2.0 * PLOT_PADDING).max(0.001);
        let hit_radius = 10.0 / (viewport.width.min(viewport.height) * denom); // 10px hit area
        let mut closest_idx = None;
        let mut closest_dist = f32::MAX;

        for (i, data_point) in self.data.iter().enumerate() {
            let normalized_x = (data_point.x - visible_bounds.min_x)
                / (visible_bounds.max_x - visible_bounds.min_x);
            let normalized_y = (data_point.y - visible_bounds.min_y)
                / (visible_bounds.max_y - visible_bounds.min_y);

            let dx = chart_x - normalized_x;
            let dy = chart_y - normalized_y;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < closest_dist && dist < hit_radius {
                closest_dist = dist;
                closest_idx = Some(i);
            }
        }

        closest_idx.map(|i| HitResult {
            series: 0,
            index: i,
            value: self.data[i],
            screen_position: point,
        })
    }

    fn data_bounds(&self) -> DataBounds {
        self.bounds
    }

    fn data_count(&self) -> usize {
        self.data.len()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// GPU-friendly line point.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuLinePoint {
    x: f32,
    y: f32,
}

/// Line chart specific uniforms.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct LineUniforms {
    chart: ChartUniforms,
    line_color: glam::Vec4,
    line_width: f32,
    show_fill: f32,
    fill_opacity: f32,
    point_count: f32,
    render_mode: f32,
}
