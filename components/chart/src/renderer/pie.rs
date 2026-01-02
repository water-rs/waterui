//! Pie chart GPU renderer.
//!
//! Renders pie/donut charts using GPU shaders with smooth animations
//! and hover interactions.

extern crate alloc;

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{wgpu, GpuContext, GpuFrame, GpuRenderer};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, DataPoint};
use crate::interaction::{ChartViewport, HitResult};
use crate::renderer::base::{
    create_storage_buffer, create_uniform_buffer, shader_with_common,
    write_storage_buffer, write_uniform_buffer,
};
use crate::renderer::ChartRenderer;

/// GPU-accelerated pie chart renderer.
pub struct PieChartRenderer {
    // Data
    data: Vec<DataPoint>,
    colors: Vec<Srgb>,

    // Appearance
    inner_radius: f32, // 0.0 for pie, > 0.0 for donut
    start_angle: f32,  // Starting angle in radians

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    slice_buffer: Option<wgpu::Buffer>,
    previous_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,

    // Animation state
    animation: ChartAnimation,
    previous_data: Vec<DataPoint>,
    needs_redraw: bool,
}

/// GPU slice data.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuSliceData {
    /// Start angle and end angle.
    angles: glam::Vec2,
    /// Value (for animation).
    value: f32,
    /// Color [r, g, b, a].
    color: glam::Vec4,
}

/// Pie chart uniforms.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct PieUniforms {
    /// Viewport [width, height, 1/width, 1/height].
    viewport: glam::Vec4,
    /// Animation [time, progress, easing, entry_active].
    animation: glam::Vec4,
    /// Pointer [x, y, pressed, hovered_slice].
    pointer: glam::Vec4,
    /// Config [inner_radius, start_angle, slice_count, 0].
    config: glam::Vec4,
}

const PIE_SHADER: &str = include_str!("../shaders/pie.wgsl");

// Default color palette for slices
const DEFAULT_COLORS: &[u32] = &[
    0x3B82F6, // Blue
    0x22C55E, // Green
    0xEF4444, // Red
    0xF59E0B, // Amber
    0x8B5CF6, // Purple
    0xEC4899, // Pink
    0x06B6D4, // Cyan
    0xF97316, // Orange
];

impl PieChartRenderer {
    /// Creates a new pie chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            colors: Vec::new(),
            inner_radius: 0.0,
            start_angle: -core::f32::consts::FRAC_PI_2, // Start at top
            pipeline: None,
            uniform_buffer: None,
            slice_buffer: None,
            previous_buffer: None,
            bind_group: None,
            animation: ChartAnimation::new(),
            previous_data: Vec::new(),
            needs_redraw: true,
        }
    }

    /// Sets the data points.
    pub fn set_data(&mut self, data: Vec<DataPoint>) {
        self.previous_data = core::mem::replace(&mut self.data, data);

        // Pad previous data to match current length
        while self.previous_data.len() < self.data.len() {
            self.previous_data.push(DataPoint::new(0.0, 0.0));
        }

        self.needs_redraw = true;
    }

    /// Sets custom colors for slices.
    pub fn set_colors(&mut self, colors: Vec<Srgb>) {
        self.colors = colors;
        self.needs_redraw = true;
    }

    /// Sets the inner radius (0.0 for pie, > 0.0 for donut).
    /// Value is a fraction of the outer radius (0.0 to 1.0).
    pub fn set_inner_radius(&mut self, radius: f32) {
        self.inner_radius = radius.clamp(0.0, 0.95);
        self.needs_redraw = true;
    }

    /// Sets the starting angle in radians.
    pub fn set_start_angle(&mut self, angle: f32) {
        self.start_angle = angle;
        self.needs_redraw = true;
    }

    /// Gets the color for a slice index.
    fn get_color(&self, index: usize) -> Srgb {
        if index < self.colors.len() {
            self.colors[index]
        } else {
            let hex = DEFAULT_COLORS[index % DEFAULT_COLORS.len()];
            Srgb::from_u32(hex)
        }
    }

    /// Calculates slice data for GPU.
    fn calculate_slices(&self) -> Vec<GpuSliceData> {
        if self.data.is_empty() {
            return Vec::new();
        }

        let total: f32 = self.data.iter().map(|p| p.y.max(0.0)).sum();
        if total <= 0.0 {
            return Vec::new();
        }

        let mut slices = Vec::with_capacity(self.data.len());
        let mut current_angle = self.start_angle;
        let two_pi = core::f32::consts::TAU;

        for (i, point) in self.data.iter().enumerate() {
            let value = point.y.max(0.0);
            let fraction = value / total;
            let sweep = fraction * two_pi;
            let end_angle = current_angle + sweep;

            let color = self.get_color(i);

            slices.push(GpuSliceData {
                angles: glam::Vec2::new(current_angle, end_angle),
                value,
                color: glam::Vec4::new(color.red, color.green, color.blue, 1.0),
            });

            current_angle = end_angle;
        }

        slices
    }

    /// Calculates previous slice data for GPU animation.
    fn calculate_previous_slices(&self) -> Vec<GpuSliceData> {
        if self.previous_data.is_empty() {
            return self.calculate_slices(); // Use current as fallback
        }

        let total: f32 = self.previous_data.iter().map(|p| p.y.max(0.0)).sum();
        if total <= 0.0 {
            return self.calculate_slices();
        }

        let mut slices = Vec::with_capacity(self.previous_data.len());
        let mut current_angle = self.start_angle;
        let two_pi = core::f32::consts::TAU;

        for (i, point) in self.previous_data.iter().enumerate() {
            let value = point.y.max(0.0);
            let fraction = value / total;
            let sweep = fraction * two_pi;
            let end_angle = current_angle + sweep;

            let color = self.get_color(i);

            slices.push(GpuSliceData {
                angles: glam::Vec2::new(current_angle, end_angle),
                value,
                color: glam::Vec4::new(color.red, color.green, color.blue, 1.0),
            });

            current_angle = end_angle;
        }

        slices
    }

    /// Creates the render pipeline.
    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        };
        let shader_source = shader_with_common(PIE_SHADER);
        let shader = ctx
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Pie Chart Shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Pie Chart Bind Group Layout"),
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
                label: Some("Pie Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Pie Chart Pipeline"),
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

impl Default for PieChartRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuRenderer for PieChartRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = PieUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Pie Chart Uniforms", &uniforms));

        // Create slice buffers with initial capacity
        let max_slices = 64;
        let initial_slices = self.calculate_slices();
        let initial_slice_data: Vec<GpuSliceData> = if initial_slices.is_empty() {
            vec![GpuSliceData::default(); max_slices]
        } else {
            let mut data = initial_slices;
            data.resize(max_slices, GpuSliceData::default());
            data
        };

        self.slice_buffer = Some(create_storage_buffer(
            ctx,
            "Pie Chart Slice Data",
            &initial_slice_data,
        ));

        self.previous_buffer = Some(create_storage_buffer(
            ctx,
            "Pie Chart Previous Data",
            &initial_slice_data,
        ));

        // Create bind group
        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Pie Chart Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.slice_buffer.as_ref().unwrap().as_entire_binding(),
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

        // Update slice data
        let slices = self.calculate_slices();
        if !slices.is_empty() {
            write_storage_buffer(frame.queue, self.slice_buffer.as_ref().unwrap(), &slices);
        }

        // Determine hovered slice
        let hovered_slice: f32 = if let Some((x, y)) = frame.pointer_normalized() {
            // Simple hit test based on angle
            let cx = x - 0.5;
            let cy = y - 0.5;
            let dist = (cx * cx + cy * cy).sqrt();
            let outer = 0.45; // 90% of 0.5
            let inner = outer * self.inner_radius;

            if dist >= inner && dist <= outer && !self.data.is_empty() {
                let angle = cy.atan2(cx);
                let total: f32 = self.data.iter().map(|p| p.y.max(0.0)).sum();
                if total > 0.0 {
                    let mut current = self.start_angle;
                    let two_pi = core::f32::consts::TAU;
                    let mut found = -1.0_f32;
                    for (i, point) in self.data.iter().enumerate() {
                        let sweep = (point.y.max(0.0) / total) * two_pi;
                        let end = current + sweep;

                        // Normalize angle for comparison
                        let mut test = angle;
                        while test < current {
                            test += two_pi;
                        }
                        while test >= current + two_pi {
                            test -= two_pi;
                        }

                        if test >= current && test < end {
                            found = i as f32;
                            break;
                        }
                        current = end;
                    }
                    found
                } else {
                    -1.0
                }
            } else {
                -1.0
            }
        } else {
            -1.0
        };

        // Update uniforms
        let uniforms = PieUniforms {
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
                if self.animation.entry_active > 0 { 1.0 } else { 0.0 },
            ),
            pointer: if let Some((x, y)) = frame.pointer_normalized() {
                glam::Vec4::new(x, y, if frame.pointer.hit.is_some() { 1.0 } else { 0.0 }, hovered_slice)
            } else {
                glam::Vec4::new(-1.0, -1.0, 0.0, -1.0)
            },
            config: glam::Vec4::new(
                self.inner_radius,
                self.start_angle,
                self.data.len() as f32,
                0.0,
            ),
        };
        write_uniform_buffer(frame.queue, self.uniform_buffer.as_ref().unwrap(), &uniforms);

        // Render
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Pie Chart Encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Pie Chart Render Pass"),
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

            // Draw fullscreen quad
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);

        self.needs_redraw = !self.animation.is_complete();
    }
}

impl ChartRenderer for PieChartRenderer {
    type Data = Vec<DataPoint>;
    type DataValue = f32;

    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
        self.previous_data = core::mem::take(&mut self.data);
        self.data = data.clone();

        // Update GPU buffer
        if let Some(buffer) = &self.slice_buffer {
            let slices = self.calculate_slices();
            if !slices.is_empty() {
                write_storage_buffer(queue, buffer, &slices);
            }
        }
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = true;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<Self::DataValue>> {
        if self.data.is_empty() {
            return None;
        }

        // Convert to centered coordinates
        let norm = viewport.screen_to_normalized(point)?;
        let cx = norm.0 - 0.5;
        let cy = norm.1 - 0.5;

        // Calculate radius and angle
        let outer = 0.45; // 90% of 0.5
        let inner = outer * self.inner_radius;
        let dist = (cx * cx + cy * cy).sqrt();

        // Check if within pie/donut ring
        if dist < inner || dist > outer {
            return None;
        }

        // Calculate angle
        let angle = cy.atan2(cx);

        // Find which slice contains this angle
        let total: f32 = self.data.iter().map(|p| p.y.max(0.0)).sum();
        if total <= 0.0 {
            return None;
        }

        let mut current_angle = self.start_angle;
        let two_pi = core::f32::consts::TAU;

        for (i, data_point) in self.data.iter().enumerate() {
            let fraction = data_point.y.max(0.0) / total;
            let sweep = fraction * two_pi;
            let end_angle = current_angle + sweep;

            // Normalize angle to same range
            let mut test_angle = angle;
            while test_angle < current_angle {
                test_angle += two_pi;
            }
            while test_angle > current_angle + two_pi {
                test_angle -= two_pi;
            }

            if test_angle >= current_angle && test_angle < end_angle {
                return Some(HitResult::new(0, i, data_point.y, point));
            }

            current_angle = end_angle;
        }

        None
    }

    fn data_bounds(&self) -> DataBounds {
        // Pie charts don't have typical bounds - return a unit square
        DataBounds {
            min_x: 0.0,
            max_x: 1.0,
            min_y: 0.0,
            max_y: 1.0,
        }
    }

    fn data_count(&self) -> usize {
        self.data.len()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw || !self.animation.is_complete()
    }
}
