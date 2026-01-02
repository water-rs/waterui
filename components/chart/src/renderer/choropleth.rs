//! Choropleth map renderer for geographic visualization.
//!
//! Renders tessellated polygons colored by data values using GPU acceleration.
//! Supports color scales, hover highlighting, and smooth data transitions.

extern crate alloc;

use alloc::vec::Vec;

use encase::ShaderType;
use waterui_core::layout::Point;
use waterui_graphics::color::Srgb;
use waterui_graphics::{wgpu, GpuContext, GpuFrame, GpuRenderer};

use super::base::{create_storage_buffer, create_uniform_buffer, shader_with_common, write_storage_buffer, write_uniform_buffer, ChartUniforms};
use super::ChartRenderer;
use crate::animation::ChartAnimation;
use crate::data::{ChoroplethData, DataBounds};
use crate::interaction::{ChartViewport, HitResult, ZoomPanState};

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
    color_stop_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,

    // Animation state
    animation: ChartAnimation,
    needs_redraw: bool,

    // Cached data
    total_indices: u32,
    hover_polygon_id: Option<u32>,

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
            color_stop_buffer: None,
            bind_group: None,
            animation: ChartAnimation::default(),
            needs_redraw: false,
            total_indices: 0,
            hover_polygon_id: None,
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
    pub const fn stroke_width(mut self, width: f32) -> Self {
        self.stroke_width = width;
        self
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
        self.data.color_scale.stops.iter().map(|(pos, color)| {
            GpuColorStop {
                position: *pos,
                _pad: [0.0; 3],
                color: glam::Vec4::new(color.red, color.green, color.blue, 1.0),
            }
        }).collect()
    }
}

impl Default for ChoroplethRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuRenderer for ChoroplethRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl core::future::Future<Output = ()> {
        // Create shader
        let shader_source = shader_with_common(include_str!("../shaders/choropleth.wgsl"));
        let shader = ctx.device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("choropleth_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // Bind group layout
        let bind_group_layout = ctx.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        let pipeline_layout = ctx.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("choropleth_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Render pipeline
        self.pipeline = Some(ctx.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("choropleth_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        }));

        // Create initial buffers with default data
        let default_uniforms = ChoroplethUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(ctx, "choropleth_uniforms", &default_uniforms));

        // Create vertex buffers with minimum size
        let default_vertex = GpuVertex {
            pos: glam::Vec2::ZERO,
            value: 0.0,
            polygon_id: 0.0,
        };
        self.vertex_buffer = Some(create_storage_buffer(ctx, "choropleth_vertices", &[default_vertex]));
        self.prev_vertex_buffer = Some(create_storage_buffer(ctx, "choropleth_prev_vertices", &[default_vertex]));

        // Create color stop buffer
        let default_stops = vec![
            GpuColorStop { position: 0.0, _pad: [0.0; 3], color: glam::Vec4::new(0.27, 0.0, 0.33, 1.0) },
            GpuColorStop { position: 0.25, _pad: [0.0; 3], color: glam::Vec4::new(0.23, 0.32, 0.55, 1.0) },
            GpuColorStop { position: 0.5, _pad: [0.0; 3], color: glam::Vec4::new(0.13, 0.57, 0.55, 1.0) },
            GpuColorStop { position: 0.75, _pad: [0.0; 3], color: glam::Vec4::new(0.37, 0.79, 0.38, 1.0) },
            GpuColorStop { position: 1.0, _pad: [0.0; 3], color: glam::Vec4::new(0.99, 0.91, 0.15, 1.0) },
        ];
        self.color_stop_buffer = Some(create_storage_buffer(ctx, "choropleth_color_stops", &default_stops));

        // Create bind group
        self.bind_group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("choropleth_bind_group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.vertex_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.prev_vertex_buffer.as_ref().unwrap().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.color_stop_buffer.as_ref().unwrap().as_entire_binding(),
                },
            ],
        }));

        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        let Some(pipeline) = &self.pipeline else { return };
        let Some(bind_group) = &self.bind_group else { return };

        // Update zoom/pan state from gesture input
        self.zoom_pan
            .update(&frame.gesture, frame.width as f32, frame.height as f32);

        if self.total_indices == 0 {
            return;
        }

        // Update uniforms
        // Transform geo bounds for zoom/pan
        let geo_bounds = self.data.bounds();
        let geo_data_bounds = DataBounds::new(geo_bounds[0], geo_bounds[2], geo_bounds[1], geo_bounds[3]);
        let visible_bounds = self.zoom_pan.transform_bounds(&geo_data_bounds);
        let uniforms = ChoroplethUniforms {
            viewport: glam::Vec4::new(
                frame.width as f32,
                frame.height as f32,
                1.0 / frame.width as f32,
                1.0 / frame.height as f32,
            ),
            bounds: glam::Vec4::new(visible_bounds.min_x, visible_bounds.max_x, visible_bounds.min_y, visible_bounds.max_y),
            animation: glam::Vec4::new(
                self.animation.time,
                self.animation.progress,
                self.animation.easing as f32,
                if self.animation.entry_active > 0 { 1.0 } else { 0.0 },
            ),
            pointer: glam::Vec4::new(
                frame.pointer.position.map_or(-1.0, |p| p.x),
                frame.pointer.position.map_or(-1.0, |p| p.y),
                if frame.pointer.hit.is_some() { 1.0 } else { 0.0 },
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
        let mut encoder = frame.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("choropleth_encoder"),
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("choropleth_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                ..Default::default()
            });

            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..self.total_indices, 0..1);
        }

        frame.queue.submit(Some(encoder.finish()));

        // Check if animation is in progress
        self.needs_redraw = self.animation.progress < 1.0;
    }
}

impl ChartRenderer for ChoroplethRenderer {
    type Data = ChoroplethData;
    type DataValue = u32; // Polygon ID

    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
        // Swap current to previous
        if let (Some(curr), Some(prev)) = (&self.vertex_buffer, &self.prev_vertex_buffer) {
            // Copy current vertices to previous buffer
            let (vertices, _) = self.data_to_vertices();
            if !vertices.is_empty() {
                write_storage_buffer(queue, prev, &vertices);
            }
        }

        // Update data
        self.data = data.clone();

        // Calculate bounds
        let geo_bounds = data.bounds();
        self.bounds = DataBounds::new(geo_bounds[0], geo_bounds[2], geo_bounds[1], geo_bounds[3]);

        // Convert to GPU format
        let (vertices, indices) = self.data_to_vertices();
        self.total_indices = indices.len() as u32;

        // Update vertex buffer
        if let Some(buffer) = &self.vertex_buffer {
            if !vertices.is_empty() {
                write_storage_buffer(queue, buffer, &vertices);
            }
        }

        // Update color stops
        if let Some(buffer) = &self.color_stop_buffer {
            let stops = self.color_scale_to_stops();
            if !stops.is_empty() {
                write_storage_buffer(queue, buffer, &stops);
            }
        }

        self.needs_redraw = true;
    }

    fn set_animation(&mut self, animation: &ChartAnimation) {
        self.animation = *animation;
        self.needs_redraw = true;
    }

    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<Self::DataValue>> {
        if self.data.polygons.is_empty() {
            return None;
        }

        let geo_bounds = self.data.bounds();
        let lon_range = geo_bounds[2] - geo_bounds[0];
        let lat_range = geo_bounds[3] - geo_bounds[1];

        if lon_range <= 0.0 || lat_range <= 0.0 {
            return None;
        }

        // Convert screen point to geographic coordinates
        let norm_x = (point.x - viewport.x) / viewport.width;
        let norm_y = (point.y - viewport.y) / viewport.height;

        let lon = geo_bounds[0] + norm_x * lon_range;
        let lat = geo_bounds[3] - norm_y * lat_range; // Y is flipped

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

#[allow(dead_code)]
fn check(
    viewport: glam::Vec4,
    bounds: glam::Vec4,
    animation: glam::Vec4,
    pointer: glam::Vec4,
    value_range: glam::Vec4,
    stroke_color: glam::Vec4,
    pos: glam::Vec2,
    value: f32,
    polygon_id: f32,
    position: f32,
    _pad: [f32; 3],
    color: glam::Vec4,
) {}
