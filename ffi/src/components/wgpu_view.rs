use crate::WuiEnv;
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::Any;
use core::cell::Cell;
use core::ffi::c_void;
use waterui_graphics::WgpuView;

// NOTE: The following are assumed to be defined elsewhere in the FFI crate
// or backend context, providing access to the core runtime components.
unsafe extern "C" {
    fn waterui_get_current_device() -> &'static wgpu::Device;
    fn waterui_get_current_queue() -> &'static wgpu::Queue;
    fn with_view_registry(view_id: u64, callback: Box<dyn FnOnce(&mut dyn Any)>);
}

/// C-compatible struct for WgpuView properties.
#[repr(C)]
pub struct WuiWgpuViewProperties {
    pub width: f32,
    pub height: f32,
}

/// Gets the properties (width and height) of a WgpuView.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_wgpu_view_get_properties(view_id: u64) -> WuiWgpuViewProperties {
    let width = Rc::new(Cell::new(0.0f32));
    let height = Rc::new(Cell::new(0.0f32));
    let width_capture = Rc::clone(&width);
    let height_capture = Rc::clone(&height);
    unsafe {
        with_view_registry(
            view_id,
            Box::new(move |view_any| {
                if let Some(wgpu_view) = view_any.downcast_ref::<WgpuView>() {
                    width_capture.set(wgpu_view.width);
                    height_capture.set(wgpu_view.height);
                }
            }),
        )
    };
    WuiWgpuViewProperties {
        width: width.get(),
        height: height.get(),
    }
}

/// Triggers the drawing callback for a WgpuView.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_wgpu_view_draw(
    view_id: u64,
    env: *const WuiEnv,
    _native_texture_handle: *mut c_void,
    texture_format_u32: u32,
) {
    let _ = env;
    let texture_format =
        decode_texture_format(texture_format_u32).unwrap_or(wgpu::TextureFormat::Bgra8Unorm); // Sensible default until backend wiring is complete.
    let device = unsafe { waterui_get_current_device() };
    let queue = unsafe { waterui_get_current_queue() };
    unsafe {
        with_view_registry(
            view_id,
            Box::new(move |view_any| {
                if let Some(wgpu_view) = view_any.downcast_ref::<WgpuView>() {
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("WgpuView Wrapped Texture"),
                        size: wgpu::Extent3d {
                            width: wgpu_view.width as u32,
                            height: wgpu_view.height as u32,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: texture_format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        view_formats: &[],
                    });
                    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                    (wgpu_view.on_draw)(device, queue, &texture_view, texture_format);
                }
            }),
        )
    };
}

fn decode_texture_format(raw: u32) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat::*;
    // TODO: Expand mapping as supported formats are required by the backends.
    Some(match raw {
        0 => Rgba8Unorm,
        1 => Rgba8UnormSrgb,
        2 => Bgra8Unorm,
        3 => Bgra8UnormSrgb,
        _ => return None,
    })
}
