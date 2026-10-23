//! Immutable, colour-tagged `IOSurface`s for recorded layer pixels.

use cherenkov::RenderError;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetIOSurface,
    kCVPixelBufferIOSurfacePropertiesKey, kCVPixelBufferMetalCompatibilityKey,
    kCVPixelFormatType_32BGRA, kCVPixelFormatType_64RGBAHalf, kCVReturnSuccess,
};
use objc2_io_surface::IOSurfaceRef;
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLSharedEvent, MTLSharedEventListener, MTLTextureDescriptor,
    MTLTextureType, MTLTextureUsage,
};
use objc2_quartz_core::CALayer;
use std::ptr::NonNull;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct Buffer {
    pub texture: wgpu::Texture,
    pub surface: CFRetained<IOSurfaceRef>,
    pub generation: u64,
    pub ready: Arc<AtomicBool>,
    pub headroom: f32,
    pub pending_headroom: Option<crate::render::headroom::Readback>,
    pub range: crate::render::present::ColorRange,
    /// Signalled on the conversion command buffer. The listener waits for
    /// value 1, which this buffer publishes once.
    completion: Retained<ProtocolObject<dyn MTLSharedEvent>>,
}

fn surface(size: (u32, u32), hdr: bool) -> Result<CFRetained<IOSurfaceRef>, RenderError> {
    let empty = CFDictionary::<CFString, CFType>::from_slices(&[], &[]);
    // SAFETY: immutable framework keys and the CFBoolean singleton.
    let attributes = unsafe {
        CFDictionary::<CFString, CFType>::from_slices(
            &[
                kCVPixelBufferIOSurfacePropertiesKey,
                kCVPixelBufferMetalCompatibilityKey,
            ],
            &[
                &empty,
                objc2_core_foundation::kCFBooleanTrue.expect("CFBoolean"),
            ],
        )
    };
    let mut raw = std::ptr::null_mut();
    // SAFETY: valid attributes, and an out pointer for the retained buffer.
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            size.0 as usize,
            size.1 as usize,
            if hdr {
                kCVPixelFormatType_64RGBAHalf
            } else {
                kCVPixelFormatType_32BGRA
            },
            Some(attributes.as_opaque()),
            std::ptr::NonNull::from(&mut raw),
        )
    };
    if status != kCVReturnSuccess {
        return Err(RenderError::Render(format!(
            "static plane pixel buffer: {status}"
        )));
    }
    // SAFETY: success transfers one reference to the caller.
    let buffer = unsafe {
        CFRetained::<CVPixelBuffer>::from_raw(
            std::ptr::NonNull::new(raw).expect("successful pixel-buffer allocation"),
        )
    };
    let surface = CVPixelBufferGetIOSurface(Some(&buffer)).expect("IOSurface-backed pixel buffer");
    // SAFETY: the colour-space name and IOSurface key are immutable statics.
    let space = objc2_core_graphics::CGColorSpace::with_name(Some(unsafe {
        if hdr {
            objc2_core_graphics::kCGColorSpaceExtendedLinearDisplayP3
        } else {
            objc2_core_graphics::kCGColorSpaceDisplayP3
        }
    }))
    .expect("Display P3 color space");
    let property = space.property_list().expect("serializable colour space");
    // SAFETY: `surface` is a live IOSurface, the key is the framework's
    // colour-space constant, and `property` is the colour space's
    // property-list representation — a value IOSurface accepts.
    unsafe { surface.set_value(objc2_io_surface::kIOSurfaceColorSpace, &property) };
    Ok(surface)
}

impl Buffer {
    pub fn new(
        device: &wgpu::Device,
        size: (u32, u32),
        generation: u64,
        headroom: f32,
        range: crate::render::present::ColorRange,
    ) -> Result<Self, RenderError> {
        let hdr = range == crate::render::present::ColorRange::HighDynamicRange;
        let format = if hdr {
            wgpu::TextureFormat::Rgba16Float
        } else {
            wgpu::TextureFormat::Bgra8Unorm
        };
        let surface = surface(size, hdr)?;
        // SAFETY: Metal is the Apple backend and the descriptor is fully specified.
        let metal = unsafe { device.as_hal::<wgpu::hal::metal::Api>() }.expect("Metal device");
        // SAFETY: the pixel format is a valid `MTLPixelFormat` and the
        // dimensions describe the 2D texture the IOSurface created above
        // backs.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                if hdr {
                    MTLPixelFormat::RGBA16Float
                } else {
                    MTLPixelFormat::BGRA8Unorm
                },
                size.0 as usize,
                size.1 as usize,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        let raw = metal
            .raw_device()
            .newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, 0)
            .ok_or_else(|| RenderError::Render("Metal refused a static plane IOSurface".into()))?;
        let completion = metal.raw_device().newSharedEvent().ok_or_else(|| {
            RenderError::Render("Metal refused a capture completion event".into())
        })?;
        let extent = wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        };
        // SAFETY: this texture belongs to the engine's device, has the declared
        // format and extent, and wgpu retains it through every GPU submission.
        let texture = unsafe {
            let texture = wgpu::hal::metal::Device::texture_from_raw(
                raw,
                format,
                MTLTextureType::Type2D,
                1,
                1,
                extent.into(),
                None,
            );
            device.create_texture_from_hal::<wgpu::hal::metal::Api>(
                texture,
                &wgpu::TextureDescriptor {
                    label: Some("static plane IOSurface"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::wgt::TextureUses::UNINITIALIZED,
            )
        };
        Ok(Self {
            texture,
            surface,
            generation,
            headroom,
            pending_headroom: None,
            range,
            ready: Arc::new(AtomicBool::new(false)),
            completion,
        })
    }

    /// Stage the completion signal on the capture's own submission.
    pub fn signal(&self, queue: &wgpu::Queue) {
        // SAFETY: this live event is signalled after the queue's next submission.
        unsafe {
            queue
                .as_hal::<wgpu::hal::metal::Api>()
                .expect("Metal queue")
                .add_signal_event(self.completion.clone(), 1);
        }
    }

    /// Register after submission. A completed event immediately invokes its
    /// listener, publishing the capture and its shared headroom readback.
    pub fn completing(&self, waker: cherenkov::CompletionWaker) {
        autoreleasepool(|_| {
            let ready = Arc::clone(&self.ready);
            let block = block2::RcBlock::new(
                move |_: NonNull<ProtocolObject<dyn MTLSharedEvent>>, _: u64| {
                    ready.store(true, Ordering::Release);
                    waker.wake();
                },
            );
            // SAFETY: Metal retains the block and invokes it when the live
            // event reaches 1, including when it reached 1 before registration.
            // The callback publishes readiness and wakes the engine.
            unsafe {
                self.completion.notifyListener_atValue_block(
                    &MTLSharedEventListener::sharedListener(),
                    1,
                    block2::RcBlock::as_ptr(&block),
                );
            }
        });
    }

    pub fn bytes(&self) -> u64 {
        self.surface.alloc_size() as u64
            + self
                .pending_headroom
                .as_ref()
                .map_or(0, crate::render::headroom::Readback::bytes)
    }
}

/// `IOSurface` is reference counted and its immutable contents may cross threads.
pub(super) struct Contents(
    pub CFRetained<IOSurfaceRef>,
    pub crate::render::present::ColorRange,
    pub f32,
);
#[expect(
    clippy::non_send_fields_in_send_ty,
    reason = "IOSurface is thread safe; only immutable GPU-completed contents cross to main"
)]
// SAFETY: the reference holds an immutable IOSurface, shown only after its
// GPU fence, so handing it to the main thread races with nothing.
unsafe impl Send for Contents {}

impl Contents {
    pub fn set(self, layer: &CALayer) {
        // SAFETY: CALayer accepts an IOSurface as contents and retains it.
        let value: &AnyObject = unsafe { &*CFRetained::as_ptr(&self.0).as_ptr().cast() };
        // SAFETY: `value` is the retained IOSurface above — a type CALayer's
        // `contents` accepts — and `layer` is touched only on main.
        unsafe { layer.setContents(Some(value)) };
        // Scene-referred pixels carry their source headroom.
        // Core Animation requires content headroom tagging to activate EDR.
        layer.setContentsHeadroom(
            if self.1 == crate::render::present::ColorRange::HighDynamicRange {
                f64::from(self.2)
            } else {
                1.0
            },
        );
        // SAFETY: the tone-map mode is an immutable framework static.
        layer.setToneMapMode(unsafe { objc2_quartz_core::CAToneMapModeIfSupported });
        // SAFETY: both dynamic-range names are immutable framework statics.
        layer.setPreferredDynamicRange(unsafe {
            if self.1 == crate::render::present::ColorRange::HighDynamicRange {
                objc2_quartz_core::CADynamicRangeHigh
            } else {
                objc2_quartz_core::CADynamicRangeStandard
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::present::ColorRange;

    #[test]
    fn sdr_native_capture_uses_less_storage_than_hdr() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .expect("Metal adapter");
        let (device, _) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("Metal device");
        let sdr = Buffer::new(&device, (1024, 512), 1, 1.0, ColorRange::WideGamut)
            .expect("SDR IOSurface");
        let hdr = Buffer::new(&device, (1024, 512), 2, 2.0, ColorRange::HighDynamicRange)
            .expect("HDR IOSurface");
        assert_eq!(sdr.texture.format(), wgpu::TextureFormat::Bgra8Unorm);
        assert_eq!(hdr.texture.format(), wgpu::TextureFormat::Rgba16Float);
        assert!(sdr.bytes() < hdr.bytes());
    }
}
