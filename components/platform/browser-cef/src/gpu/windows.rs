use std::ptr::NonNull;
use std::rc::Rc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};
use wgpu_external_frame::shared_handle::SharedHandleFrame;

use super::presenter::{OwnedFrameMailbox, TexturePresenter, copy_source_texture};
use super::{request_browser_frame, sync_browser_viewport};
use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

struct WindowsFrameSink {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mailbox: Rc<OwnedFrameMailbox>,
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

pub(super) struct CefGpuView {
    page: CefPageHandle,
    mailbox: Rc<OwnedFrameMailbox>,
    presenter: Option<TexturePresenter>,
}

impl CefGpuView {
    pub(super) fn new(page: CefPageHandle) -> Self {
        Self {
            page,
            mailbox: Rc::new(OwnedFrameMailbox::new()),
            presenter: None,
        }
    }
}

impl GpuView for CefGpuView {
    #[expect(
        clippy::future_not_send,
        reason = "CEF, Direct3D 12, and WaterUI view state are confined to the UI thread"
    )]
    async fn setup(&mut self, context: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        assert_eq!(
            context.adapter.get_info().backend,
            wgpu::Backend::Dx12,
            "CEF shared D3D texture composition requires WaterUI's Direct3D 12 backend"
        );
        let redraw = context.redraw_handle.clone();
        self.mailbox
            .set_waker(Rc::new(move || redraw.request_redraw()));
        self.page.set_frame_sink(WindowsFrameSink {
            device: context.device.clone(),
            queue: context.queue.clone(),
            mailbox: Rc::clone(&self.mailbox),
        });
        self.presenter = Some(TexturePresenter::new(context));
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.page.pump();
        request_browser_frame(&self.page, frame);
        let scale = sync_browser_viewport(&self.page, frame);
        let presenter = self
            .presenter
            .as_mut()
            .expect("CEF GPU view rendered before setup");
        if let Some(texture) = self.mailbox.take_view() {
            presenter.set_source(texture);
        }
        if let Some(texture) = self.mailbox.take_popup() {
            presenter.set_popup_source(texture);
        }
        presenter.set_popup_rect(self.mailbox.popup_rect());
        presenter.render(frame, scale);
    }
}
