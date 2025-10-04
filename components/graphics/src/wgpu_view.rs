use waterui_core::raw_view;
use wgpu::{Device, Queue, TextureView};

/// The drawing callback that performs WGPU rendering.
pub type WgpuDrawCallback = Box<dyn Fn(&Device, &Queue, &TextureView, wgpu::TextureFormat)>;

/// A raw, primitive view that provides a bridge for custom WGPU rendering.
///
/// This is the fundamental primitive for all custom-drawn content in `WaterUI`.
/// It provides a closure with a WGPU context (`Device`, `Queue`, `TextureView`)
/// and relies on the backend to display the resulting texture.
///
/// This view itself is an opaque primitive; the backend must have specific logic
/// to handle it, create a shareable texture, and execute the drawing callback.
pub struct WgpuView {
    /// The closure that performs the WGPU drawing operations.
    pub on_draw: WgpuDrawCallback,
    /// The desired width of the view.
    pub width: f32,
    /// The desired height of the view.
    pub height: f32,
}

// Implement Debug manually because closures don't implement it.
impl std::fmt::Debug for WgpuView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgpuView")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

// Register WgpuView as a raw, primitive view.
raw_view!(WgpuView);

impl WgpuView {
    /// Creates a new `WgpuView` with a drawing closure.
    pub fn new(
        on_draw: impl Fn(&Device, &Queue, &TextureView, wgpu::TextureFormat) + 'static,
    ) -> Self {
        Self {
            on_draw: Box::new(on_draw),
            width: 100.0,  // Default width
            height: 100.0, // Default height
        }
    }

    /// Sets the view width.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the view height.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}
