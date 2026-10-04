//! CEF's Linux GPU path.
//!
//! Chromium's Linux shared images are DMA-BUFs, imported on the engine's
//! Vulkan device. The pooled destination of the inside-the-callback copy
//! is an engine-owned dma-buf from [`vulkan::Device::alloc_dmabuf`],
//! re-imported once per present; a pooled buffer hosts a new generation
//! only after [`Generation::state`] reports `Released`.

use std::os::fd::{BorrowedFd, OwnedFd};

use cef::{AcceleratedPaintInfo, ColorType};
use waterui_graphics::cherenkov_gpu::interop::vulkan::{
    self, DmaBuf, DmaBufPlane, FrameSource, QueueFamily, State,
};
use waterui_graphics::cherenkov_gpu::interop::{ExternalFrame, FrameColor, RgbAlpha};
use waterui_graphics::gpu::{ExternalFrameView, FrameOutput};
use wgpu_external_frame::dma_buf::DmaBufFormat;

use super::sink::{Backend, external_view};
use crate::CefPageHandle;

/// One engine-owned texture: the allocation's import descriptor — built
/// once by [`vulkan::Device::alloc_dmabuf`] — and the generation the last
/// write left in it.
struct OwnedTarget {
    descriptor: DmaBuf,
    frame: Option<vulkan::Frame>,
}

/// What a pooled target is reused on: the pixel extent, the pixel format
/// and the Vulkan layout class it was allocated in.
#[derive(Clone, Copy, PartialEq, Eq)]
struct TargetKey {
    size: (u32, u32),
    fourcc: u32,
    layout: u32,
}

/// A transient import of one paint's shared DMA-BUF and the pixel format
/// it declared — dropped inside the callback, never retained.
struct LinuxImport {
    frame: vulkan::Frame,
    fourcc: u32,
}

/// The Linux [`Backend`]: imports on the output's Vulkan device.
struct LinuxBackend {
    native: vulkan::Device,
}

impl Backend for LinuxBackend {
    type Key = TargetKey;
    type Import = LinuxImport;
    type Target = OwnedTarget;

    const COPIES: bool = true;

    fn open(output: &FrameOutput) -> Self {
        // Native import needs the engine's Vulkan device: this path is
        // DMA-BUF-only, so the host must run on it — a `FrameOutput` on any
        // other backend cannot take these frames.
        Self {
            native: vulkan::Device::new(output.shared_device())
                .expect("CEF DMA-BUF import requires WaterUI's Vulkan backend"),
        }
    }

    fn import(&self, frame: &AcceleratedPaintInfo) -> Self::Import {
        let fourcc = format_of(frame).fourcc();
        let frame = self
            .native
            .import(FrameSource::DmaBuf(Box::new(dmabuf_of(frame))))
            .expect("CEF DMA-BUF import failed");
        LinuxImport { frame, fourcc }
    }

    fn texture(import: &Self::Import) -> &wgpu::Texture {
        import
            .frame
            .generation
            .rgb_wrap
            .as_ref()
            .expect("a CEF DMA-BUF imports as an RGB frame")
    }

    fn size(import: &Self::Import) -> (u32, u32) {
        import.frame.size()
    }

    fn source_uv(_import: &Self::Import) -> [f32; 4] {
        // The dmabuf import wraps the declared visible extent, so the
        // whole texture is the frame.
        [0.0, 0.0, 1.0, 1.0]
    }

    fn key(import: &Self::Import) -> Self::Key {
        TargetKey {
            size: import.frame.size(),
            fourcc: import.fourcc,
            layout: vulkan::LAYOUT_TRANSFER_DST,
        }
    }

    fn view_key(key: Self::Key, composited: bool) -> Self::Key {
        if composited {
            // A composited target renders BGRA through the blit pipeline
            // and ends the pass in COLOR_ATTACHMENT_OPTIMAL; a plain copy
            // keeps the source's own format and ends in
            // TRANSFER_DST_OPTIMAL.
            TargetKey {
                fourcc: DmaBufFormat::Bgra8.fourcc(),
                layout: vulkan::LAYOUT_COLOR_ATTACHMENT,
                ..key
            }
        } else {
            key
        }
    }

    fn alloc(&self, key: Self::Key) -> Self::Target {
        let descriptor = self
            .native
            .alloc_dmabuf(
                key.size,
                key.fourcc,
                key.layout,
                FrameColor::SRGB,
                RgbAlpha::Premultiplied,
            )
            .expect("CEF pool texture allocation failed");
        OwnedTarget {
            descriptor,
            frame: None,
        }
    }

    fn materialize<'a>(&self, target: &'a mut Self::Target) -> &'a wgpu::Texture {
        let frame = self
            .native
            .import(FrameSource::DmaBuf(Box::new(
                target
                    .descriptor
                    .reopen()
                    .expect("pool DMA-BUF duplication failed"),
            )))
            .expect("CEF pool texture import failed");
        target.frame = Some(frame);
        Self::current(target).expect("a pool DMA-BUF imports as an RGB frame")
    }

    fn current(target: &Self::Target) -> Option<&wgpu::Texture> {
        target
            .frame
            .as_ref()
            .and_then(|frame| frame.generation.rgb_wrap.as_ref())
    }

    fn released(target: &mut Self::Target) -> bool {
        target
            .frame
            .as_ref()
            .is_none_or(|frame| frame.generation.state() == State::Released)
    }

    fn frame(&self, target: &mut Self::Target) -> ExternalFrame {
        ExternalFrame::native(
            target
                .frame
                .as_ref()
                .expect("materialize ran first")
                .clone(),
        )
        .expect("a pool texture frame is a valid external frame")
    }
}

/// The DMA-BUF format a CEF accelerated paint carries.
///
/// # Panics
///
/// Panics on a color type outside the ones Chromium's Linux shared images
/// use.
fn format_of(frame: &AcceleratedPaintInfo) -> DmaBufFormat {
    if frame.format == ColorType::BGRA_8888 {
        DmaBufFormat::Bgra8
    } else if frame.format == ColorType::RGBA_8888 {
        DmaBufFormat::Rgba8
    } else {
        panic!("CEF returned unsupported Linux accelerated color format")
    }
}

/// Builds the transient [`DmaBuf`] descriptor for one accelerated paint —
/// imported and dropped inside the callback, never retained.
///
/// # Panics
///
/// Panics when the paint is not a single packed DMA-BUF plane, or its
/// geometry does not fit a `u32`.
fn dmabuf_of(frame: &AcceleratedPaintInfo) -> DmaBuf {
    assert_eq!(
        frame.plane_count, 1,
        "CEF Linux accelerated paint must provide one packed DMA-BUF plane"
    );
    let plane = &frame.planes[0];
    assert!(plane.fd >= 0, "CEF DMA-BUF file descriptor is invalid");
    // SAFETY: `borrow_raw` requires the descriptor to be open and to stay
    // open for the borrow's lifetime. CEF owns this descriptor and keeps it
    // valid for the duration of the `on_accelerated_paint` callback this
    // runs inside, which is exactly the scope of `borrowed`; it is asserted
    // non-negative just above. The borrow is only used to duplicate the
    // descriptor into an `OwnedFd`, so nothing outlives the callback and
    // CEF's own close is unaffected.
    let borrowed = unsafe { BorrowedFd::borrow_raw(plane.fd) };
    let fd: OwnedFd = borrowed
        .try_clone_to_owned()
        .expect("failed to duplicate CEF DMA-BUF file descriptor");
    let coded = &frame.extra.coded_size;
    let coded_width = u32::try_from(coded.width).expect("CEF DMA-BUF width must be positive");
    let coded_height = u32::try_from(coded.height).expect("CEF DMA-BUF height must be positive");
    // Only the region that holds the page: Chromium may allocate the shared
    // image at a coded size with alignment padding, and copying that edge
    // to edge drew the gutter.
    let visible = &frame.extra.visible_rect;
    let size = match (u32::try_from(visible.width), u32::try_from(visible.height)) {
        (Ok(visible_width), Ok(visible_height))
            if visible_width <= coded_width && visible_height <= coded_height =>
        {
            (visible_width, visible_height)
        }
        _ => (coded_width, coded_height),
    };
    DmaBuf {
        fourcc: format_of(frame).fourcc(),
        modifier: frame.modifier,
        size,
        planes: vec![DmaBufPlane {
            memory: 0,
            offset: u32::try_from(plane.offset).expect("CEF DMA-BUF offset exceeds u32"),
            stride: plane.stride,
        }],
        memory: vec![fd],
        // A shared image whose producer was never a Vulkan queue sits in
        // `VK_IMAGE_LAYOUT_GENERAL`, which is a legal copy-source layout —
        // the import declares `COPY_SRC` up front so the copy below records
        // no transition and CEF's buffer state is left untouched.
        layout: vulkan::LAYOUT_GENERAL,
        producer_family: QueueFamily::External,
        // CEF completes the shared image's writes before
        // `on_accelerated_paint` runs and the buffer returns to CEF's pool
        // when the callback returns, so the frame carries neither a wait
        // nor a release sync.
        sync: None,
        release: None,
        color: FrameColor::SRGB,
        alpha: RgbAlpha::Premultiplied,
        usage: vulkan::DmaBufUsage::CopySource,
    }
}

/// Creates the GPU view for one visible CEF page on Linux: an
/// [`ExternalFrameView`] whose source imports the page's shared DMA-BUF
/// frames on the layer's own device.
pub(super) fn gpu_view(page: CefPageHandle) -> ExternalFrameView {
    external_view::<LinuxBackend>(page)
}
