//! Chart renderer trait and base utilities.
//!
//! The `ChartRenderer` trait extends `GpuView` with chart-specific functionality
//! for data updates, animation, and hit-testing.

use waterui_core::layout::Point;
use waterui_graphics::{GpuView, wgpu};

use crate::animation::ChartAnimation;
use crate::data::DataBounds;
use crate::interaction::{ChartViewport, HitResult};

pub mod area;
pub mod bar;
pub mod bubble;
pub mod candlestick;
pub mod choropleth;
pub mod contour;
pub mod depth;
pub mod gauge;
pub mod heatmap;
pub mod line;
pub mod pie;
pub mod radar;
pub mod scatter;

pub use area::AreaRenderer;
pub use bar::BarChartRenderer;
pub use bubble::BubbleRenderer;
pub use candlestick::CandlestickRenderer;
pub use choropleth::ChoroplethRenderer;
pub use contour::ContourRenderer;
pub use depth::DepthRenderer;
pub use gauge::GaugeRenderer;
pub use heatmap::HeatmapRenderer;
pub use line::LineChartRenderer;
pub use pie::PieChartRenderer;
pub use radar::RadarRenderer;
pub use scatter::ScatterChartRenderer;

/// Extended GPU renderer for chart components.
///
/// This trait extends `GpuView` with chart-specific methods for:
/// - Updating data with GPU buffer synchronization
/// - Animation state management
/// - Hit-testing for interactivity
/// - Data bounds for axis scaling
///
/// # Double-Buffer Architecture
///
/// Chart renderers maintain two data buffers for smooth interpolation:
/// - `current_buffer`: Current target data
/// - `previous_buffer`: Snapshot before animation started
///
/// When data changes:
/// 1. Copy current -> previous
/// 2. Upload new data -> current
/// 3. Start animation
///
/// The shader then interpolates: `value = mix(prev, curr, progress)`
///
/// # Example
///
/// ```ignore
/// impl ChartRenderer for BarChartRenderer {
///     type Data = Vec<DataPoint>;
///     type DataValue = DataPoint;
///
///     fn update_data(
///         &mut self,
///         data: &Self::Data,
///         _device: &wgpu::Device,
///         queue: &wgpu::Queue,
///     ) {
///         // Swap buffers
///         std::mem::swap(&mut self.current_buffer, &mut self.previous_buffer);
///         // Upload new data
///         queue.write_buffer(&self.current_buffer, 0, bytemuck::cast_slice(&data));
///     }
///
///     fn set_animation(&mut self, animation: &ChartAnimation) {
///         self.animation_uniform = *animation;
///     }
///
///     fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<DataPoint>> {
///         // Check if point is within chart area, calculate which bar was hit
///     }
///
///     fn data_bounds(&self) -> DataBounds {
///         self.bounds
///     }
/// }
/// ```
pub trait ChartRenderer: GpuView {
    /// The input data type for this chart.
    type Data;

    /// The data value type returned from hit-testing.
    type DataValue: Clone;

    /// Updates the chart data with GPU buffer synchronization.
    ///
    /// This method should:
    /// 1. Swap current and previous buffers (for animation interpolation)
    /// 2. Upload new data to the current buffer
    /// 3. Update internal data bounds
    ///
    /// The `device` parameter allows renderers to recreate GPU buffers when
    /// incoming data exceeds current capacity.
    ///
    /// The `queue` parameter allows immediate buffer writes without waiting
    /// for the next render frame.
    fn update_data(&mut self, data: &Self::Data, _device: &wgpu::Device, queue: &wgpu::Queue);

    /// Sets the current animation state.
    ///
    /// This state is passed to shaders for interpolation between
    /// previous and current data states.
    fn set_animation(&mut self, animation: &ChartAnimation);

    /// Performs hit-testing at the given screen position.
    ///
    /// Returns `Some(HitResult)` if the point intersects with a data element,
    /// `None` otherwise.
    ///
    /// Hit-testing is performed on CPU using the same coordinate math as
    /// the shader, since all elements are rendered in a single draw call.
    fn hit_test(
        &self,
        point: Point,
        viewport: &ChartViewport,
    ) -> Option<HitResult<Self::DataValue>>;

    /// Returns the current data bounds.
    ///
    /// Used for axis scaling and coordinate transformation.
    fn data_bounds(&self) -> DataBounds;

    /// Returns the number of data points being rendered.
    ///
    /// Used for determining hit test granularity and animation complexity.
    fn data_count(&self) -> usize {
        0
    }

    /// Returns true if the chart needs a redraw (animation in progress).
    fn needs_redraw(&self) -> bool {
        false
    }
}

fn unpad_normalized(value: f32, padding: f32) -> Option<f32> {
    let denom = 1.0 - 2.0 * padding;
    if denom <= 0.0 {
        return None;
    }
    let unpadded = (value - padding) / denom;
    if (0.0..=1.0).contains(&unpadded) {
        Some(unpadded)
    } else {
        None
    }
}

fn unpad_normalized_point(x: f32, y: f32, padding: f32) -> Option<(f32, f32)> {
    let x = unpad_normalized(x, padding)?;
    let y = unpad_normalized(y, padding)?;
    Some((x, y))
}

fn chart_coords_from_viewport(
    viewport: &ChartViewport,
    point: Point,
    padding: f32,
) -> Option<(f32, f32)> {
    let (norm_x, norm_y) = viewport.screen_to_normalized(point)?;
    let x = unpad_normalized(norm_x, padding)?;
    let y = unpad_normalized(norm_y, padding)?;
    Some((x, y))
}

/// Base GPU utilities shared across chart renderers.
pub mod base {
    extern crate alloc;

    use alloc::string::String;
    use alloc::vec::Vec;
    use encase::{ShaderType, StorageBuffer, UniformBuffer};
    use waterui_graphics::{GpuContext, wgpu};

    /// Max MSAA sample count for chart renderers.
    pub const MSAA_MAX_SAMPLES: u32 = 4;

    /// MSAA render target for resolving into the surface view.
    #[derive(Debug)]
    pub struct MsaaTarget {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        sample_count: u32,
    }

    impl MsaaTarget {
        fn new(
            device: &wgpu::Device,
            format: wgpu::TextureFormat,
            width: u32,
            height: u32,
            sample_count: u32,
        ) -> Self {
            let width = width.max(1);
            let height = height.max(1);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("chart_msaa_target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            Self {
                texture,
                view,
                width,
                height,
                format,
                sample_count,
            }
        }
    }

    /// Returns the color attachment view and optional resolve target for MSAA.
    #[must_use]
    pub fn msaa_attachment<'a>(
        target: &'a mut Option<MsaaTarget>,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
        resolve: &'a wgpu::TextureView,
        sample_count: u32,
    ) -> (&'a wgpu::TextureView, Option<&'a wgpu::TextureView>) {
        let sample_count = sample_count.clamp(1, MSAA_MAX_SAMPLES);
        if sample_count <= 1 {
            return (resolve, None);
        }

        let needs_rebuild = target.as_ref().map_or(true, |msaa| {
            msaa.width != width
                || msaa.height != height
                || msaa.format != format
                || msaa.sample_count != sample_count
        });
        if needs_rebuild {
            *target = Some(MsaaTarget::new(device, format, width, height, sample_count));
        }

        let view = &target.as_ref().expect("MSAA target missing").view;
        (view, Some(resolve))
    }

    /// Multisample state for chart pipelines.
    #[must_use]
    pub fn multisample_state(sample_count: u32) -> wgpu::MultisampleState {
        wgpu::MultisampleState {
            count: sample_count.clamp(1, MSAA_MAX_SAMPLES),
            mask: !0,
            alpha_to_coverage_enabled: false,
        }
    }

    /// Common WGSL shader utilities (SDF, easing, color).
    /// Prepended to all chart shaders at compile time.
    const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");

    /// Combines common utilities with a chart-specific shader.
    /// Returns a shader source ready for wgpu.
    #[must_use]
    pub fn shader_with_common(main_shader: &str) -> String {
        format!("{COMMON_WGSL}\n\n// === Main Shader ===\n\n{main_shader}")
    }

    /// Creates a storage buffer with the given data using encase for proper alignment.
    #[must_use]
    pub fn create_storage_buffer<
        T: ShaderType + encase::ShaderSize + encase::internal::WriteInto,
    >(
        ctx: &GpuContext,
        label: &str,
        data: &[T],
    ) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let mut buffer = StorageBuffer::new(Vec::new());
        buffer.write(data).expect("Failed to write storage buffer");
        ctx.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: buffer.as_ref(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
    }

    /// Creates an empty storage buffer with the given capacity.
    #[must_use]
    pub fn create_empty_storage_buffer<T: ShaderType + encase::ShaderSize>(
        ctx: &GpuContext,
        label: &str,
        capacity: usize,
    ) -> wgpu::Buffer {
        // Calculate size with proper WGSL alignment
        let element_size = T::SHADER_SIZE.get() as usize;
        ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (capacity * element_size) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Creates a uniform buffer for animation/transform data using encase.
    #[must_use]
    pub fn create_uniform_buffer<T: ShaderType + encase::internal::WriteInto>(
        ctx: &GpuContext,
        label: &str,
        data: &T,
    ) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let mut buffer = UniformBuffer::new(Vec::new());
        buffer.write(data).expect("Failed to write uniform buffer");
        ctx.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: buffer.as_ref(),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
    }

    /// Writes data to a storage buffer using encase for proper alignment.
    pub fn write_storage_buffer<
        T: ShaderType + encase::ShaderSize + encase::internal::WriteInto,
    >(
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        data: &[T],
    ) {
        let mut storage = StorageBuffer::new(Vec::new());
        storage.write(data).expect("Failed to write storage buffer");
        let bytes = storage.as_ref().len() as u64;
        let capacity = buffer.size();
        assert!(
            bytes <= capacity,
            "Chart renderer attempted to write {bytes} bytes into a {capacity}-byte storage buffer"
        );
        queue.write_buffer(buffer, 0, storage.as_ref());
    }

    /// Writes data to a storage buffer and grows it when capacity is insufficient.
    ///
    /// Returns `true` when the buffer handle was replaced (bind groups must be rebuilt),
    /// otherwise writes in-place and returns `false`.
    #[must_use]
    pub fn write_storage_buffer_with_growth<
        T: ShaderType + encase::ShaderSize + encase::internal::WriteInto,
    >(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &mut wgpu::Buffer,
        label: &str,
        data: &[T],
    ) -> bool {
        use wgpu::util::DeviceExt;

        let mut storage = StorageBuffer::new(Vec::new());
        storage.write(data).expect("Failed to write storage buffer");
        let required_bytes = storage.as_ref();
        if (required_bytes.len() as u64) > buffer.size() {
            *buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: required_bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            true
        } else {
            queue.write_buffer(buffer, 0, required_bytes);
            false
        }
    }

    /// Writes data to a uniform buffer using encase for proper alignment.
    pub fn write_uniform_buffer<T: ShaderType + encase::internal::WriteInto>(
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        data: &T,
    ) {
        let mut uniform = UniformBuffer::new(Vec::new());
        uniform.write(data).expect("Failed to write uniform buffer");
        let bytes = uniform.as_ref().len() as u64;
        let capacity = buffer.size();
        assert!(
            bytes <= capacity,
            "Chart renderer attempted to write {bytes} bytes into a {capacity}-byte uniform buffer"
        );
        queue.write_buffer(buffer, 0, uniform.as_ref());
    }

    /// Standard uniform struct for chart shaders.
    /// Uses encase for automatic WGSL-compatible alignment.
    #[derive(Debug, Clone, Copy, Default, ShaderType)]
    pub struct ChartUniforms {
        /// Viewport dimensions [width, height, 1/width, 1/height].
        pub viewport: glam::Vec4,
        /// Data bounds [min_x, max_x, min_y, max_y].
        pub bounds: glam::Vec4,
        /// Animation state [time, progress, easing, entry_active].
        pub animation: glam::Vec4,
        /// Pointer state [x, y, pressed, 0].
        pub pointer: glam::Vec4,
    }
}
