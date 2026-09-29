//! Engine-scoped resource registration for scene content.
//!
//! A host builds one [`SceneResources`] over the engine it already selected —
//! the GPU engine its surfaces render through, or the CPU raster engine an
//! offscreen rasterizer owns — keeps it for as long as that engine lives, and
//! hands it to every [`SceneContent::build_scene`] call. Content registers a
//! font, an image or a shader paint in the frame that first draws it, and
//! keeps the returned [`Registered`] handle for as long as its recordings name
//! the resource.
//!
//! Registrations live exactly as long as somebody holds them. A
//! [`Registered`] handle is the only strong owner of its registration: the
//! table keeps a weak entry per registration for deduplication, never a strong
//! one, so when the last content holding a handle lets go of it the entry
//! leaves the table and the engine's own handle drops, which unregisters the
//! resource. Nothing waits for the content to detach or for the table to be
//! swept; the release is the handle's drop, the same `Rc` semantics the
//! engine's handles already have.
//!
//! The type deliberately carries no drawing methods, no layer operations and
//! no backend choice: the engine is the host's, selected before this exists.
//! What this adds on top of `Engine` is deduplication — two contents drawing
//! the same font or image while both hold it share one registration — and a
//! uniform surface for backends with differing capabilities: a shader paint
//! on a backend without shader support is an explicit
//! [`ResourceError::Unsupported`], never a silent miss.
//!
//! [`SceneContent::build_scene`]: crate::scene_view::SceneContent::build_scene

use alloc::rc::{Rc, Weak};
use core::cell::RefCell;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::Deref;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;

use cherenkov::{
    Backend, Engine, Font, FontSource, Image, ImageData, ResourceError, Rgba8, Rgba16F, Shader,
    ShaderPaintCapability, ShaderSource, Uploads,
};

/// Identity of a registered font: the data's content hash plus the index in
/// its collection, so two `Arc` copies of the same bytes share one
/// registration while a reused `Arc` address can never alias one.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FontKey {
    hash: u64,
    index: u32,
}

/// Identity of a registered image: the data's content hash plus the upload's
/// full shape — the same bytes at a different size are a different image.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ImageKey {
    hash: u64,
    width: u32,
    height: u32,
}

/// Identity of a registered shader: the source text's content hash plus the
/// `animated` flag — a static shader re-registered as animated would freeze
/// mid-frame if the two collided.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ShaderKey {
    hash: u64,
    animated: bool,
}

/// Which table entry a registration occupies, so its last handle can take
/// the entry out when it drops.
#[derive(Clone, Copy)]
enum EntryKey {
    Font(FontKey),
    Rgba8(ImageKey),
    Rgba16F(ImageKey),
    Shader(ShaderKey),
}

fn content_hash(bytes: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// The registration an engine always provides: fonts and image uploads.
///
/// Implemented for `cherenkov::Engine<B>` where `B` uploads both image
/// formats; `SceneResources` holds it as a trait object so the table's
/// deduplication is backend-agnostic.
pub trait SceneBackend {
    /// Registers `source` with the engine, minting a live [`Font`] handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Font`] when the data cannot be used,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_font(&self, source: FontSource) -> Result<Font, ResourceError>;

    /// Uploads `data`, minting a live [`Image`] handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError>;

    /// Uploads `data` in the HDR/linear format, minting a live [`Image`]
    /// handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Image<Rgba16F>, ResourceError>;
}

impl<B> SceneBackend for Engine<B>
where
    B: Backend + Uploads<Rgba8> + Uploads<Rgba16F>,
{
    fn register_font(&self, source: FontSource) -> Result<Font, ResourceError> {
        Self::font(self, source)
    }

    fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Image<Rgba8>, ResourceError> {
        Self::image(self, data)
    }

    fn register_rgba16f(&self, data: ImageData<Rgba16F>) -> Result<Image<Rgba16F>, ResourceError> {
        Self::image(self, data)
    }
}

/// The registration only a shader-paint backend provides.
pub trait ShaderBackend: SceneBackend {
    /// Registers `source` with the engine, minting a live [`Shader`] handle.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Shader`] when the source fails validation,
    /// [`ResourceError::Lost`] when the render thread is gone.
    fn register_shader(&self, source: ShaderSource) -> Result<Shader, ResourceError>;
}

impl<B> ShaderBackend for Engine<B>
where
    B: Backend + Uploads<Rgba8> + Uploads<Rgba16F> + ShaderPaintCapability,
{
    fn register_shader(&self, source: ShaderSource) -> Result<Shader, ResourceError> {
        Self::shader(self, source)
    }
}

/// What a backend offers beyond the unconditional registration, resolved on
/// the backend type so one `SceneResources::new` serves every engine.
mod sealed {
    use alloc::rc::Rc;

    use cherenkov::{Backend, Engine};

    /// Per-backend capabilities [`SceneResources`](super::SceneResources)
    /// surfaces.
    ///
    /// A backend declares them here; under-declaring simply reports
    /// `Unsupported` at registration.
    pub trait SceneCaps: Backend {
        /// The shader registry, when `Self` accepts shader paints.
        fn shaders(engine: &Rc<Engine<Self>>) -> Option<Rc<dyn super::ShaderBackend>> {
            let _ = engine;
            None
        }
    }
}

pub use sealed::SceneCaps;

#[cfg(feature = "gpu")]
impl SceneCaps for cherenkov_gpu::Gpu {
    fn shaders(engine: &Rc<Engine<Self>>) -> Option<Rc<dyn ShaderBackend>> {
        let shaders: Rc<dyn ShaderBackend> = Rc::<Engine<Self>>::clone(engine);
        Some(shaders)
    }
}

#[cfg(feature = "cpu")]
impl SceneCaps for cherenkov_cpu::Raster {}

#[cfg(test)]
impl SceneCaps for cherenkov::testing::Null {
    fn shaders(engine: &Rc<Engine<Self>>) -> Option<Rc<dyn ShaderBackend>> {
        let shaders: Rc<dyn ShaderBackend> = Rc::<Engine<Self>>::clone(engine);
        Some(shaders)
    }
}

/// One live registration: the engine's handle plus the table entry that
/// dedupes it. Dropping it takes the entry out of the table, then drops the
/// engine handle, which unregisters the resource.
struct Entry<H> {
    handle: H,
    key: EntryKey,
    table: Weak<Table>,
}

impl<H> Drop for Entry<H> {
    fn drop(&mut self) {
        if let Some(table) = self.table.upgrade() {
            table.unlist(self.key);
        }
    }
}

/// A resource registered through [`SceneResources`], shared by everyone who
/// asked for the same source while it was held.
///
/// This is the registration's owner: cloning it shares the registration, and
/// dropping the last clone releases it — the table's entry leaves with it, and
/// the engine unregisters the resource. It dereferences to the engine handle
/// (`Font`, `Image<F>` or `Shader`), whose `id()` is what a recording names.
pub struct Registered<H> {
    entry: Rc<Entry<H>>,
}

impl<H> Clone for Registered<H> {
    fn clone(&self) -> Self {
        Self {
            entry: Rc::clone(&self.entry),
        }
    }
}

impl<H> Deref for Registered<H> {
    type Target = H;

    fn deref(&self) -> &H {
        &self.entry.handle
    }
}

impl<H: fmt::Debug> fmt::Debug for Registered<H> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Registered")
            .field(&self.entry.handle)
            .finish()
    }
}

/// A deduplication map from source identity to the live registration.
type Listing<K, H> = RefCell<HashMap<K, Weak<Entry<H>>>>;

/// The shared state behind [`SceneResources`]; entries reach it weakly to
/// take themselves out.
struct Table {
    backend: Rc<dyn SceneBackend>,
    shaders: Option<Rc<dyn ShaderBackend>>,
    fonts: Listing<FontKey, Font>,
    images_rgba8: Listing<ImageKey, Image<Rgba8>>,
    images_rgba16f: Listing<ImageKey, Image<Rgba16F>>,
    shaders_cache: Listing<ShaderKey, Shader>,
}

impl Table {
    /// Removes the entry of a registration whose last handle is dropping.
    fn unlist(&self, key: EntryKey) {
        match key {
            EntryKey::Font(key) => {
                self.fonts.borrow_mut().remove(&key);
            }
            EntryKey::Rgba8(key) => {
                self.images_rgba8.borrow_mut().remove(&key);
            }
            EntryKey::Rgba16F(key) => {
                self.images_rgba16f.borrow_mut().remove(&key);
            }
            EntryKey::Shader(key) => {
                self.shaders_cache.borrow_mut().remove(&key);
            }
        }
    }
}

/// Resource registration over one engine, shared by the content drawn on it.
///
/// Constructed once from the host's already-selected engine and handed to
/// every [`SceneContent::build_scene`] on that engine; see the module
/// documentation for the ownership contract. The table holds the engine
/// strongly and its registrations weakly: it never keeps a resource alive,
/// so it can live as long as the engine does without pinning anything a
/// content has stopped drawing.
///
/// [`SceneContent::build_scene`]: crate::scene_view::SceneContent::build_scene
pub struct SceneResources {
    table: Rc<Table>,
}

impl SceneResources {
    /// Resource registration over `engine`, the host's already-selected one.
    ///
    /// `B` declares which optional registries exist through its capability
    /// implementations: an engine that accepts shader paints gets them, an
    /// engine without the capability reports
    /// [`ResourceError::Unsupported`] instead.
    pub fn new<B>(engine: Rc<Engine<B>>) -> Self
    where
        B: SceneCaps + Uploads<Rgba8> + Uploads<Rgba16F>,
    {
        let shaders = B::shaders(&engine);
        Self {
            table: Rc::new(Table {
                backend: engine,
                shaders,
                fonts: RefCell::new(HashMap::new()),
                images_rgba8: RefCell::new(HashMap::new()),
                images_rgba16f: RefCell::new(HashMap::new()),
                shaders_cache: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// The live registration listed under `key`, or a new one from
    /// `register` listed there while it lives.
    fn intern<K, H>(
        &self,
        listing: fn(&Table) -> &Listing<K, H>,
        key: K,
        entry_key: EntryKey,
        register: impl FnOnce(&Table) -> Result<H, ResourceError>,
    ) -> Result<Registered<H>, ResourceError>
    where
        K: Hash + Eq,
    {
        let live = listing(&self.table)
            .borrow()
            .get(&key)
            .and_then(Weak::upgrade);
        if let Some(entry) = live {
            return Ok(Registered { entry });
        }
        let entry = Rc::new(Entry {
            handle: register(&self.table)?,
            key: entry_key,
            table: Rc::downgrade(&self.table),
        });
        listing(&self.table)
            .borrow_mut()
            .insert(key, Rc::downgrade(&entry));
        Ok(Registered { entry })
    }

    /// Registers `source` with the engine, returning the shared handle.
    ///
    /// While any handle to it is held, identical sources — same data and
    /// same collection index — map to that one registration.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Font`] when the data cannot be used,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn font(&self, source: FontSource) -> Result<Registered<Font>, ResourceError> {
        let key = FontKey {
            hash: content_hash(&source.data),
            index: source.index,
        };
        self.intern(
            |table| &table.fonts,
            key,
            EntryKey::Font(key),
            |table| table.backend.register_font(source),
        )
    }

    /// Registers `Rgba8` image data.
    ///
    /// [`ImageData::new`] validates the dimensions and byte length before the
    /// engine ever sees the upload; identical images share one registration
    /// while any handle to it is held.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn image(&self, data: ImageData<Rgba8>) -> Result<Registered<Image<Rgba8>>, ResourceError> {
        let key = ImageKey {
            hash: content_hash(&data.data),
            width: data.width,
            height: data.height,
        };
        self.intern(
            |table| &table.images_rgba8,
            key,
            EntryKey::Rgba8(key),
            |table| table.backend.register_rgba8(data),
        )
    }

    /// Registers `Rgba16Float` image data — the format HDR and linear-space
    /// sources upload as.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Image`] when the backend rejects the upload,
    /// [`ResourceError::Lost`] when the render thread is gone.
    pub fn image16f(
        &self,
        data: ImageData<Rgba16F>,
    ) -> Result<Registered<Image<Rgba16F>>, ResourceError> {
        let key = ImageKey {
            hash: content_hash(&data.data),
            width: data.width,
            height: data.height,
        };
        self.intern(
            |table| &table.images_rgba16f,
            key,
            EntryKey::Rgba16F(key),
            |table| table.backend.register_rgba16f(data),
        )
    }

    /// Registers a shader paint's WGSL source.
    ///
    /// # Errors
    ///
    /// [`ResourceError::Unsupported`] when the engine's backend does not draw
    /// shader paints, [`ResourceError::Shader`] when the source fails
    /// validation, [`ResourceError::Lost`] when the render thread is gone.
    pub fn shader(&self, source: ShaderSource) -> Result<Registered<Shader>, ResourceError> {
        let Some(backend) = self.table.shaders.clone() else {
            return Err(ResourceError::Unsupported("shader paint"));
        };
        let key = ShaderKey {
            hash: content_hash(source.source.as_bytes()),
            animated: source.animated,
        };
        self.intern(
            |table| &table.shaders_cache,
            key,
            EntryKey::Shader(key),
            move |_| backend.register_shader(source),
        )
    }
}

impl fmt::Debug for SceneResources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SceneResources")
            .field("shaders", &self.table.shaders.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use std::collections::HashSet;
    use std::sync::mpsc::{Receiver, channel};

    use cherenkov::kurbo::Rect;
    use cherenkov::testing::{Event, Null, NullConfig};
    use cherenkov::{
        Command, Draw as _, Engine, FrameTime, Image, ImageData, ImageId, Offscreen,
        OffscreenFormat, Recorder, Rgba8, Sampling, Surface, WorkingColor,
    };

    use super::{Registered, SceneResources};
    use crate::scene_view::SceneContent;

    fn null_engine() -> (Rc<Engine<Null>>, Receiver<Event>) {
        let (events, probe) = channel();
        let engine = Engine::<Null>::new(NullConfig {
            events,
            reject: HashSet::new(),
        })
        .expect("the null engine failed to start");
        (Rc::new(engine), probe)
    }

    /// A registration table over a `Null` engine, with the engine's event
    /// probe, for tests that draw content without caring about pixels.
    pub fn null_resources() -> (SceneResources, Receiver<Event>) {
        let (engine, probe) = null_engine();
        (SceneResources::new(engine), probe)
    }

    fn one_pixel() -> ImageData<Rgba8> {
        ImageData::<Rgba8>::new(1, 1, Vec::from([255, 0, 0, 255])).expect("valid image")
    }

    fn added_images(events: &[Event]) -> Vec<ImageId> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::AddImage(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    fn removed_images(events: &[Event]) -> Vec<ImageId> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::RemoveImage(id) => Some(*id),
                _ => None,
            })
            .collect()
    }

    /// Content whose first two frames are a plain fill, whose third frame
    /// first draws an image, whose fourth draws it again, and which draws
    /// the fill alone — and lets the image go — from its fifth frame on.
    struct LateImage {
        frame: u32,
        image: Option<Registered<Image<Rgba8>>>,
    }

    impl SceneContent for LateImage {
        fn build_scene(
            &mut self,
            recorder: &mut Recorder,
            resources: &SceneResources,
            width: f32,
            height: f32,
        ) -> bool {
            self.frame += 1;
            let bounds = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
            recorder.fill(bounds, WorkingColor::BLACK);
            if (3..=4).contains(&self.frame) {
                let image = self.image.get_or_insert_with(|| {
                    resources
                        .image(one_pixel())
                        .expect("image registration failed")
                });
                recorder.image(image.id(), bounds, Sampling::Nearest);
            } else {
                self.image = None;
            }
            false
        }
    }

    /// What one frame did: the image ids the installed recording draws, and
    /// the render-thread events from recording it through rendering it.
    struct FrameReport {
        drawn: Vec<ImageId>,
        events: Vec<Event>,
    }

    /// A host mounting one content on a `Null` engine's surface root.
    struct Mount {
        engine: Rc<Engine<Null>>,
        probe: Receiver<Event>,
        resources: SceneResources,
        surface: Surface<Null>,
    }

    impl Mount {
        fn new() -> Self {
            let (engine, probe) = null_engine();
            let surface = engine
                .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16))
                .expect("surface");
            let _ = probe.try_iter().count();
            Self {
                resources: SceneResources::new(Rc::clone(&engine)),
                engine,
                probe,
                surface,
            }
        }

        /// One host frame: record, install the recording on the mounted
        /// layer, render.
        fn frame(&self, content: &mut LateImage) -> FrameReport {
            let mut recorded = self.surface.record(|recorder| {
                content.build_scene(recorder, &self.resources, 8.0, 8.0);
            });
            let drawn = recorded
                .snapshot()
                .commands()
                .iter()
                .filter_map(|command| match command {
                    Command::Image { image, .. } => Some(*image),
                    _ => None,
                })
                .collect();
            self.surface.update(|tx| {
                tx[self.surface.root()].content(recorded);
            });
            self.engine.render(FrameTime::now()).expect("render");
            FrameReport {
                drawn,
                events: self.probe.try_iter().collect(),
            }
        }
    }

    /// The frame registered `id`, then installed and rendered a recording
    /// naming it — in that order on the render thread.
    fn assert_registered_then_drawn(report: &FrameReport, id: ImageId) {
        assert_eq!(
            report.drawn,
            [id],
            "the frame draws the image it registered"
        );
        let position = |wanted: fn(&Event) -> bool| report.events.iter().position(wanted);
        let added = position(|event| matches!(event, Event::AddImage(_))).expect("registered");
        let installed =
            position(|event| matches!(event, Event::SetContent(..))).expect("installed");
        let rendered = position(|event| matches!(event, Event::Frame(_))).expect("rendered");
        assert!(
            added < installed && installed < rendered,
            "the image must reach the engine before the recording naming it: {:?}",
            report.events
        );
    }

    #[test]
    fn content_registers_an_image_on_its_third_frame_and_releases_it_while_mounted() {
        let mount = Mount::new();
        let mut content = LateImage {
            frame: 0,
            image: None,
        };

        for _ in 0..2 {
            let report = mount.frame(&mut content);
            assert!(report.drawn.is_empty());
            assert!(
                added_images(&report.events).is_empty(),
                "nothing is registered before the content draws it"
            );
        }

        let third = mount.frame(&mut content);
        let registered = added_images(&third.events);
        assert_eq!(
            registered.len(),
            1,
            "the third frame registers its image exactly once: {:?}",
            third.events
        );
        let id = registered[0];
        assert_eq!(
            content.image.as_ref().map(|image| image.id()),
            Some(id),
            "the content holds the registration the engine committed"
        );
        assert_registered_then_drawn(&third, id);
        assert!(removed_images(&third.events).is_empty());

        // Asking for the same source while it is held shares the
        // registration instead of uploading it again.
        let shared = mount
            .resources
            .image(one_pixel())
            .expect("image registration");
        assert_eq!(shared.id(), id);
        drop(shared);

        let fourth = mount.frame(&mut content);
        assert_eq!(fourth.drawn, [id], "a held image keeps drawing");
        assert!(
            added_images(&fourth.events).is_empty(),
            "a held image is not registered again"
        );
        assert!(
            removed_images(&fourth.events).is_empty(),
            "dropping a shared clone must not release a registration the content still holds"
        );

        let fifth = mount.frame(&mut content);
        assert!(fifth.drawn.is_empty());
        assert_eq!(
            removed_images(&fifth.events),
            [id],
            "an image the content stopped holding is released while the content is mounted"
        );
        assert!(content.image.is_none());

        // The table kept no dead entry: the same source is a new
        // registration now, not the released one.
        let fresh = mount
            .resources
            .image(one_pixel())
            .expect("image registration");
        assert_ne!(fresh.id(), id);
        assert_eq!(
            added_images(&mount.probe.try_iter().collect::<Vec<_>>()),
            [fresh.id()]
        );
    }
}
