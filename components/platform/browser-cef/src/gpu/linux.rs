use std::os::fd::{BorrowedFd, OwnedFd};
use std::rc::Rc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use num_traits::ToPrimitive as _;
use waterui_browser_wpe::{DmaBufFormat, DmaBufFrame, DmaBufFrameCopier, DmaBufPlane};
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};

use super::presenter::{OwnedFrameMailbox, TexturePresenter};
use super::{CefViewport, request_browser_frame};
use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

struct LinuxFrameSink {
    copier: Rc<DmaBufFrameCopier>,
    mailbox: Rc<OwnedFrameMailbox>,
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
        let borrowed_frame = DmaBufFrame::owned(
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
        self.mailbox
            .publish(element, self.copier.copy(borrowed_frame));
    }

    fn set_popup_rect(&self, rect: Option<CefPopupRect>) {
        self.mailbox.set_popup_rect(rect);
    }
}

pub(super) struct CefGpuView {
    page: CefPageHandle,
    viewport: CefViewport,
    mailbox: Rc<OwnedFrameMailbox>,
    presenter: Option<TexturePresenter>,
}

impl CefGpuView {
    pub(super) fn new(page: CefPageHandle, viewport: CefViewport) -> Self {
        Self {
            page,
            viewport,
            mailbox: Rc::new(OwnedFrameMailbox::new()),
            presenter: None,
        }
    }
}

impl GpuView for CefGpuView {
    #[expect(
        clippy::future_not_send,
        reason = "CEF, DMA-BUF interop, and WaterUI view state are confined to the UI thread"
    )]
    async fn setup(&mut self, context: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        assert!(
            matches!(
                context.adapter.get_info().backend,
                wgpu::Backend::Vulkan | wgpu::Backend::Gl
            ),
            "CEF DMA-BUF composition requires WaterUI's Vulkan or EGL/GLES backend"
        );
        let redraw = context.redraw_handle.clone();
        self.mailbox
            .set_waker(Rc::new(move || redraw.request_redraw()));
        self.page.set_frame_sink(LinuxFrameSink {
            copier: Rc::new(DmaBufFrameCopier::new(context)),
            mailbox: Rc::clone(&self.mailbox),
        });
        self.presenter = Some(TexturePresenter::new(context));
    }

    fn render(&mut self, frame: &mut GpuFrame<'_>) {
        self.page.pump();
        request_browser_frame(&self.page, frame);
        let scale = self.viewport.scale();
        let logical_width = (f64::from(frame.width) / scale).round().max(1.0);
        let logical_height = (f64::from(frame.height) / scale).round().max(1.0);
        self.page.set_viewport(
            logical_width
                .to_u32()
                .expect("CEF logical width exceeds u32"),
            logical_height
                .to_u32()
                .expect("CEF logical height exceeds u32"),
            scale.to_f32().expect("CEF scale exceeds f32"),
        );
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

pub(super) fn gpu_view(page: CefPageHandle, viewport: CefViewport) -> CefGpuView {
    CefGpuView::new(page, viewport)
}
