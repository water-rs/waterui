//! The semantic resources a canvas draws: images, fonts and glyph runs.
//!
//! A resource is described, never registered, by the author. The render
//! target that draws the canvas registers it on first use, holds it while a
//! recording references it, and registers it again on a new engine
//! generation. What the target made of an image reaches the app through
//! [`ImageSource::status`].

use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::marker::PhantomData;
use std::sync::Mutex;

use nami::watcher::{Context, WatcherGuard};
use nami::{Computed, Signal, impl_constant};
use send_wrapper::SendWrapper;

use crate::canvas::paint::StrokeStyle;
use crate::draw::{self, ImageData, ResourceError, Rgba8, Rgba16F};
use crate::source::FontSource;

/// How an image's texels are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// Four 8-bit channels, red first.
    Rgba8,
    /// Four 16-bit float channels, red first: the format for wide-gamut and
    /// high-dynamic-range images.
    Rgba16F,
}

/// The colour space an image's texels are encoded in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ImageColorSpace {
    /// sRGB primaries and transfer. The default.
    #[default]
    Srgb,
    /// Display P3 primaries with the sRGB transfer.
    DisplayP3,
    /// Linear sRGB, extended range allowed.
    LinearSrgb,
    /// Linear Display P3, extended range allowed: the working space.
    LinearDisplayP3,
}

impl ImageColorSpace {
    const fn lower(self) -> draw::ImageColorSpace {
        match self {
            Self::Srgb => draw::ImageColorSpace::Srgb,
            Self::DisplayP3 => draw::ImageColorSpace::DisplayP3,
            Self::LinearSrgb => draw::ImageColorSpace::LinearSrgb,
            Self::LinearDisplayP3 => draw::ImageColorSpace::LinearP3,
        }
    }
}

/// A buffer of pixels and how to read them: what an [`ImageSource`] is made
/// from.
#[derive(Clone, Debug)]
pub struct PixelBuffer {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// How each texel is stored.
    pub format: PixelFormat,
    /// The colour space the texels are encoded in.
    pub color_space: ImageColorSpace,
    /// Whether the colour channels are premultiplied by alpha.
    pub premultiplied: bool,
    /// The texels, row by row with no padding.
    pub pixels: Arc<[u8]>,
}

impl PixelBuffer {
    /// Straight-alpha sRGB pixels in `format`.
    #[must_use]
    pub fn new(width: u32, height: u32, format: PixelFormat, pixels: impl Into<Arc<[u8]>>) -> Self {
        Self {
            width,
            height,
            format,
            color_space: ImageColorSpace::Srgb,
            premultiplied: false,
            pixels: pixels.into(),
        }
    }
}

/// The pixels of an image source, validated in their storage format.
#[derive(Clone, Debug)]
pub enum Pixels {
    Rgba8(ImageData<Rgba8>),
    Rgba16F(ImageData<Rgba16F>),
}

/// What the render target made of an image.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ImageStatus {
    /// No target has tried to register it yet.
    Pending,
    /// The target registered it and draws it.
    Ready,
    /// The target refused it — too large for the device, over the image
    /// budget, or unreadable — and draws nothing in its place.
    Refused(Arc<ResourceError>),
}

impl PartialEq for ImageStatus {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Pending, Self::Pending) | (Self::Ready, Self::Ready) => true,
            (Self::Refused(a), Self::Refused(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

/// A watcher of an image's status, callable only on the thread that
/// registered it.
type StatusWatcher = SendWrapper<Rc<dyn Fn(Context<ImageStatus>)>>;

struct StatusState {
    status: ImageStatus,
    next: u64,
    watchers: Vec<(u64, StatusWatcher)>,
}

struct ImageInner {
    pixels: Pixels,
    state: Mutex<StatusState>,
}

/// An image a canvas draws: a shared pixel buffer.
///
/// Cloning shares the buffer. Every clone is the same image to a render
/// target, which registers it once however many recordings draw it, and two
/// sources made from the same pixel allocation share one registration too.
/// It is `Send`, so an image decoded on a worker thread is drawn as it is.
#[derive(Clone)]
pub struct ImageSource(Arc<ImageInner>);

impl fmt::Debug for ImageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ImageSource").field(&self.0.pixels).finish()
    }
}

impl PartialEq for ImageSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl ImageSource {
    /// An image of `buffer`'s pixels.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when a dimension is zero or the buffer's
    /// length does not match its dimensions and format.
    pub fn new(buffer: PixelBuffer) -> Result<Self, ResourceError> {
        let PixelBuffer {
            width,
            height,
            format,
            color_space,
            premultiplied,
            pixels,
        } = buffer;
        let pixels = match format {
            PixelFormat::Rgba8 => {
                let mut data = ImageData::<Rgba8>::new(width, height, pixels)?
                    .color_space(color_space.lower());
                data.premultiplied = premultiplied;
                Pixels::Rgba8(data)
            }
            PixelFormat::Rgba16F => {
                let mut data = ImageData::<Rgba16F>::new(width, height, pixels)?
                    .color_space(color_space.lower());
                data.premultiplied = premultiplied;
                Pixels::Rgba16F(data)
            }
        };
        Ok(Self(Arc::new(ImageInner {
            pixels,
            state: Mutex::new(StatusState {
                status: ImageStatus::Pending,
                next: 0,
                watchers: Vec::new(),
            }),
        })))
    }

    /// Width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        match &self.0.pixels {
            Pixels::Rgba8(data) => data.width(),
            Pixels::Rgba16F(data) => data.width(),
        }
    }

    /// Height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        match &self.0.pixels {
            Pixels::Rgba8(data) => data.height(),
            Pixels::Rgba16F(data) => data.height(),
        }
    }

    /// What the render target made of this image, as a signal: pending
    /// until a target first draws it, then ready or refused. A refusal is
    /// how an image too large for the device, or over the target's image
    /// budget, reaches the app.
    #[must_use]
    pub fn status(&self) -> Computed<ImageStatus> {
        Computed::new(StatusSignal(self.clone()))
    }

    pub(crate) fn pixels(&self) -> &Pixels {
        &self.0.pixels
    }

    /// The identity a render target registers the image by.
    pub(crate) fn key(&self) -> usize {
        Arc::as_ptr(&self.0).addr()
    }

    /// Records what the render target made of the image, notifying the
    /// status watchers registered on this thread.
    pub(crate) fn set_status(&self, status: &ImageStatus) {
        let watchers: Vec<Rc<dyn Fn(Context<ImageStatus>)>> = {
            let mut state = self.0.state.lock().expect(
                "the image status lock is held only for plain field updates, which cannot panic",
            );
            if state.status == *status {
                return;
            }
            state.status = status.clone();
            state
                .watchers
                .iter()
                .filter(|(_, watcher)| watcher.valid())
                .map(|(_, watcher)| Rc::clone(watcher))
                .collect()
        };
        for watcher in watchers {
            watcher(Context::from(status.clone()));
        }
    }
}

/// [`ImageSource::status`] as a signal.
#[derive(Clone)]
struct StatusSignal(ImageSource);

/// Keeps a status watcher registered; it never leaves the thread that
/// registered it, so the watcher is dropped where it was made.
struct StatusGuard {
    image: ImageSource,
    id: u64,
    _thread_bound: PhantomData<Rc<()>>,
}

impl Drop for StatusGuard {
    fn drop(&mut self) {
        let removed = {
            let mut state = self.image.0.state.lock().expect(
                "the image status lock is held only for plain field updates, which cannot panic",
            );
            state
                .watchers
                .iter()
                .position(|(id, _)| *id == self.id)
                .map(|at| state.watchers.swap_remove(at))
        };
        drop(removed);
    }
}

impl WatcherGuard for StatusGuard {}

impl Signal for StatusSignal {
    type Output = ImageStatus;
    type Guard = StatusGuard;

    fn snapshot(&self) -> ImageStatus {
        self.0
            .0
            .state
            .lock()
            .expect(
                "the image status lock is held only for plain field updates, which cannot panic",
            )
            .status
            .clone()
    }

    fn watch(&self, watcher: impl Fn(Context<ImageStatus>) + 'static) -> StatusGuard {
        let watcher: Rc<dyn Fn(Context<ImageStatus>)> = Rc::new(watcher);
        let id = {
            let mut state = self.0.0.state.lock().expect(
                "the image status lock is held only for plain field updates, which cannot panic",
            );
            let id = state.next;
            state.next += 1;
            state.watchers.push((id, SendWrapper::new(watcher)));
            id
        };
        StatusGuard {
            image: self.0.clone(),
            id,
            _thread_bound: PhantomData,
        }
    }
}

#[derive(Debug)]
enum FontInner {
    Bytes { data: Arc<[u8]>, index: u32 },
    System { family: String },
}

/// A font a glyph run draws with: a font file's bytes and the face's index
/// in its collection, or a family of the platform's font stack.
///
/// Cloning shares the font; every clone is one registration on a target.
#[derive(Clone, Debug)]
pub struct Font(Arc<FontInner>);

impl PartialEq for Font {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Font {
    /// The face at `index` of the font file `data`.
    #[must_use]
    pub fn from_bytes(data: impl Into<Arc<[u8]>>, index: u32) -> Self {
        Self(Arc::new(FontInner::Bytes {
            data: data.into(),
            index,
        }))
    }

    /// A family of the platform's font stack. A target without a platform
    /// font stack refuses it.
    #[must_use]
    pub fn system(family: impl Into<String>) -> Self {
        Self(Arc::new(FontInner::System {
            family: family.into(),
        }))
    }

    pub(crate) fn source(&self) -> FontSource {
        match &*self.0 {
            FontInner::Bytes { data, index } => {
                FontSource::bytes(Arc::clone(data)).with_index(*index)
            }
            FontInner::System { family } => FontSource::system(family.clone()),
        }
    }

    /// The identity a render target registers the font by.
    pub(crate) fn key(&self) -> usize {
        Arc::as_ptr(&self.0).addr()
    }
}

/// One positioned glyph of a [`GlyphRun`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glyph {
    /// The glyph's index in the font.
    pub id: u32,
    /// The x of the glyph's origin, in canvas space.
    pub x: f32,
    /// The y of the glyph's origin (its baseline), in canvas space.
    pub y: f32,
}

/// Whether a glyph run's outlines are filled or stroked.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum GlyphStyle {
    /// Filled outlines. The default.
    #[default]
    Fill,
    /// Stroked outlines, as for a text halo.
    Stroke(StrokeStyle),
}

/// Glyphs of one font at one size, drawn exactly as positioned: no shaping,
/// no layout. What a math typesetter or a map labeller produces.
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphRun {
    /// The font.
    pub font: Font,
    /// The size, in points per em.
    pub size: f32,
    /// The variable-font axis coordinates, normalized to -1..=1 in the
    /// font's axis order; empty for the default instance.
    pub coords: Arc<[f32]>,
    /// The glyphs.
    pub glyphs: Arc<[Glyph]>,
    /// Fill or stroke.
    pub style: GlyphStyle,
}

impl GlyphRun {
    /// `glyphs` of `font` at `size`, filled, at the default instance.
    #[must_use]
    pub fn new(font: Font, size: f32, glyphs: impl Into<Arc<[Glyph]>>) -> Self {
        Self {
            font,
            size,
            coords: Arc::from([]),
            glyphs: glyphs.into(),
            style: GlyphStyle::Fill,
        }
    }

    /// Sets the variable-font coordinates.
    #[must_use]
    pub fn coords(mut self, coords: impl Into<Arc<[f32]>>) -> Self {
        self.coords = coords.into();
        self
    }

    /// Sets fill or stroke.
    #[must_use]
    pub fn style(mut self, style: GlyphStyle) -> Self {
        self.style = style;
        self
    }

    /// The recorded run, drawing with `font`.
    pub(crate) fn lower(&self, font: draw::FontId) -> draw::GlyphRun {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "F2Dot14 holds -2..2; a normalized coordinate is clamped to -1..=1 first"
        )]
        let coords: Arc<[i16]> = self
            .coords
            .iter()
            .map(|coord| (coord.clamp(-1.0, 1.0) * 16384.0).round() as i16)
            .collect();
        draw::GlyphRun {
            font,
            size: self.size,
            coords,
            glyphs: self
                .glyphs
                .iter()
                .map(|glyph| draw::Glyph {
                    id: glyph.id,
                    x: glyph.x,
                    y: glyph.y,
                    transform: None,
                })
                .collect(),
            style: match &self.style {
                GlyphStyle::Fill => draw::GlyphStyle::Fill,
                GlyphStyle::Stroke(stroke) => draw::GlyphStyle::Stroke(stroke.lower()),
            },
        }
    }
}

impl_constant!(ImageSource, Font, GlyphRun, ImageStatus);
