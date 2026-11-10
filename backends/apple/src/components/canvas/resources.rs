//! The resources a canvas registers with the Apple target: images as
//! `CGImage`s and fonts as Core Text font descriptors.
//!
//! The scene resource table (`SceneResources`) deduplicates and holds
//! registrations; this is the target half it registers with. A registration
//! lives in the [`Registry`] exactly as long as its [`Handle`] does.
//!
//! # Safety
//!
//! The `unsafe` calls build Core Graphics and Core Text objects from byte
//! buffers and descriptors that outlive each call.

use core::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use super::text::LaidOutText;
use cocoa_ui::objc2_core_foundation::{CFData, CFRetained, CFString};
use cocoa_ui::objc2_core_graphics::{
    CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGDataProvider, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, CGImageComponentInfo, kCGColorSpaceDisplayP3,
    kCGColorSpaceExtendedDisplayP3, kCGColorSpaceExtendedLinearDisplayP3,
    kCGColorSpaceExtendedLinearSRGB, kCGColorSpaceExtendedSRGB, kCGColorSpaceLinearDisplayP3,
    kCGColorSpaceLinearSRGB, kCGColorSpaceSRGB,
};
use cocoa_ui::objc2_core_text::{CTFontDescriptor, CTFontManagerCreateFontDescriptorsFromData};
use waterui_graphics::draw::{
    FontId, ImageId, ImageLimits, ImageStorage, Paint, Picture, TextLayoutId,
};
use waterui_graphics::resources::{Handle, ResourceHandle, SceneBackend};
use waterui_graphics::source::{
    FontSource, Format, ImageColorSpace, ImageData, ResourceError, Rgba8, Rgba16F,
};

/// The largest side of an image a layer composites: the texture limit of
/// every GPU Core Animation runs on.
const MAX_IMAGE_DIMENSION: u32 = 16_384;

/// What registrations hold, by id.
#[derive(Default)]
pub struct Registry {
    next: Cell<u64>,
    images: RefCell<HashMap<u64, CFRetained<CGImage>>>,
    fonts: RefCell<HashMap<u64, CFRetained<CTFontDescriptor>>>,
    texts: RefCell<HashMap<u64, LaidOutText>>,
}

impl core::fmt::Debug for Registry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Registry")
            .field("images", &self.images.borrow().len())
            .field("fonts", &self.fonts.borrow().len())
            .field("texts", &self.texts.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Registry {
    fn fresh(&self) -> u64 {
        let id = self.next.get() + 1;
        self.next.set(id);
        id
    }

    /// Registers `descriptor` as a font, for a glyph run to name.
    pub(super) fn add_font(&self, descriptor: CFRetained<CTFontDescriptor>) -> FontId {
        let id = self.fresh();
        self.fonts.borrow_mut().insert(id, descriptor);
        FontId::new(id)
    }

    /// Registers a laid-out text under a fresh id.
    pub(super) fn add_text(&self, text: LaidOutText) -> TextLayoutId {
        let id = self.fresh();
        self.texts.borrow_mut().insert(id, text);
        TextLayoutId::new(id)
    }

    /// The commands a text draws, as a picture: `paint` takes the place of
    /// the colour the text draws in by default.
    ///
    /// # Panics
    ///
    /// When no live registration has that id.
    #[must_use]
    pub fn text(&self, id: TextLayoutId, paint: Option<&Paint>) -> Picture {
        self.texts
            .borrow()
            .get(&id.raw())
            .unwrap_or_else(|| {
                panic!(
                    "text layout {} is named by a recording but not registered",
                    id.raw()
                )
            })
            .picture(paint)
    }

    /// The image registered as `id`.
    ///
    /// # Panics
    ///
    /// When no live registration has that id: a recording names only what
    /// it holds.
    #[must_use]
    pub fn image(&self, id: ImageId) -> CFRetained<CGImage> {
        self.images
            .borrow()
            .get(&id.raw())
            .unwrap_or_else(|| {
                panic!(
                    "image {} is named by a recording but not registered",
                    id.raw()
                )
            })
            .clone()
    }

    /// The font descriptor registered as `id`.
    ///
    /// # Panics
    ///
    /// When no live registration has that id.
    #[must_use]
    pub fn font(&self, id: FontId) -> CFRetained<CTFontDescriptor> {
        self.fonts
            .borrow()
            .get(&id.raw())
            .unwrap_or_else(|| {
                panic!(
                    "font {} is named by a recording but not registered",
                    id.raw()
                )
            })
            .clone()
    }
}

/// An image registration, released with its handle.
struct ImageHandle {
    id: ImageId,
    registry: Weak<Registry>,
}

impl ResourceHandle for ImageHandle {
    type Id = ImageId;

    fn id(&self) -> ImageId {
        self.id
    }
}

impl Drop for ImageHandle {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.images.borrow_mut().remove(&self.id.raw());
        }
    }
}

/// A font registration, released with its handle.
struct FontHandle {
    id: FontId,
    registry: Weak<Registry>,
}

impl ResourceHandle for FontHandle {
    type Id = FontId;

    fn id(&self) -> FontId {
        self.id
    }
}

impl Drop for FontHandle {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.fonts.borrow_mut().remove(&self.id.raw());
        }
    }
}

/// A laid-out text's registration, released with its handle together with
/// the fonts its glyph runs name.
pub(super) struct TextHandle {
    id: TextLayoutId,
    registry: Weak<Registry>,
}

impl TextHandle {
    /// The registration of `text` in `registry`.
    pub(super) fn new(registry: &Rc<Registry>, text: LaidOutText) -> Self {
        Self {
            id: registry.add_text(text),
            registry: Rc::downgrade(registry),
        }
    }
}

impl ResourceHandle for TextHandle {
    type Id = TextLayoutId;

    fn id(&self) -> TextLayoutId {
        self.id
    }
}

impl Drop for TextHandle {
    fn drop(&mut self) {
        let Some(registry) = self.registry.upgrade() else {
            return;
        };
        let text = registry.texts.borrow_mut().remove(&self.id.raw());
        if let Some(text) = text {
            let mut fonts = registry.fonts.borrow_mut();
            for font in text.fonts() {
                fonts.remove(&font.raw());
            }
        }
    }
}

/// The Apple target's registration, over a [`Registry`].
pub struct Backend {
    registry: Rc<Registry>,
    limits: ImageLimits,
}

impl core::fmt::Debug for Backend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Backend")
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl Backend {
    /// A target registering into `registry`, with `max_bytes` for all its
    /// images together.
    #[must_use]
    pub const fn new(registry: Rc<Registry>, max_bytes: u64) -> Self {
        Self {
            registry,
            limits: ImageLimits {
                max_dimension: MAX_IMAGE_DIMENSION,
                max_bytes,
                storage: ImageStorage::GPU,
            },
        }
    }

    fn color_space(color_space: ImageColorSpace, float: bool) -> CFRetained<CGColorSpace> {
        // SAFETY: the statics are `CFString` constants Core Graphics exports.
        let name: &CFString = unsafe {
            match (color_space, float) {
                (ImageColorSpace::Srgb, false) => kCGColorSpaceSRGB,
                (ImageColorSpace::DisplayP3, false) => kCGColorSpaceDisplayP3,
                (ImageColorSpace::LinearSrgb, false) => kCGColorSpaceLinearSRGB,
                (ImageColorSpace::LinearP3, false) => kCGColorSpaceLinearDisplayP3,
                (ImageColorSpace::Srgb, true) => kCGColorSpaceExtendedSRGB,
                (ImageColorSpace::DisplayP3, true) => kCGColorSpaceExtendedDisplayP3,
                (ImageColorSpace::LinearSrgb, true) => kCGColorSpaceExtendedLinearSRGB,
                (ImageColorSpace::LinearP3, true) => kCGColorSpaceExtendedLinearDisplayP3,
            }
        };
        CGColorSpace::with_name(Some(name))
            .expect("Core Graphics provides every image colour space")
    }

    fn image<F: Format>(
        &self,
        data: &ImageData<F>,
        float: bool,
    ) -> Result<Handle<ImageId>, ResourceError> {
        let (bits, pixel) = if float { (16, 64) } else { (8, 32) };
        let alpha = if data.premultiplied {
            CGImageAlphaInfo::PremultipliedLast
        } else {
            CGImageAlphaInfo::Last
        };
        let info = if float {
            CGBitmapInfo(
                alpha.0 | CGImageComponentInfo::Float.0 | CGImageByteOrderInfo::Order16Little.0,
            )
        } else {
            CGBitmapInfo(alpha.0)
        };
        let bytes = CFData::from_bytes(data.data());
        let provider = CGDataProvider::with_cf_data(Some(&bytes))
            .ok_or_else(|| ResourceError::Image("Core Graphics refused the pixel data".into()))?;
        let space = Self::color_space(data.color_space, float);
        // SAFETY: a null decode array is the identity; every other argument
        // describes the layout `data` was validated to carry.
        let image = unsafe {
            CGImage::new(
                data.width() as usize,
                data.height() as usize,
                bits,
                pixel,
                data.width() as usize * pixel / 8,
                Some(&space),
                info,
                Some(&provider),
                core::ptr::null(),
                true,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
        .ok_or_else(|| {
            ResourceError::Image(format!(
                "Core Graphics refused a {}x{} {:?} image",
                data.width(),
                data.height(),
                F::FORMAT
            ))
        })?;
        let id = self.registry.fresh();
        self.registry.images.borrow_mut().insert(id, image);
        Ok(Handle::new(ImageHandle {
            id: ImageId::new(id),
            registry: Rc::downgrade(&self.registry),
        }))
    }
}

impl SceneBackend for Backend {
    fn register_font(&self, source: FontSource) -> Result<Handle<FontId>, ResourceError> {
        let descriptor = match source {
            FontSource::Bytes { data, index } => {
                let bytes = CFData::from_bytes(&data);
                // SAFETY: `bytes` is a live `CFData`.
                let descriptors = unsafe { CTFontManagerCreateFontDescriptorsFromData(&bytes) };
                // SAFETY: the array holds `CTFontDescriptor`s.
                let descriptors = unsafe { descriptors.cast_unchecked::<CTFontDescriptor>() };
                descriptors.get(index as usize).ok_or_else(|| {
                    ResourceError::Font(format!(
                        "the file has {} faces, not a face {index}",
                        descriptors.len()
                    ))
                })?
            }
            FontSource::System { family } => {
                let name = CFString::from_str(&family);
                // SAFETY: the name is a live string and the size is plain.
                unsafe { CTFontDescriptor::with_name_and_size(&name, 0.0) }
            }
            _ => return Err(ResourceError::Unsupported("font source")),
        };
        let id = self.registry.fresh();
        self.registry.fonts.borrow_mut().insert(id, descriptor);
        Ok(Handle::new(FontHandle {
            id: FontId::new(id),
            registry: Rc::downgrade(&self.registry),
        }))
    }

    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Handle<ImageId>, ResourceError> {
        self.image(&data, false)
    }

    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Handle<ImageId>, ResourceError> {
        self.image(&data, true)
    }

    fn image_limits(&self) -> ImageLimits {
        self.limits
    }
}
