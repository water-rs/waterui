//! Bubble chart GPU renderer.
//!
//! Renders scatter points with variable sizes.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, wgpu};

use crate::animation::ChartAnimation;
use crate::data::{BubblePoint, DataBounds};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::ChartRenderer;
use crate::renderer::base::{
    MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer, write_uniform_buffer,
};

const PLOT_PADDING: f32 = 0.1;

/// GPU-accelerated bubble chart renderer.
///
/// Renders scatter points where each point has a variable radius
/// proportional to its size value.
pub struct BubbleRenderer {
    // Data
    data: Vec<BubblePoint>,
    bounds: DataBounds,
    size_bounds: (f32, f32),

    // Style
    default_color: Srgb,
    min_radius: f32,
    max_radius: f32,
    opacity: f32,

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    point_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,
    msaa_samples: u32,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl Default for BubbleRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl BubbleRenderer {
    /// Creates a new bubble chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bounds: DataBounds::default(),
            size_bounds: (0.0, 1.0),
            default_color: Srgb::new(0.23, 0.51, 0.96), // Blue
            min_radius: 5.0,
            max_radius: 30.0,
            opacity: 0.7,
            pipeline: None,
            uniform_buffer: None,
            point_buffer: None,
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

    /// Sets the default color for bubbles without custom colors.
    #[must_use]
    pub fn color(mut self, color: Srgb) -> Self {
        self.default_color = color;
        self
    }

    /// Sets the minimum bubble radius in pixels.
    #[must_use]
    pub fn min_radius(mut self, radius: f32) -> Self {
        assert!(
            radius.is_finite() && radius > 0.0,
            "Bubble min radius must be > 0"
        );
        self.min_radius = radius;
        if self.max_radius < self.min_radius {
            self.max_radius = self.min_radius;
        }
        self
    }

    /// Sets the maximum bubble radius in pixels.
    #[must_use]
    pub fn max_radius(mut self, radius: f32) -> Self {
        assert!(
            radius.is_finite() && radius > 0.0,
            "Bubble max radius must be > 0"
        );
        self.max_radius = radius;
        if self.max_radius < self.min_radius {
            self.min_radius = self.max_radius;
        }
        self
    }

    /// Sets the bubble opacity.
    #[must_use]
    pub fn opacity(mut self, opacity: f32) -> Self {
        assert!(
            opacity.is_finite() && (0.0..=1.0).contains(&opacity),
            "Bubble opacity must be in [0.0, 1.0]"
        );
        self.opacity = opacity;
        self
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(include_str!("../shaders/bubble.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Bubble Shader",
            &shader_source,
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Bubble Bind Group Layout"),
                    entries: &[
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
                label: Some("Bubble Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Bubble Pipeline"),
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

    fn compute_bounds(data: &[BubblePoint]) -> (DataBounds, (f32, f32)) {
        if data.is_empty() {
            return (DataBounds::default(), (0.0, 1.0));
        }

        let mut bounds = DataBounds {
            min_x: f32::MAX,
            max_x: f32::MIN,
            min_y: f32::MAX,
            max_y: f32::MIN,
        };
        let mut size_min = f32::MAX;
        let mut size_max = f32::MIN;

        for p in data {
            bounds.min_x = bounds.min_x.min(p.x);
            bounds.max_x = bounds.max_x.max(p.x);
            bounds.min_y = bounds.min_y.min(p.y);
            bounds.max_y = bounds.max_y.max(p.y);
            size_min = size_min.min(p.size);
            size_max = size_max.max(p.size);
        }

        (bounds.with_padding(0.1), (size_min, size_max))
    }

    /// Convert data to GPU format
    fn to_gpu_points(&self) -> Vec<GpuBubblePoint> {
        self.data
            .iter()
            .map(|p| GpuBubblePoint {
                x: p.x,
                y: p.y,
                size: p.size,
                color: glam::Vec4::new(p.color[0], p.color[1], p.color[2], p.color[3]),
            })
            .collect()
    }
}

impl GpuRenderer for BubbleRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        self.pipeline = Some(Self::create_pipeline(ctx));

        let uniforms = BubbleUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Bubble Uniforms", &uniforms));

        let mut initial_points = if self.data.is_empty() {
            vec![GpuBubblePoint::default(); 16384]
        } else {
            self.to_gpu_points()
        };
        if initial_points.len() < 16384 {
            initial_points.resize(16384, GpuBubblePoint::default());
        }
        self.point_buffer = Some(create_storage_buffer(ctx, "Bubble Points", &initial_points));

        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bubble Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.point_buffer.as_ref().unwrap().as_entire_binding(),
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

        if self.data.is_empty() {
            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Bubble Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Bubble Clear Pass"),
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

        let uniforms = BubbleUniforms {
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
            default_color: glam::Vec4::new(
                self.default_color.red,
                self.default_color.green,
                self.default_color.blue,
                self.opacity,
            ),
            size_config: glam::Vec4::new(
                self.min_radius,
                self.max_radius,
                self.size_bounds.0,
                self.size_bounds.1,
            ),
        };
        write_uniform_buffer(
            frame.queue,
            self.uniform_buffer.as_ref().unwrap(),
            &uniforms,
        );

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Bubble Encoder"),
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
                label: Some("Bubble Render Pass"),
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
            pass.draw(0..6, 0..self.data.len() as u32);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl ChartRenderer for BubbleRenderer {
    type Data = Vec<BubblePoint>;
    type DataValue = BubblePoint;

    fn update_data(&mut self, data: &Self::Data, _device: &wgpu::Device, queue: &wgpu::Queue) {
        self.data = data.clone();
        let (bounds, size_bounds) = Self::compute_bounds(data);
        self.bounds = bounds;
        self.size_bounds = size_bounds;

        if let Some(buffer) = &self.point_buffer {
            let gpu_points = self.to_gpu_points();
            if !gpu_points.is_empty() {
                write_storage_buffer(queue, buffer, &gpu_points);
            }
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<BubblePoint>> {
        if self.data.is_empty() {
            return None;
        }

        let (chart_x, chart_y) = super::chart_coords_from_viewport(viewport, point, PLOT_PADDING)?;
        let chart_y = 1.0 - chart_y;

        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);
        if visible_bounds.width() <= 0.0 || visible_bounds.height() <= 0.0 {
            return None;
        }

        // Convert to data coordinates
        let data_x = visible_bounds.min_x + chart_x * visible_bounds.width();
        let data_y = visible_bounds.min_y + chart_y * visible_bounds.height();

        // Find closest point within radius
        let mut closest_idx = None;
        let mut closest_dist = f32::MAX;

        for (i, p) in self.data.iter().enumerate() {
            let dx = p.x - data_x;
            let dy = p.y - data_y;
            let dist = (dx * dx + dy * dy).sqrt();

            // Check if within bubble radius (approximate)
            let denom = (1.0 - 2.0 * PLOT_PADDING).max(0.001);
            let radius_data = self.max_radius * visible_bounds.width() / (viewport.width * denom);
            if dist < radius_data && dist < closest_dist {
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

#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct BubbleUniforms {
    viewport: glam::Vec4,
    bounds: glam::Vec4,
    animation: glam::Vec4,
    default_color: glam::Vec4,
    size_config: glam::Vec4,
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuBubblePoint {
    x: f32,
    y: f32,
    size: f32,
    color: glam::Vec4,
}
