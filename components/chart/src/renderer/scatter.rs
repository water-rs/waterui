//! Scatter chart GPU renderer.
//!
//! Renders scatter plots using instanced rendering where each point is a circle.
//! Supports 1M+ data points with O(1) hit detection via spatial indexing.

extern crate alloc;

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

// ============================================================================
// Spatial Index for O(1) Hit Detection
// ============================================================================

/// Grid-based spatial index for fast point lookup.
/// Divides the data space into cells, each storing indices of points within.
struct SpatialGrid {
    /// Grid resolution (cells per axis)
    resolution: usize,
    /// Cells storing point indices (flattened 2D array)
    cells: Vec<Vec<usize>>,
    /// Data bounds used to build this grid
    bounds: DataBounds,
}

impl SpatialGrid {
    /// Creates an empty spatial grid.
    fn new() -> Self {
        Self {
            resolution: 0,
            cells: Vec::new(),
            bounds: DataBounds::default(),
        }
    }

    /// Rebuilds the grid from the given points.
    /// Resolution scales with sqrt(point_count) for balanced cell sizes.
    fn rebuild(&mut self, points: &[DataPoint], bounds: &DataBounds) {
        self.bounds = *bounds;

        // Scale resolution with data size: sqrt(n) cells per axis
        // Clamped to reasonable range [8, 256]
        self.resolution = ((points.len() as f32).sqrt() as usize).clamp(8, 256);

        // Clear and resize cells
        let total_cells = self.resolution * self.resolution;
        self.cells.clear();
        self.cells.resize_with(total_cells, Vec::new);

        // Handle degenerate bounds
        let width = (bounds.max_x - bounds.min_x).max(0.001);
        let height = (bounds.max_y - bounds.min_y).max(0.001);

        // Insert points into cells
        for (i, p) in points.iter().enumerate() {
            let cell_x = (((p.x - bounds.min_x) / width) * self.resolution as f32) as usize;
            let cell_y = (((p.y - bounds.min_y) / height) * self.resolution as f32) as usize;

            let cell_x = cell_x.min(self.resolution - 1);
            let cell_y = cell_y.min(self.resolution - 1);

            let cell_idx = cell_y * self.resolution + cell_x;
            self.cells[cell_idx].push(i);
        }
    }

    /// Finds the nearest point within the given radius (in data coordinates).
    /// Returns (point_index, distance) or None if no point found.
    fn find_nearest(
        &self,
        data_x: f32,
        data_y: f32,
        radius: f32,
        points: &[DataPoint],
    ) -> Option<(usize, f32)> {
        if self.resolution == 0 || points.is_empty() {
            return None;
        }

        let width = (self.bounds.max_x - self.bounds.min_x).max(0.001);
        let height = (self.bounds.max_y - self.bounds.min_y).max(0.001);

        // Calculate which cells to search (including neighbors within radius)
        let cell_size_x = width / self.resolution as f32;
        let cell_size_y = height / self.resolution as f32;

        let cells_to_check_x = (radius / cell_size_x).ceil() as i32 + 1;
        let cells_to_check_y = (radius / cell_size_y).ceil() as i32 + 1;

        let center_cell_x =
            (((data_x - self.bounds.min_x) / width) * self.resolution as f32) as i32;
        let center_cell_y =
            (((data_y - self.bounds.min_y) / height) * self.resolution as f32) as i32;

        let mut nearest_idx = None;
        let mut nearest_dist = f32::MAX;

        // Search cells in radius
        for dy in -cells_to_check_y..=cells_to_check_y {
            for dx in -cells_to_check_x..=cells_to_check_x {
                let cx = center_cell_x + dx;
                let cy = center_cell_y + dy;

                if cx < 0 || cy < 0 || cx >= self.resolution as i32 || cy >= self.resolution as i32
                {
                    continue;
                }

                let cell_idx = (cy as usize) * self.resolution + (cx as usize);

                for &point_idx in &self.cells[cell_idx] {
                    let p = &points[point_idx];
                    let dx = p.x - data_x;
                    let dy = p.y - data_y;
                    let dist = (dx * dx + dy * dy).sqrt();

                    if dist <= radius && dist < nearest_dist {
                        nearest_dist = dist;
                        nearest_idx = Some(point_idx);
                    }
                }
            }
        }

        nearest_idx.map(|idx| (idx, nearest_dist))
    }
}

/// GPU-accelerated scatter chart renderer.
pub struct ScatterChartRenderer {
    // Data
    data: Vec<DataPoint>,
    bounds: DataBounds,
    point_color: [f32; 4],
    point_radius: f32,

    // Spatial index for O(1) hit detection
    spatial_index: SpatialGrid,

    // GPU resources
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

/// GPU data point for scatter charts.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct GpuDataPoint {
    x: f32,
    y: f32,
    color: glam::Vec4,
}

const SCATTER_SHADER: &str = include_str!("../shaders/scatter.wgsl");

impl ScatterChartRenderer {
    /// Creates a new scatter chart renderer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bounds: DataBounds::default(),
            point_color: [0.23, 0.51, 0.97, 1.0], // Blue
            point_radius: 4.0,
            spatial_index: SpatialGrid::new(),
            pipeline: None,
            uniform_buffer: None,
            current_buffer: None,
            previous_buffer: None,
            bind_group: None,
            msaa_target: None,
            msaa_samples: 1,
            animation: ChartAnimation::new(),
            needs_redraw: true,
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

    /// Sets the data points.
    pub fn set_data(&mut self, data: Vec<DataPoint>) {
        self.data = data;
        self.bounds = DataBounds::from_points(&self.data);
        self.spatial_index.rebuild(&self.data, &self.bounds);
        self.needs_redraw = true;
    }

    /// Sets the point color.
    pub fn set_color(&mut self, color: Srgb) {
        self.point_color = [color.red, color.green, color.blue, 1.0];
        self.needs_redraw = true;
    }

    /// Sets the point radius in pixels.
    pub fn set_radius(&mut self, radius: f32) {
        assert!(
            radius.is_finite() && radius > 0.0,
            "Scatter point radius must be > 0"
        );
        self.point_radius = radius;
        self.needs_redraw = true;
    }

    /// Creates the render pipeline.
    fn create_pipeline(ctx: &GpuContext) -> wgpu::RenderPipeline {
        // Charts output premultiplied alpha from shaders (including SDF-based edge AA),
        // so blending must stay enabled even on HDR surfaces.
        let blend = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let shader_source = shader_with_common(SCATTER_SHADER);
        let shader = waterui_graphics::shared_context::create_cached_shader_module(
            ctx.device,
            "Scatter Chart Shader",
            &shader_source,
        );

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Scatter Chart Bind Group Layout"),
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
                label: Some("Scatter Chart Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

        ctx.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Scatter Chart Pipeline"),
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
                color: glam::Vec4::from_array(self.point_color),
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
            label: Some("Scatter Chart Bind Group"),
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

impl Default for ScatterChartRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl GpuView for ScatterChartRenderer {
    fn setup(&mut self, ctx: &GpuContext, _env: &mut waterui_core::Environment) -> impl Future<Output = ()> {
        self.msaa_samples = ctx.msaa_samples;
        // Create pipeline
        self.pipeline = Some(Self::create_pipeline(ctx));

        // Create uniform buffer
        let uniforms = ChartUniforms::default();
        self.uniform_buffer = Some(create_uniform_buffer(
            ctx,
            "Scatter Chart Uniforms",
            &uniforms,
        ));

        // Create data buffers
        let initial_capacity = self.data.len().max(16384);
        let initial_data = self.to_gpu_data(&self.data);

        self.current_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Scatter Chart Current Data",
                &vec![GpuDataPoint::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Scatter Chart Current Data", &initial_data)
        });

        self.previous_buffer = Some(if initial_data.is_empty() {
            create_storage_buffer(
                ctx,
                "Scatter Chart Previous Data",
                &vec![GpuDataPoint::default(); initial_capacity],
            )
        } else {
            create_storage_buffer(ctx, "Scatter Chart Previous Data", &initial_data)
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
                        label: Some("Scatter Chart Clear Encoder"),
                    });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Scatter Chart Clear Pass"),
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

        // Update uniforms with point radius in the pointer.w field (reusing)
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
                        self.point_radius,
                    )
                } else {
                    glam::Vec4::new(-1.0, -1.0, 0.0, self.point_radius)
                }
            } else {
                glam::Vec4::new(-1.0, -1.0, 0.0, self.point_radius)
            },
        };
        write_uniform_buffer(
            frame.queue,
            self.uniform_buffer.as_ref().unwrap(),
            &uniforms,
        );

        // Render points
        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Scatter Chart Encoder"),
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
                label: Some("Scatter Chart Render Pass"),
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

            // Draw 6 vertices per point (quad), instanced for each point
            pass.draw(0..6, 0..self.data.len() as u32);
        }

        frame.queue.submit([encoder.finish()]);

        self.needs_redraw = !self.animation.is_complete();
    }
}

impl ChartRenderer for ScatterChartRenderer {
    type Data = Vec<DataPoint>;
    type DataValue = DataPoint;

    fn update_data(&mut self, data: &Self::Data, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Swap buffers for animation interpolation
        core::mem::swap(&mut self.current_buffer, &mut self.previous_buffer);

        let previous_data = core::mem::replace(&mut self.data, data.clone());
        self.bounds = DataBounds::from_points(&self.data);
        self.spatial_index.rebuild(&self.data, &self.bounds);

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
                "Scatter Chart Current Data",
                &current_gpu_data,
            );
        }
        if let Some(buffer) = self.previous_buffer.as_mut() {
            needs_rebind |= write_storage_buffer_with_growth(
                device,
                queue,
                buffer,
                "Scatter Chart Previous Data",
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
        self.needs_redraw = true;
    }

    fn hit_test(
        &self,
        point: Point,
        viewport: &ChartViewport,
    ) -> Option<HitResult<Self::DataValue>> {
        if self.data.is_empty() {
            return None;
        }

        let (chart_x, chart_y) = super::chart_coords_from_viewport(viewport, point, PLOT_PADDING)?;
        let chart_y = 1.0 - chart_y;

        let visible_bounds = self.zoom_pan.transform_bounds(&self.bounds);
        if visible_bounds.width() <= 0.0 || visible_bounds.height() <= 0.0 {
            return None;
        }

        let data_x = visible_bounds.min_x + chart_x * visible_bounds.width();
        let data_y = visible_bounds.min_y + chart_y * visible_bounds.height();

        // Find nearest point within radius using spatial index (O(1) average case)
        let denom = (1.0 - 2.0 * PLOT_PADDING).max(0.001);
        let radius_data =
            self.point_radius * 2.0 * visible_bounds.width() / (viewport.width * denom);

        self.spatial_index
            .find_nearest(data_x, data_y, radius_data, &self.data)
            .map(|(idx, _dist)| {
                let p = self.data[idx];
                HitResult::new(0, idx, p, point)
            })
    }

    fn data_bounds(&self) -> DataBounds {
        self.bounds
    }

    fn data_count(&self) -> usize {
        self.data.len()
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw || !self.animation.is_complete()
    }
}
