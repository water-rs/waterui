use std::ptr::NonNull;
use std::sync::Arc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use waterui_graphics::gpu::Context as GpuContext;
use wgpu_external_frame::shared_handle::SharedHandleFrame;

use super::presenter::{OwnedFrameMailbox, copy_source_texture};
use crate::{AcceleratedFrameSink, CefPopupRect};

pub(super) struct SinkParts {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mailbox: Arc<OwnedFrameMailbox>,
}

pub(super) fn check_backend(backend: wgpu::Backend) {
    assert_eq!(
        backend,
        wgpu::Backend::Dx12,
        "CEF shared D3D texture composition requires WaterUI's Direct3D 12 backend"
    );
}

pub(super) fn sink_parts(context: &GpuContext<'_>, mailbox: Arc<OwnedFrameMailbox>) -> SinkParts {
    SinkParts {
        device: context.device.clone(),
        queue: context.queue.clone(),
        mailbox,
    }
}

pub(super) fn frame_sink(parts: SinkParts) -> impl AcceleratedFrameSink {
    WindowsFrameSink {
        device: parts.device,
        queue: parts.queue,
        mailbox: parts.mailbox,
    }
}

struct WindowsFrameSink {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mailbox: Arc<OwnedFrameMailbox>,
}

impl AcceleratedFrameSink for WindowsFrameSink {
    fn import(
        &self,
        element: PaintElementType,
        _dirty_rects: &[Rect],
        frame: &AcceleratedPaintInfo,
    ) {
        let handle = NonNull::new(frame.shared_texture_handle)
            .expect("CEF accelerated paint returned a null D3D shared handle");
        let size = &frame.extra.coded_size;
        let width = u32::try_from(size.width).expect("CEF D3D texture width must be positive");
        let height = u32::try_from(size.height).expect("CEF D3D texture height must be positive");
        let format = if frame.format == ColorType::BGRA_8888 {
            wgpu::TextureFormat::Bgra8Unorm
        } else if frame.format == ColorType::RGBA_8888 {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            panic!("CEF returned unsupported Windows accelerated color format")
        };
        // SAFETY: `import` is the accelerated paint callback, so the handle CEF
        // put in the paint info is valid in this process for exactly this call,
        // which is when it is duplicated. The extent and format come out of the
        // same paint info and describe the resource behind it.
        let shared = unsafe { SharedHandleFrame::duplicate(handle, width, height, format) };
        let source = shared.import(&self.device);
        // Only the visible region: `coded_size` may carry alignment padding, and
        // presenting the padded texture edge to edge stretches the page and
        // draws the gutter.
        let visible = &frame.extra.visible_rect;
        let owned = copy_source_texture(
            &self.device,
            &self.queue,
            &source,
            wgpu::Extent3d {
                width: u32::try_from(visible.width).unwrap_or(width).min(width),
                height: u32::try_from(visible.height).unwrap_or(height).min(height),
                depth_or_array_layers: 1,
            },
            format,
        );
        self.mailbox.publish(element, owned);
    }

    fn set_popup_rect(&self, rect: Option<CefPopupRect>) {
        self.mailbox.set_popup_rect(rect);
    }
}
