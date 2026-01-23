//! Depth (order book) chart GPU renderer.
//!
//! Renders order book depth as two filled area charts meeting at the mid price.
//! Bids are shown in green, asks in red.

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{wgpu, GpuContext, GpuFrame, GpuRenderer};

use crate::animation::ChartAnimation;
use crate::data::{DataBounds, DepthData, DepthLevel};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::base::{
    create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer, write_uniform_buffer, ChartUniforms, MsaaTarget,
};
use crate::renderer::ChartRenderer;

const PLOT_PADDING: f32 = 0.1;

/// GPU-accelerated depth chart renderer.
///
/// Renders order book depth as step-area charts with bids on the left
/// and asks on the right, meeting at the mid price.
pub struct DepthRenderer {
    // Data
    data: DepthData,
    bounds: DataBounds,
    bid_fill_color: [f32; 4],
    bid_line_color: [f32; 4],
    ask_fill_color: [f32; 4],
    ask_line_color: [f32; 4],

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    color_buffer: Option<wgpu::Buffer>,
    bid_buffer: Option<wgpu::Buffer>,
    ask_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl Default for DepthRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl DepthRenderer {
    /// Creates a new depth chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: DepthData::default(),
            bounds: DataBounds::default(),
            bid_fill_color: [0.133, 0.773, 0.369, 0.5],  // Green with 50% opacity
            bid_line_color: [0.133, 0.773, 0.369, 1.0],  // Green #22C55E
            ask_fill_color: [0.937, 0.267, 0.267, 0.5],  // Red with 50% opacity
            ask_line_color: [0.937, 0.267, 0.267, 1.0],  // Red #EF4444
            pipeline: None,
            uniform_buffer: None,
            color_buffer: None,
            bid_buffer: None,
            ask_buffer: None,
            bind_group: None,
            msaa_target: None,
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

    /// Sets the bid (buy) colors.
    pub fn set_bid_color(&mut self, fill: Srgb, line: Srgb) {
        self.bid_fill_color = [fill.red, fill.green, fill.blue, 0.5];
        self.bid_line_color = [line.red, line.green, line.blue, 1.0];
    }

    /// Sets the ask (sell) colors.
    pub fn set_ask_color(&mut self, fill: Srgb, line: Srgb) {
        self.ask_fill_color = [fill.red, fill.green, fill.blue, 0.5];
        self.ask_line_color = [line.red, line.green, line.blue, 1.0];
    }

    /// Sets the depth data directly.
    pub fn set_data(&mut self, data: DepthData) {
        self.bounds = data.bounds();
        self.data = data;
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        };
        let shader_source = shader_with_common(include_str!("../shaders/depth.wgsl"));
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Depth Chart Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Depth Chart Bind Group Layout"),
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
                        // Colors
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // Bids
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
                        // Asks
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
                label: Some("Depth Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Depth Chart Pipeline"),
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
                multisample: multisample_state(ctx.surface_format),
                multiview: None,
                cache: ctx.pipeline_cache,
            })
    }
}

impl GpuRenderer for DepthRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = ChartUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Depth Chart Uniforms", &uniforms));

        // Create color buffer
        let mid_price = self.data.mid_price().unwrap_or(0.0);
        let colors = DepthColors {
            bid_fill: glam::Vec4::from_array(self.bid_fill_color),
            bid_line: glam::Vec4::from_array(self.bid_line_color),
            ask_fill: glam::Vec4::from_array(self.ask_fill_color),
            ask_line: glam::Vec4::from_array(self.ask_line_color),
            mid_price,
            line_width: 2.0,
            _pad: glam::Vec2::ZERO,
        };
        self.color_buffer = Some(create_uniform_buffer(ctx, "Depth Colors", &colors));

        // Create bid/ask buffers
        let initial_capacity = 64;
        let bid_data: Vec<GpuDepthLevel> = self
            .data
            .bids
            .iter()
            .map(|l| GpuDepthLevel {
                price: l.price,
                cumulative_volume: l.cumulative_volume,
            })
            .collect();

        let ask_data: Vec<GpuDepthLevel> = self
            .data
            .asks
            .iter()
            .map(|l| GpuDepthLevel {
                price: l.price,
                cumulative_volume: l.cumulative_volume,
            })
            .collect();

        self.bid_buffer = Some(if bid_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Depth Bids",
                &vec![GpuDepthLevel::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Depth Bids", &bid_data)
        });

        self.ask_buffer = Some(if ask_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Depth Asks",
                &vec![GpuDepthLevel::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Depth Asks", &ask_data)
        });

        // Create bind group
        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Depth Chart Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.color_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.bid_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.ask_buffer.as_ref().unwrap().as_entire_binding(),
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

        let total_levels = self.data.bids.len() + self.data.asks.len();

        if total_levels == 0 {
            // Clear to transparent
            let mut encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Depth Chart Clear Encoder"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Depth Chart Clear Pass"),
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
        self.zoom_pan.update(&frame.gesture, frame.width as f32, frame.height as f32);

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
                if self.animation.entry_active > 0 { 1.0 } else { 0.0 },
            ),
            pointer: if let Some((x, y)) = frame.pointer_normalized() {
                if let Some((px, py)) = super::unpad_normalized_point(x, y, PLOT_PADDING) {
                    glam::Vec4::new(px, py, if frame.pointer.hit.is_some() { 1.0 } else { 0.0 }, 0.0)
                } else {
                    glam::Vec4::new(-1.0, -1.0, 0.0, 0.0)
                }
            } else {
                glam::Vec4::new(-1.0, -1.0, 0.0, 0.0)
            },
        };
        write_uniform_buffer(frame.queue, self.uniform_buffer.as_ref().unwrap(), &uniforms);

        // Update colors with mid price
        let mid_price = self.data.mid_price().unwrap_or(
            (self.bounds.min_x + self.bounds.max_x) / 2.0
        );
        let colors = DepthColors {
            bid_fill: glam::Vec4::from_array(self.bid_fill_color),
            bid_line: glam::Vec4::from_array(self.bid_line_color),
            ask_fill: glam::Vec4::from_array(self.ask_fill_color),
            ask_line: glam::Vec4::from_array(self.ask_line_color),
            mid_price,
            line_width: 2.0,
            _pad: glam::Vec2::ZERO,
        };
        write_uniform_buffer(frame.queue, self.color_buffer.as_ref().unwrap(), &colors);

        // Render depth chart
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Depth Chart Encoder"),
            });

        {
            let (color_view, resolve_target) = msaa_attachment(
                &mut self.msaa_target,
                frame.device,
                frame.format,
                frame.width,
                frame.height,
                &frame.view,
            );

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Depth Chart Render Pass"),
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
            // 6 vertices per step, 2 instances per level (fill + step)
            pass.draw(0..6, 0..(total_levels as u32 * 2));
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl ChartRenderer for DepthRenderer {
    type Data = DepthData;
    type DataValue = DepthLevel;

    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
        // Update data
        self.data = data.clone();
        self.bounds = data.bounds().with_padding(0.05);

        // Upload bids to GPU
        if let Some(buffer) = &self.bid_buffer {
            let gpu_data: Vec<GpuDepthLevel> = data
                .bids
                .iter()
                .map(|l| GpuDepthLevel {
                    price: l.price,
                    cumulative_volume: l.cumulative_volume,
                })
                .collect();
            if !gpu_data.is_empty() {
                write_storage_buffer(queue, buffer, &gpu_data);
            }
        }

        // Upload asks to GPU
        if let Some(buffer) = &self.ask_buffer {
            let gpu_data: Vec<GpuDepthLevel> = data
                .asks
                .iter()
                .map(|l| GpuDepthLevel {
                    price: l.price,
                    cumulative_volume: l.cumulative_volume,
                })
                .collect();
            if !gpu_data.is_empty() {
                write_storage_buffer(queue, buffer, &gpu_data);
            }
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<DepthLevel>> {
        if self.data.bids.is_empty() && self.data.asks.is_empty() {
            return None;
        }

        let (chart_x, _chart_y) = super::chart_coords_from_viewport(viewport, point, PLOT_PADDING)?;

        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);
        if visible_bounds.width() <= 0.0 || visible_bounds.height() <= 0.0 {
            return None;
        }

        // Convert to price
        let price_range = visible_bounds.max_x - visible_bounds.min_x;
        let price = visible_bounds.min_x + chart_x * price_range;

        let mid_price = self.data.mid_price().unwrap_or(
            (visible_bounds.min_x + visible_bounds.max_x) / 2.0
        );

        // Find the closest level
        if price < mid_price {
            // Check bids
            for (i, level) in self.data.bids.iter().enumerate() {
                if level.price <= price {
                    return Some(HitResult {
                        series: 0, // Bids
                        index: i,
                        value: *level,
                        screen_position: point,
                    });
                }
            }
        } else {
            // Check asks
            for (i, level) in self.data.asks.iter().enumerate() {
                if level.price >= price {
                    return Some(HitResult {
                        series: 1, // Asks
                        index: i,
                        value: *level,
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
        self.data.bids.len() + self.data.asks.len()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// GPU-friendly depth level.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuDepthLevel {
    price: f32,
    cumulative_volume: f32,
}

/// Colors for depth chart.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct DepthColors {
    bid_fill: glam::Vec4,
    bid_line: glam::Vec4,
    ask_fill: glam::Vec4,
    ask_line: glam::Vec4,
    mid_price: f32,
    line_width: f32,
    _pad: glam::Vec2,
}
