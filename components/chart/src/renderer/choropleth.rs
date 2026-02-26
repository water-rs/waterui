//! Choropleth map renderer for geographic visualization.
//!
//! Renders tessellated polygons colored by data values using GPU acceleration.
//! Supports color scales, hover highlighting, and smooth data transitions.

extern crate alloc;

use alloc::vec::Vec;

use bytemuck::cast_slice;
use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, wgpu};
use wgpu::util::DeviceExt;

use super::ChartRenderer;
use super::base::{
    MsaaTarget, create_storage_buffer, create_uniform_buffer, msaa_attachment, multisample_state,
    shader_with_common, write_storage_buffer_with_growth, write_uniform_buffer,
};
use crate::animation::ChartAnimation;
use crate::data::{ChoroplethData, DataBounds};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};
use crate::params::{ChartParamError, PositiveF32};

/// GPU-accelerated choropleth map renderer.
///
/// Renders geographic polygons with:
/// - Tessellated triangle rendering
/// - Color scale mapping from data values
/// - Smooth animation between data states
/// - Hover highlighting
pub struct ChoroplethRenderer {
    // Data
    data: ChoroplethData,
    bounds: DataBounds,

    // Configuration
    stroke_width: f32,
    stroke_color: Srgb,
    show_stroke: bool,

    // GPU resources
    pipeline: Option<wgpu::RenderPipeline>,
    uniform_buffer: Option<wgpu::Buffer>,
    vertex_buffer: Option<wgpu::Buffer>,
    prev_vertex_buffer: Option<wgpu::Buffer>,
    index_buffer: Option<wgpu::Buffer>,
    color_stop_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    msaa_target: Option<MsaaTarget>,
    msaa_samples: u32,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Cached data
    total_indices: u32,
    vertex_capacity: usize,
    index_capacity: usize,
    color_stop_capacity: usize,
    hover_polygon_id: Option<u32>,
    pending_vertices: Option<Vec<GpuVertex>>,
    pending_prev_vertices: Option<Vec<GpuVertex>>,
    pending_indices: Option<Vec<u32>>,
    pending_color_stops: Option<Vec<GpuColorStop>>,

    // Zoom/pan state for interactive navigation
    zoom_pan: ZoomPanState,
}

impl ChoroplethRenderer {
    /// Creates a new choropleth renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: ChoroplethData::default(),
            bounds: DataBounds::default(),
            stroke_width: 1.0,
            stroke_color: Srgb::new(1.0, 1.0, 1.0),
            show_stroke: true,
            pipeline: None,
            uniform_buffer: None,
            vertex_buffer: None,
            prev_vertex_buffer: None,
            index_buffer: None,
            color_stop_buffer: None,
            bind_group: None,
            msaa_target: None,
            msaa_samples: 1,
            animation: ChartAnimation::default(),
            needs_redraw: false,
            total_indices: 0,
            vertex_capacity: 0,
            index_capacity: 0,
            color_stop_capacity: 0,
            hover_polygon_id: None,
            pending_vertices: None,
            pending_prev_vertices: None,
            pending_indices: None,
            pending_color_stops: None,
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

    /// Sets the stroke width for polygon borders.
    #[must_use]
    pub fn stroke_width(self, width: f32) -> Self {
        self.try_stroke_width(width)
            .expect("ChoroplethRenderer::stroke_width(width) requires finite width > 0")
    }

    /// Sets stroke width using a validated strong type.
    #[must_use]
    pub fn with_stroke_width(mut self, width: PositiveF32) -> Self {
        self.stroke_width = width.get();
        self
    }

    /// Fallible variant of [`Self::stroke_width`].
    pub fn try_stroke_width(self, width: f32) -> Result<Self, ChartParamError> {
        Ok(self.with_stroke_width(PositiveF32::try_new(width)?))
    }

    /// Sets the stroke color.
    #[must_use]
    pub fn stroke_color(mut self, color: Srgb) -> Self {
        self.stroke_color = color;
        self
    }

    /// Sets whether to show polygon borders.
    #[must_use]
    pub const fn show_stroke(mut self, show: bool) -> Self {
        self.show_stroke = show;
        self
    }

    /// Converts choropleth data to GPU vertex format.
    fn data_to_vertices(&self) -> (Vec<GpuVertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut vertex_offset = 0u32;

        for polygon in &self.data.polygons {
            // Add vertices for this polygon
            for &[x, y] in &polygon.vertices {
                vertices.push(GpuVertex {
                    pos: glam::Vec2::new(x, y),
                    value: polygon.value,
                    polygon_id: polygon.id as f32,
                });
            }

            // Add indices, offset by current vertex count
            for &idx in &polygon.indices {
                indices.push(vertex_offset + idx);
            }

            vertex_offset += polygon.vertices.len() as u32;
        }

        (vertices, indices)
    }

    /// Converts color scale to GPU format.
    fn color_scale_to_stops(&self) -> Vec<GpuColorStop> {
        self.data
            .color_scale
            .stops
            .iter()
            .map(|(pos, color)| GpuColorStop {
                position: *pos,
                _pad: [0.0; 3],
                color: glam::Vec4::new(color.red, color.green, color.blue, 1.0),
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
        let Some(vertex_buffer) = &self.vertex_buffer else {
            return;
        };
        let Some(prev_vertex_buffer) = &self.prev_vertex_buffer else {
            return;
        };
        let Some(color_stop_buffer) = &self.color_stop_buffer else {
            return;
        };

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("choropleth_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: vertex_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: prev_vertex_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: color_stop_buffer.as_entire_binding(),
                },
            ],
        }));
    }
}

impl Default for ChoroplethRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuRenderer for ChoroplethRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl core::future::Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create shader
        let shader_source = shader_with_common(include_str!("../shaders/choropleth.wgsl"));
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "choropleth_shader",
            &shader_source,
        );

        // Bind group layout
        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("choropleth_bind_group_layout"),
                    entries: &[
                        // Uniforms
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                        // Current vertices
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
                        // Previous vertices
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
                        // Color stops
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

        // Pipeline layout
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("choropleth_pipeline_layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        // Render pipeline
        self.pipeline = Some(
            ctx.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("choropleth_pipeline"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: shader.as_ref(),
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: shader.as_ref(),
                        entry_point: Some("fs_main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: ctx.surface_format,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                    }),
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None, // No culling for 2D map
                        polygon_mode: wgpu::PolygonMode::Fill,
                        unclipped_depth: false,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: multisample_state(ctx.msaa_samples),
                    multiview: None,
                    cache: None,
                }),
        );

        // Create initial buffers with default data
        let default_uniforms = ChoroplethUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(
            ctx,
            "choropleth_uniforms",
            &default_uniforms,
        ));

        // Create vertex buffers sized for current data (fallback to 1 element)
        let default_vertex = GpuVertex {
            pos: glam::Vec2::ZERO,
            value: 0.0,
            polygon_id: 0.0,
        };
        let (mut vertices, indices) = self.data_to_vertices();
        if vertices.is_empty() {
            vertices.push(default_vertex);
        }
        self.vertex_capacity = vertices.len();
        self.vertex_buffer = Some(create_storage_buffer(ctx, "choropleth_vertices", &vertices));
        self.prev_vertex_buffer = Some(create_storage_buffer(
            ctx,
            "choropleth_prev_vertices",
            &vertices,
        ));

        // Create index buffer (for tessellated geometry)
        let index_len = indices.len();
        let index_data = if index_len == 0 { vec![0_u32] } else { indices };
        self.index_capacity = index_data.len();
        self.total_indices = index_len as u32;
        self.index_buffer = Some(ctx.device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("choropleth_indices"),
                contents: cast_slice(&index_data),
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            },
        ));

        // Create color stop buffer
        let default_stops = vec![
            GpuColorStop {
                position: 0.0,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.27, 0.0, 0.33, 1.0),
            },
            GpuColorStop {
                position: 0.25,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.23, 0.32, 0.55, 1.0),
            },
            GpuColorStop {
                position: 0.5,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.13, 0.57, 0.55, 1.0),
            },
            GpuColorStop {
                position: 0.75,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.37, 0.79, 0.38, 1.0),
            },
            GpuColorStop {
                position: 1.0,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.99, 0.91, 0.15, 1.0),
            },
        ];
        let stops = self.color_scale_to_stops();
        let stops = if stops.is_empty() {
            default_stops
        } else {
            stops
        };
        self.color_stop_capacity = stops.len();
        self.color_stop_buffer = Some(create_storage_buffer(ctx, "choropleth_color_stops", &stops));

        self.rebuild_bind_group(ctx.device);

        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        // Update zoom/pan state from gesture input
        self.zoom_pan
            .update(&frame.gesture, frame.width as f32, frame.height as f32);

        // Upload pending buffers and resize if needed.
        let default_vertex = GpuVertex {
            pos: glam::Vec2::ZERO,
            value: 0.0,
            polygon_id: 0.0,
        };
        let default_stops = [
            GpuColorStop {
                position: 0.0,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.27, 0.0, 0.33, 1.0),
            },
            GpuColorStop {
                position: 0.25,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.23, 0.32, 0.55, 1.0),
            },
            GpuColorStop {
                position: 0.5,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.13, 0.57, 0.55, 1.0),
            },
            GpuColorStop {
                position: 0.75,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.37, 0.79, 0.38, 1.0),
            },
            GpuColorStop {
                position: 1.0,
                _pad: [0.0; 3],
                color: glam::Vec4::new(0.99, 0.91, 0.15, 1.0),
            },
        ];
        let mut needs_rebind = false;

        if let Some(mut prev_vertices) = self.pending_prev_vertices.take() {
            if prev_vertices.is_empty() {
                prev_vertices.push(default_vertex);
            }
            if let Some(buffer) = self.prev_vertex_buffer.as_mut() {
                needs_rebind |= write_storage_buffer_with_growth(
                    frame.device,
                    frame.queue,
                    buffer,
                    "choropleth_prev_vertices",
                    &prev_vertices,
                );
            }
            if prev_vertices.len() > self.vertex_capacity {
                self.vertex_capacity = prev_vertices.len();
            }
        }

        if let Some(mut vertices) = self.pending_vertices.take() {
            if vertices.is_empty() {
                vertices.push(default_vertex);
            }
            if let Some(buffer) = self.vertex_buffer.as_mut() {
                needs_rebind |= write_storage_buffer_with_growth(
                    frame.device,
                    frame.queue,
                    buffer,
                    "choropleth_vertices",
                    &vertices,
                );
            }
            if vertices.len() > self.vertex_capacity {
                self.vertex_capacity = vertices.len();
            }
        }

        if let Some(indices) = self.pending_indices.take() {
            let index_len = indices.len();
            let index_data = if index_len == 0 { vec![0_u32] } else { indices };
            if index_data.len() > self.index_capacity {
                self.index_buffer = Some(frame.device.create_buffer_init(
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("choropleth_indices"),
                        contents: cast_slice(&index_data),
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    },
                ));
                self.index_capacity = index_data.len();
            } else if let Some(buffer) = &self.index_buffer {
                frame.queue.write_buffer(buffer, 0, cast_slice(&index_data));
            }
            self.total_indices = index_len as u32;
        }

        if let Some(stops) = self.pending_color_stops.take() {
            let stops = if stops.is_empty() {
                default_stops.to_vec()
            } else {
                stops
            };
            if let Some(buffer) = self.color_stop_buffer.as_mut() {
                needs_rebind |= write_storage_buffer_with_growth(
                    frame.device,
                    frame.queue,
                    buffer,
                    "choropleth_color_stops",
                    &stops,
                );
            }
            if stops.len() > self.color_stop_capacity {
                self.color_stop_capacity = stops.len();
            }
        }

        if needs_rebind {
            self.rebuild_bind_group(frame.device);
        }

        let Some(pipeline) = &self.pipeline else {
            return;
        };
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        let Some(index_buffer) = &self.index_buffer else {
            return;
        };

        if self.total_indices == 0 {
            let mut encoder = frame
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("choropleth_clear_encoder"),
                });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("choropleth_clear_pass"),
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
            self.needs_redraw = self.animation.progress < 1.0;
            return;
        }

        // Update uniforms
        // Transform geo bounds for zoom/pan
        let geo_bounds = self.data.bounds();
        let geo_data_bounds =
            DataBounds::new(geo_bounds[0], geo_bounds[2], geo_bounds[1], geo_bounds[3]);
        let visible_bounds = self.zoom_pan.transform_bounds(&geo_data_bounds);
        let uniforms = ChoroplethUniforms {
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
            pointer: glam::Vec4::new(
                frame.pointer.position.map_or(-1.0, |p| p.x),
                frame.pointer.position.map_or(-1.0, |p| p.y),
                if frame.pointer.hit.is_some() {
                    1.0
                } else {
                    0.0
                },
                self.hover_polygon_id.map_or(-1.0, |id| id as f32),
            ),
            value_range: glam::Vec4::new(
                self.data.min_value,
                self.data.max_value,
                self.stroke_width,
                if self.show_stroke { 1.0 } else { 0.0 },
            ),
            stroke_color: glam::Vec4::new(
                self.stroke_color.red,
                self.stroke_color.green,
                self.stroke_color.blue,
                1.0,
            ),
        };

        if let Some(buffer) = &self.uniform_buffer {
            write_uniform_buffer(frame.queue, buffer, &uniforms);
        }

        // Begin render pass
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("choropleth_encoder"),
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
                label: Some("choropleth_pass"),
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
            pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.total_indices, 0, 0..1);
        }

        frame.queue.submit(Some(encoder.finish()));

        // Check if animation is in progress
        self.needs_redraw = self.animation.progress < 1.0;
    }
}

impl ChartRenderer for ChoroplethRenderer {
    type Data = ChoroplethData;
    type DataValue = u32; // Polygon ID

    fn update_data(&mut self, data: &Self::Data, __device: &wgpu::Device, _queue: &wgpu::Queue) {
        // Capture current vertices as "previous" for animation
        let (mut prev_vertices, _) = self.data_to_vertices();

        // Update data
        self.data = data.clone();

        // Calculate bounds
        let geo_bounds = data.bounds();
        self.bounds = DataBounds::new(geo_bounds[0], geo_bounds[2], geo_bounds[1], geo_bounds[3]);

        // Convert to GPU format
        let (vertices, indices) = self.data_to_vertices();
        self.total_indices = indices.len() as u32;

        if prev_vertices.len() < vertices.len() {
            prev_vertices.extend(vertices.iter().skip(prev_vertices.len()).copied());
        }

        self.pending_prev_vertices = Some(prev_vertices);
        self.pending_vertices = Some(vertices);
        self.pending_indices = Some(indices);
        self.pending_color_stops = Some(self.color_scale_to_stops());

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = true;
    }

    fn hit_test(
        &self,
        point: Point,
        viewport: &ChartViewport,
    ) -> Option<HitResult<Self::DataValue>> {
        if self.data.polygons.is_empty() {
            return None;
        }

        let geo_bounds = self.data.bounds();
        let geo_data_bounds =
            DataBounds::new(geo_bounds[0], geo_bounds[2], geo_bounds[1], geo_bounds[3]);
        let visible_bounds = self.zoom_pan.transform_bounds(&geo_data_bounds);
        let lon_range = visible_bounds.max_x - visible_bounds.min_x;
        let lat_range = visible_bounds.max_y - visible_bounds.min_y;

        if lon_range <= 0.0 || lat_range <= 0.0 {
            return None;
        }

        // Convert screen point to geographic coordinates
        let (norm_x, norm_y) = viewport.screen_to_normalized(point)?;

        let lon = visible_bounds.min_x + norm_x * lon_range;
        let lat = visible_bounds.max_y - norm_y * lat_range; // Y is flipped

        // Point-in-polygon test for each polygon
        for polygon in &self.data.polygons {
            if point_in_polygon(lon, lat, &polygon.vertices) {
                return Some(HitResult {
                    series: 0,
                    index: polygon.id as usize,
                    value: polygon.id,
                    screen_position: point,
                });
            }
        }

        None
    }

    fn data_bounds(&self) -> DataBounds {
        self.bounds
    }

    fn data_count(&self) -> usize {
        self.data.polygons.len()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

/// Point-in-polygon test using ray casting algorithm.
fn point_in_polygon(x: f32, y: f32, vertices: &[[f32; 2]]) -> bool {
    let n = vertices.len();
    if n < 3 {
        return false;
    }

    let mut inside = false;
    let mut j = n - 1;

    for i in 0..n {
        let xi = vertices[i][0];
        let yi = vertices[i][1];
        let xj = vertices[j][0];
        let yj = vertices[j][1];

        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }

        j = i;
    }

    inside
}

/// GPU vertex data for choropleth polygons.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuVertex {
    pos: glam::Vec2,
    value: f32,
    polygon_id: f32,
}

/// GPU color stop for color scale.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuColorStop {
    position: f32,
    _pad: [f32; 3],
    color: glam::Vec4,
}

/// GPU uniforms for choropleth rendering.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct ChoroplethUniforms {
    viewport: glam::Vec4,
    bounds: glam::Vec4,
    animation: glam::Vec4,
    pointer: glam::Vec4,
    value_range: glam::Vec4,
    stroke_color: glam::Vec4,
}
