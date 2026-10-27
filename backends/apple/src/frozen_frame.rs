//! Frozen frames: the last pixels an armed GPU or filtered surface put
//! on screen, kept as an immutable GPU input for a transition's
//! retirement hand-off (§2.1).
//!
//! A `CAMetalLayer`'s drawables are recycled by Core Animation, so a
//! surface that must stay capturable after its semantics end copies each
//! frame that wrote its target into a private texture — encoded into
//! the frame's own submission on the same wgpu queue, so ordering is
//! implicit and nothing blocks. Only the success arm promotes the copy
//! to `last`; every other arm returns it to the spare pool. No drawable
//! or lease ever outlives `present`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use cocoa_ui::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::{MTLDevice, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage};
use waterui_graphics::gpu::SharedGpuContext;
use waterui_graphics::wgpu;

/// An armed surface's frozen-frame bookkeeping: the copy of the last
/// frame that reached the screen, plus recycled copy targets reused
/// only when their size, format and context generation still match.
pub struct FrozenFrameSlot {
    /// The last frame that reached the screen.
    last: RefCell<Option<FrozenFrame>>,
    /// Recycled copy targets — reused only for the same size, format
    /// and context generation.
    spare: RefCell<Vec<FrozenFrame>>,
}

impl core::fmt::Debug for FrozenFrameSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrozenFrameSlot")
            .field("has_last", &self.last.borrow().is_some())
            .finish_non_exhaustive()
    }
}

impl FrozenFrameSlot {
    /// An empty slot — `last` fills on the first promoted frame.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            last: RefCell::new(None),
            spare: RefCell::new(Vec::new()),
        }
    }

    /// A private-storage copy target for `size`/`format` under
    /// `context` — a matching spare, or a fresh texture pair. The Metal
    /// texture is what a [`cocoa_ui::capture::FrozenSurface`] hands to a
    /// later capture; the wgpu import is this copy's destination.
    fn target(
        &self,
        context: &Arc<SharedGpuContext>,
        source: &wgpu::Texture,
        size: (u32, u32),
    ) -> FrozenFrame {
        let pixel_format = cocoa_ui::metal::wgpu_to_metal_format(source.format());
        {
            let mut spares = self.spare.borrow_mut();
            if let Some(index) = spares.iter().position(|frame| {
                frame.size == size
                    && frame.pixel_format == pixel_format
                    && Arc::ptr_eq(&frame.context, context)
            }) {
                return spares.swap_remove(index);
            }
        }
        // SAFETY: creates a valid descriptor; Metal validates the arguments.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                pixel_format,
                size.0 as usize,
                size.1 as usize,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead | MTLTextureUsage::RenderTarget);
        descriptor.setStorageMode(MTLStorageMode::Private);
        let texture = crate::gpu_runtime::raw_metal_device(context)
            .newTextureWithDescriptor(&descriptor)
            .expect("failed to create the frozen frame copy texture");
        // SAFETY: `texture` is a fresh private texture on this context's
        // device, so `UNINITIALIZED` is its true handoff state; the copy
        // below is its first access.
        let wgpu_texture = unsafe {
            cocoa_ui::metal::import_texture(
                context.device(),
                texture.clone(),
                source.format(),
                size.0,
                size.1,
                wgpu::TextureUsages::COPY_DST,
                wgpu::TextureUses::UNINITIALIZED,
                "Frozen Frame Copy",
            )
        };
        FrozenFrame {
            texture,
            wgpu: wgpu_texture,
            context: context.clone(),
            pixel_format,
            size,
        }
    }

    /// Appends `copy_texture_to_texture(source -> copy)` to `encoder`,
    /// the frame's own submission on the same wgpu queue, so ordering
    /// is implicit. Returns the copy for the work item to carry into
    /// [`promote`](Self::promote) or [`recycle`](Self::recycle).
    pub fn encode_copy(
        &self,
        context: &Arc<SharedGpuContext>,
        source: &wgpu::Texture,
        size: (u32, u32),
        encoder: &mut wgpu::CommandEncoder,
    ) -> FrozenFrame {
        let frame = self.target(context, source, size);
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            frame.wgpu.as_image_copy(),
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        frame
    }

    /// A successful frame: the copy becomes `last`; the previous `last`
    /// goes to the spare pool.
    pub fn promote(&self, copy: FrozenFrame) {
        if let Some(previous) = self.last.borrow_mut().replace(copy) {
            self.spare.borrow_mut().push(previous);
        }
    }

    /// A stale, failed or unpresented frame — the copy target is
    /// recycled, never promoted.
    pub fn recycle(&self, copy: FrozenFrame) {
        self.spare.borrow_mut().push(copy);
    }

    /// The retirement hand-off: the last frame that reached the screen,
    /// or `None` when nothing ever presented.
    pub fn take(&self) -> Option<FrozenFrame> {
        self.last.borrow_mut().take()
    }
}

/// One immutable copy of a presented frame: the private Metal texture a
/// capture later blits from, its wgpu import the copy wrote through, and
/// the exact context the copy was submitted on.
pub struct FrozenFrame {
    /// The private-storage copy — the [`cocoa_ui::capture::FrozenSurface`]
    /// payload.
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    /// The same texture imported for the GPU→GPU copy.
    wgpu: wgpu::Texture,
    /// The exact context the copy was submitted on — spares recycle only
    /// within it, and the generation it reports stamps the hand-off.
    context: Arc<SharedGpuContext>,
    /// The texture's Metal pixel format.
    pixel_format: objc2_metal::MTLPixelFormat,
    /// The frame's pixel size.
    size: (u32, u32),
}

impl core::fmt::Debug for FrozenFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrozenFrame")
            .field("size", &self.size)
            .field("pixel_format", &self.pixel_format)
            .field("generation", &self.context.generation())
            .finish_non_exhaustive()
    }
}

impl FrozenFrame {
    /// The immutable capture input the retirement hand-off carries.
    pub fn into_surface(self) -> cocoa_ui::capture::FrozenSurface {
        cocoa_ui::capture::FrozenSurface {
            texture: self.texture,
            pixel_format: self.pixel_format,
            generation: self.context.generation(),
        }
    }
}

/// The `Rc` a capturable shares with its surface's armed slot — kept
/// here so every owner names the same handle.
pub type SharedFrozenFrameSlot = Rc<FrozenFrameSlot>;
