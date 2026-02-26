//! Bar chart GPU renderer.
//!
//! Renders bar charts using instanced rendering where each bar is a quad.
//! Supports animation interpolation between data states.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{GpuContext, GpuFrame, GpuView, wgpu};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, DataPoint};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::ChartRenderer;
use crate::renderer::base::{
    ChartUniforms, MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment,
    multisample_state, shader_with_common, write_storage_buffer_with_growth, write_uniform_buffer,
};

const PLOT_PADDING: f32 = 0.1;

/// GPU-accelerated bar chart renderer.
///
/// Uses instanced rendering to draw all bars in a single draw call.
/// Supports smooth animation between data states via double-buffering.
pub struct BarChartRenderer {
    // Data
    data: Vec<DataPoint>,
    bounds: DataBounds,
    bar_color: [f32; 4],

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

impl Default for BarChartRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl BarChartRenderer {
    /// Creates a new bar chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bounds: DataBounds::default(),
            bar_color: [0.231, 0.510, 0.965, 1.0], // Blue #3B82F6
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

    /// Sets the bar color from sRGB values.
    pub fn set_color(&mut self, color: Srgb) {
        self.bar_color = [color.red, color.green, color.blue, 1.0];
    }

    /// Sets the bar color with alpha.
    pub fn set_color_with_alpha(&mut self, color: Srgb, alpha: f32) {
        self.bar_color = [color.red, color.green, color.blue, alpha];
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
        let shader_source = shader_with_common(include_str!("../shaders/bar.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Bar Chart Shader",
            &shader_source,
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Bar Chart Bind Group Layout"),
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
                        // Current data (accessed by both vertex and fragment shaders)
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
                label: Some("Bar Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Bar Chart Pipeline"),
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

    fn to_gpu_data(&self, data: &[DataPoint]) -> Vec<GpuDataPoint> {
        data.iter()
            .map(|p| GpuDataPoint {
                x: p.x,
                y: p.y,
                color: glam::Vec4::from_array(self.bar_color),
            })
            .collect()
    }

    fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };
        let Some(current_buffer) = &self.current_buffer else {
            return;
        };
        let Some(previous_buffer) = &self.previous_buffer else {
            return;
        };

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bar Chart Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: current_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: previous_buffer.as_entire_binding(),
                },
            ],
        }));
    }
}

impl GpuView for BarChartRenderer {
    fn setup(&mut self, ctx: &GpuContext, _env: &mut waterui_core::Environment) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = ChartUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Bar Chart Uniforms", &uniforms));

        // Create data buffers with initial capacity
        let initial_capacity = self.data.len().max(16384);
        let initial_data = self.to_gpu_data(&self.data);

        self.current_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Bar Chart Current Data",
                &vec![GpuDataPoint::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Bar Chart Current Data", &initial_data)
        });

        self.previous_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Bar Chart Previous Data",
                &vec![GpuDataPoint::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Bar Chart Previous Data", &initial_data)
        });

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

        if self.data.is_empty() {
            // Clear to transparent
            let mut encoder =
                frame
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Bar Chart Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Bar Chart Clear Pass"),
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

        // Update zoom/pan state from gesture input
        self.zoom_pan
            .update(&frame.gesture, frame.width as f32, frame.height as f32);

        // Transform bounds based on zoom/pan state
        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);

        // Update uniforms with transformed bounds
        let uniforms = ChartUniforms {
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
                        self.data.len() as f32,
                    )
                } else {
                    glam::Vec4::new(-1.0, -1.0, 0.0, self.data.len() as f32)
                }
            } else {
                glam::Vec4::new(-1.0, -1.0, 0.0, self.data.len() as f32)
            },
        };
        write_uniform_buffer(
            frame.queue,
            self.uniform_buffer.as_ref().unwrap(),
            &uniforms,
        );

        // Render bars
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Bar Chart Encoder"),
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
                label: Some("Bar Chart Render Pass"),
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
            // 6 vertices per bar (2 triangles), instanced by bar count
            pass.draw(0..6, 0..self.data.len() as u32);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl ChartRenderer for BarChartRenderer {
    type Data = Vec<DataPoint>;
    type DataValue = DataPoint;

    fn update_data(&mut self, data: &Self::Data, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Swap buffers for animation
        core::mem::swap(&mut self.current_buffer, &mut self.previous_buffer);

        // Update data
        let previous_data = core::mem::replace(&mut self.data, data.clone());
        self.bounds = DataBounds::from_points(&self.data);

        let current_gpu_data = self.to_gpu_data(data);
        let mut previous_gpu_data = self.to_gpu_data(&previous_data);
        if previous_gpu_data.len() < current_gpu_data.len() {
            previous_gpu_data.resize(current_gpu_data.len(), GpuDataPoint::default());
        }

        let mut needs_rebind = false;

        if let Some(buffer) = self.current_buffer.as_mut() {
            needs_rebind |= write_storage_buffer_with_growth(
                device,
                queue,
                buffer,
                "Bar Chart Current Data",
                &current_gpu_data,
            );
        }
        if let Some(buffer) = self.previous_buffer.as_mut() {
            needs_rebind |= write_storage_buffer_with_growth(
                device,
                queue,
                buffer,
                "Bar Chart Previous Data",
                &previous_gpu_data,
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

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<DataPoint>> {
        if self.data.is_empty() {
            return None;
        }

        let (chart_x, chart_y) = super::chart_coords_from_viewport(viewport, point, PLOT_PADDING)?;
        let chart_y = 1.0 - chart_y;

        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);
        if visible_bounds.width() <= 0.0 || visible_bounds.height() <= 0.0 {
            return None;
        }

        // Find which bar was hit
        let bar_count = self.data.len();
        let bar_width = 0.8 / bar_count as f32;
        let x_range = visible_bounds.max_x - visible_bounds.min_x;
        if x_range <= 0.0 {
            return None;
        }

        for (i, data_point) in self.data.iter().enumerate() {
            let normalized_x = (data_point.x - visible_bounds.min_x) / x_range;
            let bar_left = normalized_x - bar_width * 0.5;
            let bar_right = normalized_x + bar_width * 0.5;

            if chart_x >= bar_left && chart_x <= bar_right {
                // Check Y (bar height)
                let normalized_y = (data_point.y - visible_bounds.min_y)
                    / (visible_bounds.max_y - visible_bounds.min_y);
                if chart_y <= normalized_y {
                    return Some(HitResult {
                        series: 0,
                        index: i,
                        value: *data_point,
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
        self.data.len()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// GPU-friendly data point with color.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuDataPoint {
    x: f32,
    y: f32,
    color: glam::Vec4,
}
