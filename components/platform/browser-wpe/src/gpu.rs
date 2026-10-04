//! WPE's GPU path.
//!
//! The engine hands each frame over as a DMA-BUF from its own buffer pool:
//! a rendering fence says when the producer's writes are done, and a
//! release fence the engine exports says when the buffer may be reused.
//! The [`ExternalFrameSource`] below imports each plane in place on the
//! engine's Vulkan device — nothing copies it — and publishes the frame
//! through [`FrameOutput`], where it becomes the content of the view's own
//! engine layer.

use std::collections::VecDeque;
use std::rc::Rc;

use num_traits::ToPrimitive as _;
use waterui_graphics::cherenkov_gpu::interop::vulkan::{
    self, DmaBuf, FrameSource, QueueFamily, ReleaseSync, Wait,
};
use waterui_graphics::cherenkov_gpu::interop::{ExternalFrame, FrameColor, RgbAlpha};
use waterui_graphics::gpu::{ExternalFrameSource, ExternalFrameView, FrameOutput};
use wgpu_external_frame::dma_buf::DmaBufFrame;

#[cfg(feature = "webview")]
use crate::WpePage;
#[cfg(feature = "webview")]
use crate::input::{WpeInputGpuView, WpeSurfaceInput};

/// Source of Linux browser frames for GPU-only DMA-BUF composition.
pub trait DmaBufFrameSource: 'static {
    /// Drains engine work that is ready on the current thread.
    fn pump(&self);
    /// Updates the browser viewport.
    fn resize(&self, width: u32, height: u32, scale: f64);
    /// Installs the host redraw callback.
    fn set_frame_waker(&self, waker: Rc<dyn Fn()>);
    /// Takes the newest available frame.
    fn take_frame(&self) -> Option<DmaBufFrame>;
}

#[cfg(feature = "webview")]
impl DmaBufFrameSource for WpePage {
    fn pump(&self) {
        Self::pump(self);
    }

    fn resize(&self, width: u32, height: u32, scale: f64) {
        Self::resize(self, width, height, scale);
    }

    fn set_frame_waker(&self, waker: Rc<dyn Fn()>) {
        Self::set_frame_waker(self, move || waker());
    }

    fn take_frame(&self) -> Option<DmaBufFrame> {
        Self::take_frame(self)
    }
}

/// One browser page's [`ExternalFrameSource`].
///
/// `WpePage` and any other [`DmaBufFrameSource`] are confined to the UI
/// thread, so all of it lives here: `frame` pumps the engine, keeps its
/// logical viewport in step with the layer's presented extent, and imports
/// and publishes whatever the engine produced. Between ticks it completes
/// the buffer-lease protocol: when the engine retires a frame it signals
/// the `SYNC_FD` the producer waits on before reusing the buffer.
struct WpeExternalSource<S> {
    source: S,
    output: Option<FrameOutput>,
    native: Option<vulkan::Device>,
    /// Imported generations whose engine-side release is still in flight,
    /// paired with the producer frame whose lease the release fence closes.
    /// Holding the `Frame` here does not pin the generation: it only keeps
    /// the handle alive so `release_fd` can hand the fence over later.
    pending_release: VecDeque<(vulkan::Frame, DmaBufFrame)>,
}

impl<S: DmaBufFrameSource> WpeExternalSource<S> {
    /// Imports one DMA-BUF frame and publishes it through the output.
    ///
    /// # Panics
    ///
    /// Panics when the frame's DMA-BUF cannot be imported on the engine's
    /// device — the descriptor came from the engine's own pool, so a
    /// rejection is a contract violation, not a recoverable condition.
    fn import_frame(&mut self, output: &FrameOutput, mut frame: DmaBufFrame) {
        let native = self
            .native
            .as_ref()
            .expect("WPE source imports only after start");
        let imported = native
            .import(FrameSource::DmaBuf(Box::new(dmabuf_of(&mut frame))))
            .expect("WPE DMA-BUF import failed");
        // The backend has taken the frame; the buffer itself returns to the
        // producer only once the engine signals the release fence.
        frame.presented();
        let external = ExternalFrame::native(imported.clone())
            .expect("an imported WPE frame is a valid external frame");
        if output.present(external).is_ok() {
            self.pending_release.push_back((imported, frame));
        }
        // `RetiredOutput` is the documented stop signal: the frame drops,
        // which retires the generation and returns the buffer to WPE
        // without a fence — the host is gone and nothing waits on it.
    }

    /// Completes any release fences that are ready.
    ///
    /// # Panics
    ///
    /// Panics when the engine's release submission was accepted without the
    /// fence export the frame was imported with.
    fn drain_released(&mut self) {
        let Some(front) = self.pending_release.pop_front() else {
            return;
        };
        let mut rest = VecDeque::new();
        rest.push_back(front);
        rest.append(&mut self.pending_release);
        while let Some((imported, frame)) = rest.pop_front() {
            match imported.release_fd() {
                Ok(fd) => frame.release(Some(fd)),
                Err(vulkan::NativeError::Unready) => {
                    self.pending_release.push_back((imported, frame));
                }
                Err(err) => panic!("WPE frame release fence failed: {err}"),
            }
        }
    }
}

impl<S: DmaBufFrameSource> ExternalFrameSource for WpeExternalSource<S> {
    fn start(&mut self, output: FrameOutput) {
        // Native import needs the engine's Vulkan device: this path is
        // DMA-BUF-only, so the host must run on it — a `FrameOutput` on any
        // other backend cannot take these frames.
        self.native = Some(
            vulkan::Device::new(output.shared_device())
                .expect("WPE DMA-BUF import requires WaterUI's Vulkan backend"),
        );
        let waker = output.clone();
        self.source
            .set_frame_waker(Rc::new(move || waker.request_redraw()));
        self.output = Some(output);
    }

    /// Runs one UI-side browser frame: pump, viewport sync, import, publish.
    ///
    /// # Panics
    ///
    /// Panics when the logical viewport does not fit a `u32`.
    fn frame(&mut self) {
        self.source.pump();
        self.drain_released();
        let Some(output) = self.output.clone() else {
            return;
        };
        if output.is_retired() {
            return;
        }
        let (width, height, scale) = output.presented_size();
        if width > 0 && height > 0 {
            let scale = f64::from(scale);
            let logical_width = (f64::from(width) / scale)
                .round()
                .max(1.0)
                .to_u32()
                .expect("WPE logical width exceeds u32");
            let logical_height = (f64::from(height) / scale)
                .round()
                .max(1.0)
                .to_u32()
                .expect("WPE logical height exceeds u32");
            self.source.resize(logical_width, logical_height, scale);
        }
        while let Some(frame) = self.source.take_frame() {
            self.import_frame(&output, frame);
        }
    }
}

/// Builds the [`DmaBuf`] descriptor for one WPE frame, taking over its
/// descriptors; the frame keeps only its lease afterwards.
fn dmabuf_of(frame: &mut DmaBufFrame) -> DmaBuf {
    let plane = frame
        .planes
        .drain(..)
        .next()
        .expect("WPE frames carry exactly one packed plane");
    DmaBuf {
        fourcc: frame.format.fourcc(),
        modifier: frame.modifier,
        size: frame.visible_size(),
        planes: vec![vulkan::DmaBufPlane {
            memory: 0,
            offset: plane.offset,
            stride: plane.stride,
        }],
        memory: vec![plane.fd],
        // The engine's buffer is not a Vulkan image on the producer side;
        // it sits in `VK_IMAGE_LAYOUT_GENERAL` for the whole handoff.
        layout: vulkan::LAYOUT_GENERAL,
        producer_family: QueueFamily::External,
        // WPE's rendering fence is the GPU-side wait ordering the first
        // read behind the engine's writes; the frame returns to WPE on the
        // fence the engine exports when the generation retires.
        sync: frame.rendering_fence.take().map(|fd| Wait::SyncFd { fd }),
        release: Some(ReleaseSync::FenceFd),
        color: FrameColor::SRGB,
        alpha: if frame.format.force_opaque() {
            RgbAlpha::Opaque
        } else {
            RgbAlpha::Premultiplied
        },
        usage: vulkan::DmaBufUsage::Sampled,
    }
}

/// GPU view that composites a Linux browser DMA-BUF stream without CPU
/// readback.
///
/// The source stays on the UI thread — [`DmaBufFrameSource`] implementations
/// are browser engine objects — so the produced [`ExternalFrameView`]'s
/// source drives it on its per-frame tick; only the frames themselves are
/// handed to the host.
pub struct DmaBufGpuView<S> {
    source: S,
}

/// WPE-specialized DMA-BUF GPU view.
#[cfg(feature = "webview")]
pub type WpeGpuView = DmaBufGpuView<WpePage>;

impl<S> core::fmt::Debug for DmaBufGpuView<S> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("WpeGpuView").finish_non_exhaustive()
    }
}

impl<S: DmaBufFrameSource> DmaBufGpuView<S> {
    /// Creates a view over `source`'s frame stream.
    ///
    /// The device scale comes from the extent the host reports through
    /// [`FrameOutput::presented_size`], so nothing has to publish it
    /// separately.
    #[must_use]
    pub const fn new(source: S) -> Self {
        Self { source }
    }

    /// Returns the frame source.
    #[must_use]
    pub const fn source(&self) -> &S {
        &self.source
    }

    /// Builds the [`ExternalFrameView`] that presents the frame stream.
    #[must_use]
    pub fn into_view(self) -> ExternalFrameView {
        ExternalFrameView::new(WpeExternalSource {
            source: self.source,
            output: None,
            native: None,
            pending_release: VecDeque::new(),
        })
    }
}

/// Creates the presenter for one visible WPE page, wired to take its own
/// input.
///
/// The view answers
/// [`wants_input_events`](ExternalFrameView::wants_input_events), so a
/// backend that routes surface input to GPU views needs nothing
/// WPE-specific: the pointer, keyboard, scroll and composition events
/// landing on this layer reach `WPEPlatform` through
/// [`WpeSurfaceInput`](crate::WpeSurfaceInput). A backend whose input arrives
/// somewhere else entirely — GTK delivers it to the `GtkGLArea`'s event
/// controllers — builds a [`DmaBufGpuView`] and owns a `WpeSurfaceInput`
/// beside it instead.
#[cfg(feature = "webview")]
#[must_use]
pub fn gpu_view_with_input(page: WpePage) -> ExternalFrameView {
    WpeInputGpuView::new(
        DmaBufGpuView::new(page.clone()).into_view(),
        WpeSurfaceInput::new(page),
    )
    .into_view()
}
