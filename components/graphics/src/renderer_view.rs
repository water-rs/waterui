use alloc::boxed::Box;

use waterui_core::raw_view;

/// The pixel format of a CPU renderer surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererBufferFormat {
    /// RGBA8888 pixels in native byte order.
    Rgba8888,
}

impl RendererBufferFormat {
    /// Returns the number of bytes per pixel for this format.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8888 => 4,
        }
    }
}

/// A CPU-accessible surface that can be drawn to directly.
pub struct RendererCpuSurface<'a> {
    data: &'a mut [u8],
    /// The width in pixels of the surface.
    pub width: u32,
    /// The height in pixels of the surface.
    pub height: u32,
    /// The number of bytes that separate one row of pixels from the next.
    pub stride: usize,
    /// The pixel format stored in `data`.
    pub format: RendererBufferFormat,
}

impl core::fmt::Debug for RendererCpuSurface<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RendererCpuSurface")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stride", &self.stride)
            .field("format", &self.format)
            .finish()
    }
}

impl<'a> RendererCpuSurface<'a> {
    /// Creates a new CPU surface wrapper around an existing pixel buffer.
    #[must_use]
    pub fn new(
        data: &'a mut [u8],
        width: u32,
        height: u32,
        stride: usize,
        format: RendererBufferFormat,
    ) -> Self {
        Self {
            data,
            width,
            height,
            stride,
            format,
        }
    }

    /// Returns the mutable pixel slice backing this surface.
    #[must_use]
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        self.data
    }

    /// Returns `true` if the surface is tightly packed (no padding between rows).
    #[must_use]
    pub fn is_tightly_packed(&self) -> bool {
        (self.width as usize) * self.format.bytes_per_pixel() == self.stride
    }
}

/// A GPU surface backed by WGPU resources.
#[cfg(feature = "wgpu")]
pub struct RendererWgpuSurface<'a> {
    /// The logical WGPU device used for rendering.
    pub device: &'a wgpu::Device,
    /// The submission queue associated with the device.
    pub queue: &'a wgpu::Queue,
    /// The texture view to render into.
    pub target: &'a wgpu::TextureView,
    /// The surface format of the texture view.
    pub format: wgpu::TextureFormat,
    /// Width of the surface in pixels.
    pub width: u32,
    /// Height of the surface in pixels.
    pub height: u32,
}

#[cfg(feature = "wgpu")]
impl<'a> RendererWgpuSurface<'a> {
    /// Creates a new GPU surface wrapper.
    #[must_use]
    pub fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        target: &'a wgpu::TextureView,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        Self {
            device,
            queue,
            target,
            format,
            width,
            height,
        }
    }
}

/// The surface passed to a renderer callback.
#[non_exhaustive]
pub enum RendererSurface<'a> {
    /// A CPU surface with a mutable pixel buffer.
    Cpu(RendererCpuSurface<'a>),
    /// A GPU surface backed by WGPU resources.
    #[cfg(feature = "wgpu")]
    Wgpu(RendererWgpuSurface<'a>),
}

impl core::fmt::Debug for RendererSurface<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Cpu(surface) => f.debug_tuple("Cpu").field(surface).finish(),
            #[cfg(feature = "wgpu")]
            Self::Wgpu(surface) => f.debug_tuple("Wgpu").field(surface).finish(),
        }
    }
}

/// Callback invoked by the backend when it is time to render a view.
pub type RendererDrawCallback = Box<dyn for<'surface> FnMut(RendererSurface<'surface>) + 'static>;

/// A raw primitive view that exposes a renderer surface to user code.
pub struct RendererView {
    /// Callback invoked by the backend to render into the provided surface.
    pub on_render: RendererDrawCallback,
    /// Desired width of the renderer view.
    pub width: f32,
    /// Desired height of the renderer view.
    pub height: f32,
}

impl core::fmt::Debug for RendererView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RendererView")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

raw_view!(RendererView);

impl RendererView {
    /// Creates a new renderer view with the provided callback.
    #[must_use]
    pub fn new<F>(on_render: F) -> Self
    where
        F: for<'surface> FnMut(RendererSurface<'surface>) + 'static,
    {
        Self {
            on_render: Box::new(on_render),
            width: 100.0,
            height: 100.0,
        }
    }

    /// Sets the preferred width of the view.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the preferred height of the view.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }
}
