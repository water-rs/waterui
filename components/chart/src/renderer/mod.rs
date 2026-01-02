//! Chart renderer trait and base utilities.
//!
//! The `ChartRenderer` trait extends `GpuRenderer` with chart-specific functionality
//! for data updates, animation, and hit-testing.

use waterui_core::layout::Point;
use waterui_graphics::{wgpu, GpuRenderer};

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
/// This trait extends `GpuRenderer` with chart-specific methods for:
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
///     fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue) {
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
pub trait ChartRenderer: GpuRenderer {
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
    /// The queue parameter allows immediate buffer writes without waiting
    /// for the next render frame.
    fn update_data(&mut self, data: &Self::Data, queue: &wgpu::Queue);

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
    fn hit_test(&self, point: Point, viewport: &ChartViewport) -> Option<HitResult<Self::DataValue>>;

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

/// Base GPU utilities shared across chart renderers.
pub mod base {
    extern crate alloc;

    use alloc::vec::Vec;
    use alloc::string::String;
    use encase::{ShaderType, StorageBuffer, UniformBuffer};
    use waterui_graphics::{GpuContext, wgpu};

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
    pub fn create_storage_buffer<T: ShaderType + encase::ShaderSize + encase::internal::WriteInto>(
        ctx: &GpuContext,
        label: &str,
        data: &[T],
    ) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        let mut buffer = StorageBuffer::new(Vec::new());
        buffer.write(data).expect("Failed to write storage buffer");
        ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
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
        ctx.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: buffer.as_ref(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    /// Writes data to a storage buffer using encase for proper alignment.
    pub fn write_storage_buffer<T: ShaderType + encase::ShaderSize + encase::internal::WriteInto>(
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        data: &[T],
    ) {
        let mut storage = StorageBuffer::new(Vec::new());
        storage.write(data).expect("Failed to write storage buffer");
        queue.write_buffer(buffer, 0, storage.as_ref());
    }

    /// Writes data to a uniform buffer using encase for proper alignment.
    pub fn write_uniform_buffer<T: ShaderType + encase::internal::WriteInto>(
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        data: &T,
    ) {
        let mut uniform = UniformBuffer::new(Vec::new());
        uniform.write(data).expect("Failed to write uniform buffer");
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
