//! Windows shared texture handle import.
//!
//! A producer that renders with Direct3D publishes its texture as an NT shared
//! handle. Direct3D 12 reopens that handle as an `ID3D12Resource`, and `wgpu`
//! adopts the resource through its `hal` layer, so no pixel is copied.
//!
//! The handle a producer hands over belongs to that producer and is usually
//! reclaimed when its callback returns, so [`SharedHandleFrame::duplicate`]
//! duplicates it into this process's own handle table first.

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::ptr::NonNull;

use windows::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::System::Threading::GetCurrentProcess;

/// An owned duplicate of a producer's shared texture handle.
#[derive(Debug)]
pub struct SharedHandleFrame {
    handle: OwnedHandle,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
}

impl SharedHandleFrame {
    /// Duplicates `handle` into this process, so that it outlives the callback
    /// that handed it over.
    ///
    /// `width` and `height` are the resource's allocated extent, which is what
    /// it must be imported as; a producer that padded its allocation should
    /// narrow the *copy* it makes out of the imported texture rather than
    /// claiming a smaller resource here.
    ///
    /// # Panics
    ///
    /// Panics when the handle cannot be duplicated.
    ///
    /// # Safety
    ///
    /// `handle` must be a shared-texture handle that is valid in this process
    /// for the duration of this call, and `width`/`height`/`format` must
    /// describe the resource behind it.
    #[must_use]
    pub unsafe fn duplicate(
        handle: NonNull<c_void>,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let mut duplicate = HANDLE::default();
        // SAFETY: `DuplicateHandle` requires live process handles and a source
        // handle valid in the source process. Both process arguments are
        // `GetCurrentProcess`, the pseudo-handle for this process, which is
        // always valid and needs no closing; the source handle is valid in this
        // process by this function's contract. `duplicate` is an initialized
        // local the call writes the new handle into, and it is checked for
        // failure before being read.
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                HANDLE(handle.as_ptr()),
                GetCurrentProcess(),
                &raw mut duplicate,
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
            .expect("failed to duplicate the shared texture handle");
        }
        Self {
            // SAFETY: `from_raw_handle` takes ownership of a handle nothing else
            // owns and that must be closable with `CloseHandle`. `duplicate` is
            // the handle `DuplicateHandle` just created for this process — it
            // succeeded, so the handle is live — and this is its only owner;
            // the producer's original is untouched.
            handle: unsafe { OwnedHandle::from_raw_handle(duplicate.0) },
            width,
            height,
            format,
        }
    }

    /// The resource's allocated pixel width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The resource's allocated pixel height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The `wgpu` format the resource imports as.
    #[must_use]
    pub const fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Imports the shared resource as a `COPY_SRC` texture on `device`.
    ///
    /// The texture aliases the producer's resource rather than owning a copy of
    /// it, so the producer may overwrite it as soon as it takes the resource
    /// back; copy out of the returned texture before that happens.
    ///
    /// # Panics
    ///
    /// Panics unless `device` is a Direct3D 12 device, and unless the handle
    /// names a `ID3D12Resource`.
    #[must_use]
    pub fn import(&self, device: &wgpu::Device) -> wgpu::Texture {
        let descriptor = wgpu::TextureDescriptor {
            label: Some("wgpu_external_frame_shared_handle"),
            size: wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        // SAFETY: `Device::as_hal` requires the named backend to be the
        // device's real one and that the exposed device is not used to
        // invalidate wgpu's state. A device on another backend surfaces as
        // `None` and panics here rather than being reinterpreted, and the raw
        // device is used only to open a resource of this function's own.
        let hal_device = unsafe {
            device
                .as_hal::<wgpu::hal::api::Dx12>()
                .expect("a shared texture handle requires a Direct3D 12 device")
        };
        let mut resource = None;
        // SAFETY: `OpenSharedHandle` reads the handle and writes the requested
        // interface through the out-pointer. The handle is owned by this
        // process and still open, being borrowed from the `OwnedHandle` this
        // frame holds; `resource` is an initialized local of the interface type
        // named by the turbofish, which is what the call expects, and it is
        // checked for `None` before use.
        unsafe {
            hal_device
                .raw_device()
                .OpenSharedHandle::<ID3D12Resource>(
                    HANDLE(self.handle.as_raw_handle()),
                    &raw mut resource,
                )
                .expect("Direct3D 12 failed to open the shared texture handle");
        }
        let resource = resource.expect("the shared handle did not contain a D3D12 texture");
        // SAFETY: `texture_from_raw` adopts a resource that must match the
        // descriptor it is described with. `resource` was just opened from a
        // handle the caller declared to be a texture of this frame's extent and
        // format, the dimension and mip/array counts repeated here are the ones
        // `descriptor` carries, and ownership of the resource transfers to the
        // returned hal texture.
        let hal_texture = unsafe {
            <wgpu::hal::api::Dx12 as wgpu::hal::Api>::Device::texture_from_raw(
                resource,
                self.format,
                wgpu::TextureDimension::D2,
                descriptor.size,
                1,
                1,
            )
        };
        // SAFETY: `hal_texture` was built for this device with `descriptor`'s
        // format and size, and is moved into the wgpu texture that now owns it.
        unsafe { device.create_texture_from_hal::<wgpu::hal::api::Dx12>(hal_texture, &descriptor) }
    }
}
