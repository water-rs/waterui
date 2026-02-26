//! Gauge chart GPU renderer.
//!
//! Renders a circular speedometer-style gauge with animated value and optional needle.

use alloc::vec::Vec;
use core::f32::consts::PI;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, wgpu};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, GaugeData};
use crate::interaction::{ChartViewport, HitResult};
use crate::renderer::ChartRenderer;
use crate::renderer::base::{
    MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer_with_growth, write_uniform_buffer,
};

/// GPU-accelerated gauge chart renderer.
///
/// Renders a circular speedometer-style gauge with an arc showing
/// the current value and optional colored threshold regions.
pub struct GaugeRenderer {
    // Data
    data: GaugeData,
    previous_value: f32,

    // Style
    start_angle: f32,
    end_angle: f32,
    inner_radius: f32,
    outer_radius: f32,
    needle_width: f32,
    needle_length: f32,
    background_color: Srgb,
    value_color: Srgb,
    needle_color: Srgb,

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    region_color_buffer: Option<wgpu::Buffer>,
    region_threshold_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,
    msaa_samples: u32,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,
}

impl Default for GaugeRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GaugeRenderer {
    /// Creates a new gauge chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: GaugeData::default(),
            previous_value: 0.0,
            // Default to 270 degree arc (from -135° to 135°)
            start_angle: -PI * 0.75,
            end_angle: PI * 0.75,
            inner_radius: 0.3,
            outer_radius: 0.45,
            needle_width: 0.02,
            needle_length: 0.42,
            background_color: Srgb::new(0.2, 0.2, 0.2),
            value_color: Srgb::new(0.23, 0.51, 0.96),
            needle_color: Srgb::new(0.9, 0.9, 0.9),
            pipeline: None,
            uniform_buffer: None,
            region_color_buffer: None,
            region_threshold_buffer: None,
            bind_group: None,
            msaa_target: None,
            msaa_samples: 1,
            animation: ChartAnimation::default(),
            needs_redraw: false,
        }
    }

    /// Sets the arc angle range in radians.
    /// Default is -135° to 135° (270° arc).
    #[must_use]
    pub fn arc_angles(mut self, start: f32, end: f32) -> Self {
        assert!(
            start.is_finite() && end.is_finite() && end > start,
            "Gauge renderer arc requires finite end > start"
        );
        self.start_angle = start;
        self.end_angle = end;
        self
    }

    /// Sets the inner and outer radius (0.0 to 0.5, relative to widget size).
    #[must_use]
    pub fn radii(mut self, inner: f32, outer: f32) -> Self {
        assert!(
            inner.is_finite() && outer.is_finite() && inner >= 0.0 && outer > inner && outer <= 0.5,
            "Gauge renderer radii require 0.0 <= inner < outer <= 0.5"
        );
        self.inner_radius = inner;
        self.outer_radius = outer;
        self
    }

    /// Sets the background arc color.
    #[must_use]
    pub fn background_color(mut self, color: Srgb) -> Self {
        self.background_color = color;
        self
    }

    /// Sets the value arc color.
    #[must_use]
    pub fn value_color(mut self, color: Srgb) -> Self {
        self.value_color = color;
        self
    }

    /// Sets the needle color.
    #[must_use]
    pub fn needle_color(mut self, color: Srgb) -> Self {
        self.needle_color = color;
        self
    }

    /// Sets needle dimensions.
    #[must_use]
    pub fn needle_size(mut self, width: f32, length: f32) -> Self {
        self.needle_width = width;
        self.needle_length = length;
        self
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(include_str!("../shaders/gauge.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Gauge Chart Shader",
            &shader_source,
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Gauge Chart Bind Group Layout"),
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
                        // Region colors
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // Region thresholds
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
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
                label: Some("Gauge Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Gauge Chart Pipeline"),
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

    fn get_region_colors(&self) -> Vec<glam::Vec4> {
        if self.data.regions.is_empty() {
            vec![glam::Vec4::new(
                self.value_color.red,
                self.value_color.green,
                self.value_color.blue,
                1.0,
            )]
        } else {
            self.data
                .regions
                .iter()
                .map(|r| glam::Vec4::new(r.color[0], r.color[1], r.color[2], r.color[3]))
                .collect()
        }
    }

    fn get_region_thresholds(&self) -> Vec<f32> {
        if self.data.regions.is_empty() {
            vec![1.0]
        } else {
            // Normalize thresholds to 0-1 range
            self.data
                .regions
                .iter()
                .map(|r| {
                    if self.data.max_value <= self.data.min_value {
                        0.0
                    } else {
                        (r.threshold - self.data.min_value)
                            / (self.data.max_value - self.data.min_value)
                    }
                })
                .collect()
        }
    }

    fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };
        let Some(region_color_buffer) = &self.region_color_buffer else {
            return;
        };
        let Some(region_threshold_buffer) = &self.region_threshold_buffer else {
            return;
        };

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Gauge Chart Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: region_color_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: region_threshold_buffer.as_entire_binding(),
                },
            ],
        }));
    }
}

impl GpuRenderer for GaugeRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        self.pipeline = Some(Self::create_pipeline(ctx));

        let uniforms = GaugeUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(
            ctx,
            "Gauge Chart Uniforms",
            &uniforms,
        ));

        let mut colors = self.get_region_colors();
        if colors.len() < 256 {
            colors.resize(256, glam::Vec4::ZERO);
        }
        self.region_color_buffer = Some(create_storage_buffer(ctx, "Gauge Region Colors", &colors));

        let mut thresholds = self.get_region_thresholds();
        if thresholds.len() < 256 {
            thresholds.resize(256, 0.0);
        }
        self.region_threshold_buffer = Some(create_storage_buffer(
            ctx,
            "Gauge Region Thresholds",
            &thresholds,
        ));

        self.rebuild_bind_group(ctx.device);

        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            return;
        };

        // Update uniforms
        let uniforms = GaugeUniforms {
            viewport: glam::Vec4::new(
                frame.width as f32,
                frame.height as f32,
                1.0 / frame.width as f32,
                1.0 / frame.height as f32,
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
            config: glam::Vec4::new(
                self.start_angle,
                self.end_angle,
                self.inner_radius,
                self.outer_radius,
            ),
            value: glam::Vec4::new(
                self.data.value,
                self.previous_value,
                self.data.min_value,
                self.data.max_value,
            ),
            needle: glam::Vec4::new(
                self.needle_width,
                self.needle_length,
                if self.data.show_needle { 1.0 } else { 0.0 },
                0.0,
            ),
            background_color: glam::Vec4::new(
                self.background_color.red,
                self.background_color.green,
                self.background_color.blue,
                1.0,
            ),
            value_color: glam::Vec4::new(
                self.value_color.red,
                self.value_color.green,
                self.value_color.blue,
                1.0,
            ),
            needle_color: glam::Vec4::new(
                self.needle_color.red,
                self.needle_color.green,
                self.needle_color.blue,
                1.0,
            ),
        };
        write_uniform_buffer(
            frame.queue,
            self.uniform_buffer.as_ref().unwrap(),
            &uniforms,
        );

        // Render
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Gauge Chart Encoder"),
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
                label: Some("Gauge Chart Render Pass"),
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
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl ChartRenderer for GaugeRenderer {
    type Data = GaugeData;
    type DataValue = f32;

    fn update_data(&mut self, data: &Self::Data, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.previous_value = self.data.value;
        self.data = data.clone();

        let colors = self.get_region_colors();
        let thresholds = self.get_region_thresholds();
        let mut needs_rebind = false;

        // Update region buffers
        if let Some(buffer) = self.region_color_buffer.as_mut() {
            needs_rebind |= write_storage_buffer_with_growth(
                device,
                queue,
                buffer,
                "Gauge Region Colors",
                &colors,
            );
        }

        if let Some(buffer) = self.region_threshold_buffer.as_mut() {
            needs_rebind |= write_storage_buffer_with_growth(
                device,
                queue,
                buffer,
                "Gauge Region Thresholds",
                &thresholds,
            );
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

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<f32>> {
        // Check if point is within the gauge arc
        let center_x = viewport.x + viewport.width / 2.0;
        let center_y = viewport.y + viewport.height / 2.0;
        let size = viewport.width.min(viewport.height);

        let dx = (point.x - center_x) / size;
        let dy = (point.y - center_y) / size;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist >= self.inner_radius && dist <= self.outer_radius {
            Some(HitResult {
                series: 0,
                index: 0,
                value: self.data.value,
                screen_position: point,
            })
        } else {
            None
        }
    }

    fn data_bounds(&self) -> DataBounds {
        DataBounds::new(
            self.data.min_value,
            self.data.max_value,
            self.data.min_value,
            self.data.max_value,
        )
    }

    fn data_count(&self) -> usize {
        1
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// Gauge chart uniform data.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GaugeUniforms {
    viewport: glam::Vec4,
    animation: glam::Vec4,
    config: glam::Vec4,
    value: glam::Vec4,
    needle: glam::Vec4,
    background_color: glam::Vec4,
    value_color: glam::Vec4,
    needle_color: glam::Vec4,
}
