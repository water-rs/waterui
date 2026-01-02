//! Stacked area chart GPU renderer.
//!
//! Renders stacked area charts using instanced rendering where each segment
//! is a quad spanning two adjacent X values.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::{wgpu, GpuContext, GpuFrame, GpuRenderer};

use crate::animation::ChartAnimation;
use crate::data::{AreaData, AreaSeries, DataBounds};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::base::{
    create_storage_buffer, create_uniform_buffer, shader_with_common, write_storage_buffer,
    write_uniform_buffer, ChartUniforms,
};
use crate::renderer::ChartRenderer;

/// GPU-accelerated stacked area chart renderer.
///
/// Renders multiple data series as stacked or overlaid filled areas.
/// Each series is rendered as quads between adjacent X values.
pub struct AreaRenderer {
    // Data
    data: AreaData,
    bounds: DataBounds,

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    segment_buffer: Option<wgpu::Buffer>,
    color_buffer: Option<wgpu::Buffer>,
    prev_segment_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl Default for AreaRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl AreaRenderer {
    /// Creates a new area chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: AreaData::default(),
            bounds: DataBounds::default(),
            pipeline: None,
            uniform_buffer: None,
            segment_buffer: None,
            color_buffer: None,
            prev_segment_buffer: None,
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

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        };
        let shader_source = shader_with_common(include_str!("../shaders/area.wgsl"));
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Area Chart Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Area Chart Bind Group Layout"),
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
                        // Current segments
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
                        // Colors
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
                        // Previous segments (for animation)
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
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
                label: Some("Area Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Area Chart Pipeline"),
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

    /// Converts area data to GPU segment format.
    fn to_gpu_segments(&self) -> Vec<GpuAreaSegment> {
        let point_count = self.data.point_count();
        let series_count = self.data.series_count();

        if point_count < 2 || series_count == 0 {
            return vec![GpuAreaSegment::default()];
        }

        let mut segments = Vec::with_capacity((point_count - 1) * series_count);

        // Compute cumulative values for stacking
        let mut cumulative: Vec<Vec<f32>> = vec![vec![0.0; point_count]; series_count + 1];

        for (si, series) in self.data.series.iter().enumerate() {
            for (i, &val) in series.values.iter().enumerate().take(point_count) {
                cumulative[si + 1][i] = if self.data.stacked {
                    cumulative[si][i] + val
                } else {
                    val
                };
            }
        }

        // Generate segments for each series
        for (si, _series) in self.data.series.iter().enumerate() {
            for i in 0..(point_count - 1) {
                let x0 = self.data.x_values[i];
                let x1 = self.data.x_values[i + 1];

                let y0_bottom = if self.data.stacked { cumulative[si][i] } else { 0.0 };
                let y0_top = cumulative[si + 1][i];
                let y1_bottom = if self.data.stacked { cumulative[si][i + 1] } else { 0.0 };
                let y1_top = cumulative[si + 1][i + 1];

                segments.push(GpuAreaSegment {
                    x0,
                    y0_top,
                    y0_bottom,
                    series_index: si as f32,
                    x1,
                    y1_top,
                    y1_bottom,
                    _pad: 0.0,
                });
            }
        }

        if segments.is_empty() {
            segments.push(GpuAreaSegment::default());
        }

        segments
    }

    /// Extracts colors from series data.
    fn get_colors(&self) -> Vec<glam::Vec4> {
        if self.data.series.is_empty() {
            return vec![glam::Vec4::new(0.23, 0.51, 0.96, 0.7)];
        }

        self.data
            .series
            .iter()
            .map(|s| glam::Vec4::new(s.color[0], s.color[1], s.color[2], s.color[3]))
            .collect()
    }

    /// Returns the total number of segments to render.
    fn segment_count(&self) -> usize {
        let point_count = self.data.point_count();
        let series_count = self.data.series_count();
        if point_count < 2 || series_count == 0 {
            0
        } else {
            (point_count - 1) * series_count
        }
    }
}

impl GpuRenderer for AreaRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        self.pipeline = Some(Self::create_pipeline(ctx));

        let uniforms = AreaUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Area Chart Uniforms", &uniforms));

        let initial_segments = self.to_gpu_segments();
        self.segment_buffer = Some(create_storage_buffer(
            ctx,
            "Area Chart Segments",
            &initial_segments,
        ));
        self.prev_segment_buffer = Some(create_storage_buffer(
            ctx,
            "Area Chart Prev Segments",
            &initial_segments,
        ));

        let colors = self.get_colors();
        self.color_buffer = Some(create_storage_buffer(ctx, "Area Chart Colors", &colors));

        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Area Chart Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.segment_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.color_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.prev_segment_buffer.as_ref().unwrap().as_entire_binding(),
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

        let segment_count = self.segment_count();

        if segment_count == 0 {
            // Clear to transparent
            let mut encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Area Chart Clear Encoder"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Area Chart Clear Pass"),
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

        // Transform bounds for zoom/pan
        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);

        // Update uniforms
        let uniforms = AreaUniforms {
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
                    glam::Vec4::new(
                        x,
                        y,
                        if frame.pointer.hit.is_some() { 1.0 } else { 0.0 },
                        0.0,
                    )
                } else {
                    glam::Vec4::new(-1.0, -1.0, 0.0, 0.0)
                },
            },
            point_count: self.data.point_count() as u32,
            series_count: self.data.series_count() as u32,
            stacked: if self.data.stacked { 1 } else { 0 },
            _pad: 0,
        };
        write_uniform_buffer(frame.queue, self.uniform_buffer.as_ref().unwrap(), &uniforms);

        // Render
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Area Chart Encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Area Chart Render Pass"),
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
            // 6 vertices per segment (2 triangles forming a quad)
            pass.draw(0..6, 0..segment_count as u32);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl ChartRenderer for AreaRenderer {
    type Data = AreaData;
    type DataValue = AreaSeries;

    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
        // Swap buffers for animation
        core::mem::swap(&mut self.segment_buffer, &mut self.prev_segment_buffer);

        self.data = data.clone();
        self.bounds = data.bounds().with_padding(0.1);

        // Upload segments
        if let Some(buffer) = &self.segment_buffer {
            let segments = self.to_gpu_segments();
            write_storage_buffer(queue, buffer, &segments);
        }

        // Upload colors
        if let Some(buffer) = &self.color_buffer {
            let colors = self.get_colors();
            write_storage_buffer(queue, buffer, &colors);
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<AreaSeries>> {
        if self.data.series.is_empty() || self.data.x_values.is_empty() {
            return None;
        }

        let padding = 0.1;
        let chart_x = (point.x - viewport.x) / viewport.width;
        let chart_y = (point.y - viewport.y) / viewport.height;

        if chart_x < 0.0 || chart_x > 1.0 || chart_y < 0.0 || chart_y > 1.0 {
            return None;
        }

        // Convert to data coordinates
        let data_x =
            self.bounds.min_x + (chart_x - padding) / (1.0 - 2.0 * padding) * self.bounds.width();
        let data_y =
            self.bounds.min_y + (chart_y - padding) / (1.0 - 2.0 * padding) * self.bounds.height();

        // Find which X index we're at
        let x_idx = self
            .data
            .x_values
            .iter()
            .position(|&x| x >= data_x)
            .unwrap_or(self.data.x_values.len() - 1);

        // Compute cumulative values at this X
        let mut cumulative = 0.0;
        for (si, series) in self.data.series.iter().enumerate() {
            if x_idx < series.values.len() {
                let bottom = cumulative;
                cumulative += series.values[x_idx];
                let top = cumulative;

                if data_y >= bottom && data_y <= top {
                    return Some(HitResult {
                        series: si,
                        index: x_idx,
                        value: series.clone(),
                        screen_position: point,
                    });
                }
            }
        }

        None
    }

    fn data_bounds(&self) -> DataBounds {
        self.bounds
    }

    fn data_count(&self) -> usize {
        self.data.point_count()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// Area chart uniform data.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct AreaUniforms {
    chart: ChartUniforms,
    point_count: u32,
    series_count: u32,
    stacked: u32,
    _pad: u32,
}

/// GPU segment data for area charts.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuAreaSegment {
    x0: f32,
    y0_top: f32,
    y0_bottom: f32,
    series_index: f32,
    x1: f32,
    y1_top: f32,
    y1_bottom: f32,
    _pad: f32,
}
