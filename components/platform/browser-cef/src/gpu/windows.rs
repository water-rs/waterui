use std::cell::RefCell;
use std::ptr::NonNull;
use std::sync::Arc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use waterui_graphics::gpu::GpuContentView;
use wgpu_external_frame::shared_handle::SharedHandleFrame;

use super::presenter::{GpuHandles, OwnedFrameMailbox, copy_source_texture};
use super::{CefGpuContent, CefUiBridge};
use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

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

fn make_frame_sink(handles: GpuHandles, mailbox: Arc<OwnedFrameMailbox>) -> WindowsFrameSink {
    assert_eq!(
        handles.adapter.get_info().backend,
        wgpu::Backend::Dx12,
        "CEF shared D3D texture composition requires WaterUI's Direct3D 12 backend"
    );
    WindowsFrameSink {
        device: handles.device,
        queue: handles.queue,
        mailbox,
    }
}

pub(super) fn gpu_view(page: CefPageHandle) -> GpuContentView {
    let (content, shared) = CefGpuContent::new();
    let bridge = RefCell::new(CefUiBridge::new(page, shared, make_frame_sink));
    // No pump here. Chromium's message loop belongs to
    // `CefRuntime::start_message_pump`, which Chromium itself paces; running
    // `do_message_loop_work` inside the render callback put whatever the
    // browser had queued — parsing, script, compositing — on the main thread
    // inside one frame's budget, which is what tripped the stall probe every
    // few seconds on an idle page. The UI hook installs the sink and requests
    // the next compositor frame and nothing else.
    GpuContentView::new(content).on_frame(move || bridge.borrow_mut().frame())
}
