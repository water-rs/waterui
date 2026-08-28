use std::cell::Cell;
use std::ptr::NonNull;
use std::rc::Rc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use num_traits::ToPrimitive as _;
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};
use wgpu_external_frame::io_surface::IoSurfaceFrame;

use super::presenter::{OwnedFrameMailbox, TexturePresenter, copy_source_texture};
use super::{CefViewport, request_browser_frame};
use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

// # Safety
//
// The `unsafe` in this file rests on one fact stated by
// [`AcceleratedFrameSink::import`]: CEF hands the sink a frame whose
// `IOSurface` is valid for the duration of that call and returns it to its pool
// immediately afterwards. Everything here therefore either runs inside that
// call, or operates on the surface this module retained during it. Sites that
// depend on something else say so.

struct CefIoSurface {
    /// The retained surface, at the allocated extent it must be imported as.
    surface: IoSurfaceFrame,
    /// The part of it that actually holds the page.
    ///
    /// Chromium may allocate the shared image with alignment padding, so this
    /// is not always the coded size. Presenting the coded texture edge to edge
    /// stretched the page and sampled the padding gutter.
    visible_width: u32,
    visible_height: u32,
}

impl CefIoSurface {
    /// # Safety
    ///
    /// `frame.shared_texture_io_surface` must be the surface of a live
    /// accelerated paint callback, so that retaining it happens while it is
    /// still valid.
    unsafe fn retain(frame: &AcceleratedPaintInfo) -> Self {
        let size = &frame.extra.coded_size;
        let width = u32::try_from(size.width).expect("CEF IOSurface width must be positive");
        let height = u32::try_from(size.height).expect("CEF IOSurface height must be positive");
        let visible = &frame.extra.visible_rect;
        // Clamped to the allocation: a visible rect larger than the coded size
        // would be a CEF bug, and copying past the end of the surface is not the
        // way to find out.
        let visible_width = u32::try_from(visible.width).unwrap_or(width).min(width);
        let visible_height = u32::try_from(visible.height).unwrap_or(height).min(height);
        let pointer = NonNull::new(frame.shared_texture_io_surface)
            .expect("CEF accelerated paint returned a null IOSurface");
        let format = if frame.format == ColorType::BGRA_8888 {
            wgpu::TextureFormat::Bgra8Unorm
        } else if frame.format == ColorType::RGBA_8888 {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            panic!("CEF returned unsupported macOS accelerated color format")
        };
        // SAFETY: the caller contract makes `pointer` a live `IOSurface` of the
        // coded size and format read out of the same paint info; retaining it
        // here is what keeps it valid after CEF reclaims the frame.
        let surface = unsafe { IoSurfaceFrame::retain(pointer, width, height, format) };
        Self {
            surface,
            visible_width,
            visible_height,
        }
    }
}

struct MacFrameSink {
    device: wgpu::Device,
    queue: wgpu::Queue,
    mailbox: Rc<OwnedFrameMailbox>,
    imported_size: Cell<(u32, u32)>,
}

impl AcceleratedFrameSink for MacFrameSink {
    fn import(
        &self,
        element: PaintElementType,
        _dirty_rects: &[Rect],
        frame: &AcceleratedPaintInfo,
    ) {
        // SAFETY: `import` is the accelerated paint callback, so `frame` names a
        // surface that is valid for exactly this call.
        let surface = unsafe { CefIoSurface::retain(frame) };
        let size = (surface.surface.width(), surface.surface.height());
        if self.imported_size.replace(size) != size {
            tracing::debug!(
                width = size.0,
                height = size.1,
                format = ?surface.surface.format(),
                "Importing a resized accelerated CEF IOSurface"
            );
        }
        let source = surface.surface.import(&self.device);
        let owned = copy_source_texture(
            &self.device,
            &self.queue,
            &source,
            wgpu::Extent3d {
                width: surface.visible_width,
                height: surface.visible_height,
                depth_or_array_layers: 1,
            },
            surface.surface.format(),
        );
        self.mailbox.publish(element, owned);
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
        reason = "CEF, Metal, and WaterUI view state are confined to the UI thread"
    )]
    async fn setup(&mut self, context: &GpuContext<'_>, _env: &mut waterui_core::Environment) {
        assert_eq!(
            context.adapter.get_info().backend,
            wgpu::Backend::Metal,
            "CEF IOSurface composition requires WaterUI's Metal backend"
        );
        let redraw = context.redraw_handle.clone();
        self.mailbox
            .set_waker(Rc::new(move || redraw.request_redraw()));
        self.page.set_frame_sink(MacFrameSink {
            device: context.device.clone(),
            queue: context.queue.clone(),
            mailbox: Rc::clone(&self.mailbox),
            imported_size: Cell::new((0, 0)),
        });
        tracing::debug!("Installed the accelerated CEF frame sink");
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
