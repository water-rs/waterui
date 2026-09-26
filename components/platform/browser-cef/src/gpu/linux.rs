use std::os::fd::{BorrowedFd, OwnedFd};
use std::sync::Arc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use waterui_graphics::gpu::Context as GpuContext;
use wgpu_external_frame::dma_buf::{DmaBufFormat, DmaBufFrame, DmaBufImporter, DmaBufPlane};

use super::presenter::OwnedFrameMailbox;
use crate::{AcceleratedFrameSink, CefPopupRect};

pub(super) struct SinkParts {
    importer: DmaBufImporter,
    mailbox: Arc<OwnedFrameMailbox>,
}

pub(super) fn check_backend(backend: wgpu::Backend) {
    assert!(
        matches!(backend, wgpu::Backend::Vulkan | wgpu::Backend::Gl),
        "CEF DMA-BUF composition requires WaterUI's Vulkan or EGL/GLES backend"
    );
}

pub(super) fn sink_parts(context: &GpuContext<'_>, mailbox: Arc<OwnedFrameMailbox>) -> SinkParts {
    SinkParts {
        importer: DmaBufImporter::new(context.device, context.queue, context.adapter),
        mailbox,
    }
}

pub(super) fn frame_sink(parts: SinkParts) -> impl AcceleratedFrameSink {
    LinuxFrameSink {
        importer: parts.importer,
        mailbox: parts.mailbox,
    }
}

struct LinuxFrameSink {
    importer: DmaBufImporter,
    mailbox: Arc<OwnedFrameMailbox>,
}

impl AcceleratedFrameSink for LinuxFrameSink {
    fn import(
        &self,
        element: PaintElementType,
        _dirty_rects: &[Rect],
        frame: &AcceleratedPaintInfo,
    ) {
        assert_eq!(
            frame.plane_count, 1,
            "CEF Linux accelerated paint must provide one packed DMA-BUF plane"
        );
        let size = &frame.extra.coded_size;
        let width = u32::try_from(size.width).expect("CEF DMA-BUF width must be positive");
        let height = u32::try_from(size.height).expect("CEF DMA-BUF height must be positive");
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
        let format = if frame.format == ColorType::BGRA_8888 {
            DmaBufFormat::Bgra8
        } else if frame.format == ColorType::RGBA_8888 {
            DmaBufFormat::Rgba8
        } else {
            panic!("CEF returned unsupported Linux accelerated color format")
        };
        let borrowed_frame = DmaBufFrame::new(
            width,
            height,
            format,
            frame.modifier,
            vec![DmaBufPlane {
                fd,
                offset: u32::try_from(plane.offset).expect("CEF DMA-BUF offset exceeds u32"),
                stride: plane.stride,
            }],
            None,
        );
        // Only the region that holds the page: Chromium may allocate the shared
        // image at a coded size with alignment padding, and presenting that edge
        // to edge stretches the page and draws the gutter.
        let visible = &frame.extra.visible_rect;
        let borrowed_frame = match (u32::try_from(visible.width), u32::try_from(visible.height)) {
            (Ok(visible_width), Ok(visible_height))
                if visible_width <= width && visible_height <= height =>
            {
                borrowed_frame.with_visible_size(visible_width, visible_height)
            }
            _ => borrowed_frame,
        };
        self.mailbox
            .publish(element, self.importer.copy_to_texture(borrowed_frame));
    }

    fn set_popup_rect(&self, rect: Option<CefPopupRect>) {
        self.mailbox.set_popup_rect(rect);
    }
}
