use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use num_traits::ToPrimitive as _;
use objc2_core_foundation::CFRetained;
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureType,
    MTLTextureUsage,
};
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};

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

struct RetainedIoSurface {
    surface: CFRetained<IOSurfaceRef>,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

impl RetainedIoSurface {
    /// # Safety
    ///
    /// `frame.shared_texture_io_surface` must be the surface of a live
    /// accelerated paint callback, so that retaining it happens while it is
    /// still valid.
    unsafe fn retain(frame: &AcceleratedPaintInfo) -> Self {
        let size = &frame.extra.coded_size;
        let width = u32::try_from(size.width).expect("CEF IOSurface width must be positive");
        let height = u32::try_from(size.height).expect("CEF IOSurface height must be positive");
        let pointer = NonNull::new(frame.shared_texture_io_surface.cast::<IOSurfaceRef>())
            .expect("CEF accelerated paint returned a null IOSurface");
        // SAFETY: the caller contract makes `pointer` a live `IOSurface`; retaining
        // it here is what keeps it valid after CEF reclaims the frame.
        let surface = unsafe { CFRetained::retain(pointer) };
        let format = if frame.format == ColorType::BGRA_8888 {
            wgpu::TextureFormat::Bgra8Unorm
        } else if frame.format == ColorType::RGBA_8888 {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            panic!("CEF returned unsupported macOS accelerated color format")
        };
        Self {
            surface,
            width,
            height,
            format,
        }
    }

    fn pointer(&self) -> *mut c_void {
        let surface: &IOSurfaceRef = &self.surface;
        std::ptr::from_ref(surface).cast_mut().cast()
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
        let surface = unsafe { RetainedIoSurface::retain(frame) };
        let size = (surface.width, surface.height);
        if self.imported_size.replace(size) != size {
            tracing::debug!(
                width = surface.width,
                height = surface.height,
                format = ?surface.format,
                "Importing a resized accelerated CEF IOSurface"
            );
        }
        let source = import_iosurface(
            &self.device,
            surface.pointer(),
            surface.width,
            surface.height,
            surface.format,
        );
        let owned = copy_source_texture(
            &self.device,
            &self.queue,
            &source,
            source.size(),
            surface.format,
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

fn import_iosurface(
    device: &wgpu::Device,
    handle: *mut c_void,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    let descriptor = wgpu::TextureDescriptor {
        label: Some("waterui_cef_iosurface"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    };
    let metal_format = match format {
        wgpu::TextureFormat::Bgra8Unorm => MTLPixelFormat::BGRA8Unorm,
        wgpu::TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        _ => panic!("CEF IOSurface has unsupported wgpu format {format:?}"),
    };
    let hal_texture = objc2::rc::autoreleasepool(|_| {
        // SAFETY: the handle is only borrowed to read the raw `MTLDevice`, and it is
        // not kept past this closure.
        let hal_device = unsafe {
            device
                .as_hal::<wgpu::hal::api::Metal>()
                .expect("CEF IOSurface requires a Metal device")
        };
        let raw_device = hal_device.raw_device();
        let metal_descriptor = MTLTextureDescriptor::new();
        // SAFETY: plain setters on a descriptor this scope just allocated and owns.
        unsafe {
            metal_descriptor.setWidth(width.to_usize().expect("CEF IOSurface width exceeds usize"));
            metal_descriptor.setHeight(
                height
                    .to_usize()
                    .expect("CEF IOSurface height exceeds usize"),
            );
        }
        metal_descriptor.setTextureType(MTLTextureType::Type2D);
        metal_descriptor.setPixelFormat(metal_format);
        metal_descriptor.setUsage(MTLTextureUsage::ShaderRead);
        metal_descriptor.setStorageMode(if raw_device.hasUnifiedMemory() {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        // SAFETY: `handle` comes from `RetainedIoSurface::pointer`, which hands out
        // the surface this import retained, so it outlives the borrow.
        let surface = unsafe { &*handle.cast::<IOSurfaceRef>() };
        let texture = raw_device
            .newTextureWithDescriptor_iosurface_plane(&metal_descriptor, surface, 0)
            .expect("Metal rejected CEF IOSurface import");
        // SAFETY: the texture was created from `metal_descriptor` immediately above,
        // so the format, type, mip and layer counts repeated here match it, and
        // ownership of the `MTLTexture` transfers to the returned hal texture.
        unsafe {
            <wgpu::hal::api::Metal as wgpu::hal::Api>::Device::texture_from_raw(
                texture,
                descriptor.format,
                MTLTextureType::Type2D,
                1,
                1,
                wgpu::hal::CopyExtent {
                    width,
                    height,
                    depth: 1,
                },
            )
        }
    });
    // SAFETY: `hal_texture` was built for this device with `descriptor`'s format and
    // size, and is moved into the wgpu texture that now owns it.
    unsafe { device.create_texture_from_hal::<wgpu::hal::api::Metal>(hal_texture, &descriptor) }
}
