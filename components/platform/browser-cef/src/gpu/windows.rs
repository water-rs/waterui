use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::rc::Rc;

use cef::{AcceleratedPaintInfo, ColorType, PaintElementType, Rect};
use num_traits::ToPrimitive as _;
use waterui_graphics::gpu_surface::{GpuContext, GpuFrame, GpuView};
use windows::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::System::Threading::GetCurrentProcess;

use super::presenter::{OwnedFrameMailbox, TexturePresenter, copy_source_texture};
use super::{CefViewport, request_browser_frame};
use crate::{AcceleratedFrameSink, CefPageHandle, CefPopupRect};

struct SharedD3dFrame {
    handle: OwnedHandle,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

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
        assert!(
            !frame.shared_texture_handle.is_null(),
            "CEF accelerated paint returned a null D3D shared handle"
        );
        let mut duplicate = HANDLE::default();
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                HANDLE(frame.shared_texture_handle),
                GetCurrentProcess(),
                &raw mut duplicate,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
            .expect("failed to duplicate CEF D3D shared texture handle");
        }
        let size = &frame.extra.coded_size;
        let format = if frame.format == ColorType::BGRA_8888 {
            wgpu::TextureFormat::Bgra8Unorm
        } else if frame.format == ColorType::RGBA_8888 {
            wgpu::TextureFormat::Rgba8Unorm
        } else {
            panic!("CEF returned unsupported Windows accelerated color format")
        };
        let shared = SharedD3dFrame {
            handle: unsafe { OwnedHandle::from_raw_handle(duplicate.0) },
            width: u32::try_from(size.width).expect("CEF D3D texture width must be positive"),
            height: u32::try_from(size.height).expect("CEF D3D texture height must be positive"),
            format,
        };
        let source = import_d3d_texture(
            &self.device,
            shared.handle.as_raw_handle(),
            shared.width,
            shared.height,
            shared.format,
        );
        let owned = copy_source_texture(
            &self.device,
            &self.queue,
            &source,
            source.size(),
            shared.format,
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

fn import_d3d_texture(
    device: &wgpu::Device,
    handle: std::os::windows::io::RawHandle,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    let descriptor = wgpu::TextureDescriptor {
        label: Some("waterui_cef_d3d_shared_texture"),
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
    let hal_device = unsafe {
        device
            .as_hal::<wgpu::hal::api::Dx12>()
            .expect("CEF shared D3D texture requires a Direct3D 12 device")
    };
    let mut resource = None;
    unsafe {
        hal_device
            .raw_device()
            .OpenSharedHandle::<ID3D12Resource>(HANDLE(handle), &raw mut resource)
            .expect("Direct3D 12 failed to open CEF shared texture handle");
    }
    let resource = resource.expect("CEF shared handle did not contain a D3D12 texture");
    let hal_texture = unsafe {
        <wgpu::hal::api::Dx12 as wgpu::hal::Api>::Device::texture_from_raw(
            resource,
            format,
            wgpu::TextureDimension::D2,
            descriptor.size,
            1,
            1,
        )
    };
    unsafe { device.create_texture_from_hal::<wgpu::hal::api::Dx12>(hal_texture, &descriptor) }
}
