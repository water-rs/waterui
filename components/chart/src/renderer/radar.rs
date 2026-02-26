//! Radar/Spider chart GPU renderer.
//!
//! Renders multivariate data on radial axes with filled polygons.

use alloc::vec::Vec;
use core::future::Future;
use core::num::NonZeroU32;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::{GpuContext, GpuFrame, GpuView, wgpu};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, RadarData};
use crate::interaction::{ChartViewport, HitResult};
use crate::params::{ChartParamError, PositiveF32, UnitInterval};
use crate::renderer::ChartRenderer;
use crate::renderer::base::{
    MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer_with_growth, write_uniform_buffer,
};

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
    pub fn ring_count(self, count: u32) -> Self {
        self.try_ring_count(count)
            .expect("RadarRenderer::ring_count(count) requires count >= 1")
    }

    /// Sets ring count using a validated strong type.
    #[must_use]
    pub fn with_ring_count(mut self, count: NonZeroU32) -> Self {
        self.ring_count = count.get();
        self
    }

    /// Fallible variant of [`Self::ring_count`].
    pub fn try_ring_count(self, count: u32) -> Result<Self, ChartParamError> {
        let count = NonZeroU32::new(count).ok_or(ChartParamError::OutOfRange {
            param: "ring_count",
            value: count as f32,
            min: 1.0,
            max: u32::MAX as f32,
        })?;
        Ok(self.with_ring_count(count))
    }

    /// Sets the line width for outlines and grid.
    #[must_use]
    pub fn line_width(self, width: f32) -> Self {
        self.try_line_width(width)
            .expect("RadarRenderer::line_width(width) requires finite width > 0")
    }

    /// Sets line width using a validated strong type.
    #[must_use]
    pub fn with_line_width(mut self, width: PositiveF32) -> Self {
        self.line_width = width.get();
        self
    }

    /// Fallible variant of [`Self::line_width`].
    pub fn try_line_width(self, width: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_line_width(PositiveF32::try_new(width)?))
    }

    /// Sets the fill opacity for data polygons.
    #[must_use]
    pub fn fill_opacity(self, opacity: f32) -> Self {
        self.try_fill_opacity(opacity)
            .expect("RadarRenderer::fill_opacity(opacity) requires finite 0.0 <= opacity <= 1.0")
    }

    /// Sets fill opacity using a validated strong type.
    #[must_use]
    pub fn with_fill_opacity(mut self, opacity: UnitInterval) -> Self {
        self.fill_opacity = opacity.get();
        self
    }

    /// Fallible variant of [`Self::fill_opacity`].
    pub fn try_fill_opacity(self, opacity: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_fill_opacity(UnitInterval::try_new(opacity)?))
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(include_str!("../shaders/radar.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Radar Shader",
            &shader_source,
        );

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
        let Some(color_buffer) = &self.color_buffer else {
            return;
        };

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Radar Bind Group"),
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: color_buffer.as_entire_binding(),
                },
            ],
        }));
    }
}

impl GpuView for RadarRenderer {
    fn setup(&mut self, ctx: &GpuContext, _env: &mut waterui_core::Environment) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = RadarUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Radar Uniforms", &uniforms));

        // Create value buffer
        let mut initial_values = self.pack_values();
        if initial_values.len() < 4096 {
            initial_values.resize(4096, 0.0);
        }
        self.value_buffer = Some(create_storage_buffer(ctx, "Radar Values", &initial_values));

        // Create color buffer
        let mut initial_colors = self.pack_colors();
        if initial_colors.len() < 256 {
            initial_colors.resize(256, glam::Vec4::ZERO);
        }
        self.color_buffer = Some(create_storage_buffer(ctx, "Radar Colors", &initial_colors));

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

        let axis_count = self.data.axis_count;
        let series_count = self.data.series.len() as u32;

        // Need at least 3 axes and 1 series to render
        if axis_count < 3 || series_count == 0 {
            // Clear to transparent
            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Radar Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Radar Clear Pass"),
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
                if self.animation.entry_active > 0 {
                    1.0
                } else {
                    0.0
                },
            ),
            style: glam::Vec4::new(self.line_width, self.fill_opacity, 0.0, 0.0),
        };
        write_uniform_buffer(
            frame.queue,
            self.uniform_buffer.as_ref().unwrap(),
            &uniforms,
        );

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

    fn update_data(&mut self, data: &Self::Data, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.data = data.clone();
        self.bounds = DataBounds::new(-1.0, 1.0, -1.0, 1.0);

        let values = self.pack_values();
        let colors = self.pack_colors();
        let mut needs_rebind = false;

        // Upload values to GPU
        if let Some(buffer) = self.value_buffer.as_mut() {
            needs_rebind |=
                write_storage_buffer_with_growth(device, queue, buffer, "Radar Values", &values);
        }

        // Upload colors to GPU
        if let Some(buffer) = self.color_buffer.as_mut() {
            needs_rebind |=
                write_storage_buffer_with_growth(device, queue, buffer, "Radar Colors", &colors);
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

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<RadarHit>> {
        if self.data.series.is_empty() || self.data.axis_count < 3 {
            return None;
        }

        // Convert screen point to shader chart-space coordinates.
        // Shader compresses x in NDC by (height / width), so inverse-transform here.
        let ndc_x = ((point.x - viewport.x) / viewport.width) * 2.0 - 1.0;
        let chart_y = ((point.y - viewport.y) / viewport.height) * 2.0 - 1.0;
        let chart_x = ndc_x * (viewport.width / viewport.height.max(1.0));

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
