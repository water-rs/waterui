//! Radar/Spider chart GPU renderer.
//!
//! Renders multivariate data on radial axes with filled polygons.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::{wgpu, GpuContext, GpuFrame, GpuRenderer};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, RadarData};
use crate::interaction::{ChartViewport, HitResult};
use crate::renderer::base::{
    create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer, write_uniform_buffer, MsaaTarget,
};
use crate::renderer::ChartRenderer;

/// GPU-accelerated radar/spider chart renderer.
///
/// Renders multivariate data on radial axes emanating from a center point.
/// Each data series forms a polygon connecting values on each axis.
pub struct RadarRenderer {
    // Data
    data: RadarData,
    bounds: DataBounds,
    ring_count: u32,
    line_width: f32,
    fill_opacity: f32,

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    value_buffer: Option<wgpu::Buffer>,
    color_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,
    msaa_samples: u32,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,
}

impl Default for RadarRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl RadarRenderer {
    /// Creates a new radar chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: RadarData::default(),
            bounds: DataBounds::default(),
            ring_count: 5,
            line_width: 2.0,
            fill_opacity: 0.3,
            pipeline: None,
            uniform_buffer: None,
            value_buffer: None,
            color_buffer: None,
            bind_group: None,
            msaa_target: None,
            msaa_samples: 1,
            animation: ChartAnimation::default(),
            needs_redraw: false,
        }
    }

    /// Sets the number of concentric grid rings.
    #[must_use]
    pub const fn ring_count(mut self, count: u32) -> Self {
        self.ring_count = count;
        self
    }

    /// Sets the line width for outlines and grid.
    #[must_use]
    pub const fn line_width(mut self, width: f32) -> Self {
        self.line_width = width;
        self
    }

    /// Sets the fill opacity for data polygons.
    #[must_use]
    pub const fn fill_opacity(mut self, opacity: f32) -> Self {
        self.fill_opacity = opacity;
        self
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(include_str!("../shaders/radar.wgsl"));
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Radar Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Radar Bind Group Layout"),
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
                        // Values (all series values packed)
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
                        // Colors (per-series)
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
                label: Some("Radar Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Radar Pipeline"),
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
                multisample: multisample_state(ctx.msaa_samples),
                multiview: None,
                cache: ctx.pipeline_cache,
            })
    }

    /// Packs all series values into a flat array for GPU upload.
    fn pack_values(&self) -> Vec<f32> {
        let mut values = Vec::new();
        for series in &self.data.series {
            values.extend(&series.values);
            // Pad to axis_count if needed
            while values.len() % self.data.axis_count as usize != 0 {
                values.push(0.0);
            }
        }
        // Ensure at least one value
        if values.is_empty() {
            values.push(0.0);
        }
        values
    }

    /// Packs all series colors into a flat array for GPU upload.
    fn pack_colors(&self) -> Vec<glam::Vec4> {
        let mut colors: Vec<glam::Vec4> = self
            .data
            .series
            .iter()
            .map(|s| glam::Vec4::new(s.color[0], s.color[1], s.color[2], s.color[3]))
            .collect();
        // Ensure at least one color
        if colors.is_empty() {
            colors.push(glam::Vec4::new(0.5, 0.5, 0.5, 1.0));
        }
        colors
    }
}

impl GpuRenderer for RadarRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = RadarUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Radar Uniforms", &uniforms));

        // Create value buffer
        let initial_values = self.pack_values();
        self.value_buffer = Some(create_storage_buffer(ctx, "Radar Values", &initial_values));

        // Create color buffer
        let initial_colors = self.pack_colors();
        self.color_buffer = Some(create_storage_buffer(ctx, "Radar Colors", &initial_colors));

        // Create bind group
        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Radar Bind Group"),
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
                    resource: self.color_buffer.as_ref().unwrap().as_entire_binding(),
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

        let axis_count = self.data.axis_count;
        let series_count = self.data.series.len() as u32;

        // Need at least 3 axes and 1 series to render
        if axis_count < 3 || series_count == 0 {
            // Clear to transparent
            let mut encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Radar Clear Encoder"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Radar Clear Pass"),
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
        let uniforms = RadarUniforms {
            viewport: glam::Vec4::new(
                frame.width as f32,
                frame.height as f32,
                1.0 / frame.width as f32,
                1.0 / frame.height as f32,
            ),
            config: glam::Vec4::new(
                axis_count as f32,
                series_count as f32,
                self.data.max_value,
                self.ring_count as f32,
            ),
            animation: glam::Vec4::new(
                self.animation.time,
                self.animation.progress,
                self.animation.easing as f32,
                if self.animation.entry_active > 0 { 1.0 } else { 0.0 },
            ),
            style: glam::Vec4::new(self.line_width, self.fill_opacity, 0.0, 0.0),
        };
        write_uniform_buffer(frame.queue, self.uniform_buffer.as_ref().unwrap(), &uniforms);

        // Calculate instance counts:
        // - ring_count rings
        // - axis_count axis lines
        // - series_count fills
        // - series_count outlines
        let total_instances = self.ring_count + axis_count + series_count * 2;

        // Calculate max vertices per instance:
        // - Rings: 6 * axis_count vertices each
        // - Axes: 6 vertices each
        // - Fills: 3 * axis_count vertices each (triangle fan)
        // - Outlines: 6 * axis_count vertices each
        let max_vertices = (6 * axis_count).max(3 * axis_count);

        // Render
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Radar Encoder"),
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
                label: Some("Radar Render Pass"),
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
            pass.draw(0..max_vertices, 0..total_instances);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

/// Hit result for radar chart (series index, axis index).
#[derive(Debug, Clone, Copy, Default)]
pub struct RadarHit {
    /// Series index.
    pub series: usize,
    /// Axis index.
    pub axis: usize,
    /// Value at that axis.
    pub value: f32,
}

impl ChartRenderer for RadarRenderer {
    type Data = RadarData;
    type DataValue = RadarHit;

    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
        self.data = data.clone();
        self.bounds = DataBounds::new(-1.0, 1.0, -1.0, 1.0);

        // Upload values to GPU
        if let Some(buffer) = &self.value_buffer {
            let values = self.pack_values();
            write_storage_buffer(queue, buffer, &values);
        }

        // Upload colors to GPU
        if let Some(buffer) = &self.color_buffer {
            let colors = self.pack_colors();
            write_storage_buffer(queue, buffer, &colors);
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<RadarHit>> {
        if self.data.series.is_empty() || self.data.axis_count < 3 {
            return None;
        }

        // Convert screen point to normalized chart coordinates (-1 to 1)
        let chart_x = ((point.x - viewport.x) / viewport.width) * 2.0 - 1.0;
        let chart_y = ((point.y - viewport.y) / viewport.height) * 2.0 - 1.0;

        // Convert to polar coordinates
        let radius = (chart_x * chart_x + chart_y * chart_y).sqrt();
        let angle = chart_y.atan2(chart_x);

        // Chart radius is 0.4 in NDC
        if radius > 0.45 {
            return None;
        }

        // Find which axis we're closest to
        let axis_count = self.data.axis_count as f32;
        let angle_normalized = (angle + core::f32::consts::FRAC_PI_2) / core::f32::consts::TAU;
        let axis_float = angle_normalized * axis_count;
        let axis = (axis_float.round() as usize) % self.data.axis_count as usize;

        // Find which series has the closest value at this axis
        let value_ratio = radius / 0.4;
        let mut closest_series = 0;
        let mut closest_dist = f32::MAX;

        for (i, series) in self.data.series.iter().enumerate() {
            if let Some(&value) = series.values.get(axis) {
                let series_ratio = value / self.data.max_value;
                let dist = (series_ratio - value_ratio).abs();
                if dist < closest_dist {
                    closest_dist = dist;
                    closest_series = i;
                }
            }
        }

        let value = self.data.series[closest_series]
            .values
            .get(axis)
            .copied()
            .unwrap_or(0.0);

        Some(HitResult {
            series: closest_series,
            index: axis,
            value: RadarHit {
                series: closest_series,
                axis,
                value,
            },
            screen_position: point,
        })
    }

    fn data_bounds(&self) -> DataBounds {
        self.bounds
    }

    fn data_count(&self) -> usize {
        self.data.total_vertices()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// Radar-specific uniforms.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct RadarUniforms {
    /// Viewport: [width, height, 1/width, 1/height].
    viewport: glam::Vec4,
    /// Config: [axis_count, series_count, max_value, ring_count].
    config: glam::Vec4,
    /// Animation: [time, progress, easing, entry_active].
    animation: glam::Vec4,
    /// Style: [line_width, fill_opacity, 0, 0].
    style: glam::Vec4,
}
