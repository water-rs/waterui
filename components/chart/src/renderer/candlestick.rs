//! Candlestick (K-line) chart GPU renderer.
//!
//! Renders OHLC candles using instanced rendering where each candle
//! consists of a body (open-close) and wick (high-low).

use alloc::vec::Vec;
use core::future::Future;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{wgpu, GpuContext, GpuFrame, GpuRenderer};

use crate::animation::ChartAnimation;
use crate::data::{Candle, DataBounds};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::renderer::base::{
    create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer, write_uniform_buffer, ChartUniforms, MsaaTarget,
};
use crate::renderer::ChartRenderer;

const PLOT_PADDING: f32 = 0.1;

/// GPU-accelerated candlestick chart renderer.
///
/// Uses instanced rendering to draw all candles in a single draw call.
/// Each candle is rendered as two elements: body and wick.
pub struct CandlestickRenderer {
    // Data
    data: Vec<Candle>,
    bounds: DataBounds,
    bullish_color: [f32; 4],
    bearish_color: [f32; 4],

    // GPU resources (initialized on setup)
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    color_buffer: Option<wgpu::Buffer>,
    current_buffer: Option<wgpu::Buffer>,
    previous_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl Default for CandlestickRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl CandlestickRenderer {
    /// Creates a new candlestick chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bounds: DataBounds::default(),
            bullish_color: [0.133, 0.773, 0.369, 1.0], // Green #22C55E
            bearish_color: [0.937, 0.267, 0.267, 1.0], // Red #EF4444
            pipeline: None,
            uniform_buffer: None,
            color_buffer: None,
            current_buffer: None,
            previous_buffer: None,
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

    /// Sets the bullish (up) candle color.
    pub fn set_bullish_color(&mut self, color: Srgb) {
        self.bullish_color = [color.red, color.green, color.blue, 1.0];
    }

    /// Sets the bearish (down) candle color.
    pub fn set_bearish_color(&mut self, color: Srgb) {
        self.bearish_color = [color.red, color.green, color.blue, 1.0];
    }

    /// Sets the candle data directly (for initial setup before GPU is ready).
    pub fn set_data(&mut self, data: Vec<Candle>) {
        self.bounds = DataBounds::from_candles(&data);
        self.data = data;
    }

    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        let blend = if ctx.is_hdr() {
            None
        } else {
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING)
        };
        let shader_source = shader_with_common(include_str!("../shaders/candlestick.wgsl"));
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Candlestick Chart Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Candlestick Chart Bind Group Layout"),
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
                        // Current data
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
                        // Previous data (for animation)
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
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
                label: Some("Candlestick Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Candlestick Chart Pipeline"),
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

impl GpuRenderer for CandlestickRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl Future<Output = ()> {
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = ChartUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "Candlestick Chart Uniforms", &uniforms));

        // Create color buffer
        let colors = CandleColors {
            bullish: glam::Vec4::from_array(self.bullish_color),
            bearish: glam::Vec4::from_array(self.bearish_color),
        };
        self.color_buffer = Some(create_uniform_buffer(ctx, "Candlestick Colors", &colors));

        // Create data buffers with initial capacity
        let initial_capacity = self.data.len().max(64);
        let initial_data: Vec<GpuCandle> = self
            .data
            .iter()
            .map(|c| GpuCandle {
                timestamp: c.timestamp,
                open: c.open,
                high: c.high,
                low: c.low,
                close: c.close,
                volume: c.volume,
                _pad: glam::Vec2::ZERO,
            })
            .collect();

        self.current_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Candlestick Current Data",
                &vec![GpuCandle::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Candlestick Current Data", &initial_data)
        });

        self.previous_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Candlestick Previous Data",
                &vec![GpuCandle::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Candlestick Previous Data", &initial_data)
        });

        // Create bind group
        let bind_group_layout = self.pipeline.as_ref().unwrap().get_bind_group_layout(0);
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Candlestick Chart Bind Group"),
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
                    resource: self.current_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
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

        if self.data.is_empty() {
            // Clear to transparent
            let mut encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Candlestick Chart Clear Encoder"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Candlestick Chart Clear Pass"),
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

        // Update colors
        let colors = CandleColors {
            bullish: glam::Vec4::from_array(self.bullish_color),
            bearish: glam::Vec4::from_array(self.bearish_color),
        };
        write_uniform_buffer(frame.queue, self.color_buffer.as_ref().unwrap(), &colors);

        // Render candles
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Candlestick Chart Encoder"),
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
                label: Some("Candlestick Chart Render Pass"),
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
            // 6 vertices per element, 2 elements (body + wick) per candle
            pass.draw(0..6, 0..(self.data.len() as u32 * 2));
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl ChartRenderer for CandlestickRenderer {
    type Data = Vec<Candle>;
    type DataValue = Candle;

    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
        // Swap buffers for animation
        core::mem::swap(&mut self.current_buffer, &mut self.previous_buffer);

        // Update data
        self.data = data.clone();
        self.bounds = DataBounds::from_candles(&self.data).with_padding(0.05);

        // Upload to GPU
        if let Some(buffer) = &self.current_buffer {
            let gpu_data: Vec<GpuCandle> = data
                .iter()
                .map(|c| GpuCandle {
                    timestamp: c.timestamp,
                    open: c.open,
                    high: c.high,
                    low: c.low,
                    close: c.close,
                    volume: c.volume,
                    _pad: glam::Vec2::ZERO,
                })
                .collect();
            write_storage_buffer(queue, buffer, &gpu_data);
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = animation.progress < 1.0;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<Candle>> {
        if self.data.is_empty() {
            return None;
        }

        let (chart_x, chart_y) = super::chart_coords_from_viewport(viewport, point, PLOT_PADDING)?;
        let chart_y = 1.0 - chart_y;

        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);
        if visible_bounds.width() <= 0.0 || visible_bounds.height() <= 0.0 {
            return None;
        }

        // Find which candle was hit
        let candle_count = self.data.len();
        let candle_width = 0.8 / candle_count as f32;
        let gap = 0.2 / (candle_count + 1) as f32;

        for (i, candle) in self.data.iter().enumerate() {
            let candle_center = gap + (candle_width + gap) * i as f32 + candle_width * 0.5;
            let candle_left = candle_center - candle_width * 0.5;
            let candle_right = candle_center + candle_width * 0.5;

            if chart_x >= candle_left && chart_x <= candle_right {
                // Check Y (candle range: low to high)
                let y_range = visible_bounds.max_y - visible_bounds.min_y;
                if y_range > 0.0 {
                    let normalized_low = (candle.low - visible_bounds.min_y) / y_range;
                    let normalized_high = (candle.high - visible_bounds.min_y) / y_range;

                    if chart_y >= normalized_low && chart_y <= normalized_high {
                        return Some(HitResult {
                            series: 0,
                            index: i,
                            value: *candle,
                            screen_position: point,
                        });
                    }
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

/// GPU-friendly candle data.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuCandle {
    timestamp: f32,
    open: f32,
    high: f32,
    low: f32,
    close: f32,
    volume: f32,
    _pad: glam::Vec2,
}

/// Colors for bullish/bearish candles.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct CandleColors {
    bullish: glam::Vec4,
    bearish: glam::Vec4,
}
