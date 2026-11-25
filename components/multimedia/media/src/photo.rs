//! A Photo component that displays an image from a URL.
//!
//! `Photo` fetches an image from a local or remote [`Url`], decodes it, caches
//! the decoded [`Image`] in the [`Environment`]'s [`PhotoCache`], and displays
//! it through a persistent [`ReactiveImage`] surface, with optional static
//! generic placeholder and failure views plus reactive [`PhotoStatus`]
//! reporting.
//!
//! # Example
//!
//! ```rust
//! use waterui_media::Photo;
//! use waterui_media::url::Url;
//!
//! let url = Url::parse("https://www.rust-lang.org/logos/rust-logo-512x512.png").unwrap();
//! let photo = Photo::new(url);
//! ```

use alloc::borrow::ToOwned;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::{Cell, RefCell};
use core::fmt;

use crate::Url;
use executor_core::spawn_local;
#[cfg(feature = "remote")]
use futures::StreamExt;
use lru::LruCache;
#[cfg(target_arch = "wasm32")]
use std::path::Path;
#[cfg(target_arch = "wasm32")]
use waterkit_fs::WaterFs;
use waterui_capture::ContentLedger;
use waterui_capture::content_ledger::OwedContent;
use waterui_core::contract_violation;
use waterui_core::dynamic::{Dynamic, DynamicHandler};
use waterui_core::handler::ReportExtraction as _;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::reactive::watcher::BoxWatcherGuard;
use waterui_core::{
    Binding, Computed, Environment, Metadata, OneOf2, OneOf3, Retain, Signal, SignalExt as _, Str,
    View,
};
use waterui_image::{
    ContentMode, Image, ImageStatus, ReactiveImage, ReactiveImageHandle, reactive_image,
};
use waterui_interaction::event::{LifeCycle, LifeCycleHook};
use waterui_layout::stack::zstack;
#[cfg(feature = "remote")]
use zenwave::{Client, Method};

/// Each decoded [`Image`] stores straight-alpha sRGB8 (`width * height * 4` bytes).
const RGBA8_BYTES_PER_PIXEL: usize = 4;

/// Default byte capacity of [`PhotoCache`]: 64 MiB of decoded RGBA8 pixels
/// (~8 full 1920x1080 frames or ~256 256x256 thumbnails). Operators override
/// this per environment with [`PhotoCache::new`] or at runtime with
/// [`PhotoCache::set_max_bytes`].
const DEFAULT_PHOTO_CACHE_MAX_BYTES: usize = 64 * 1024 * 1024;

/// Lifecycle state of a [`Photo`]'s current source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PhotoStatus {
    /// The photo is fetching, decoding, or registering its image.
    #[default]
    Loading,
    /// The decoded image is registered on the render target and ready.
    Ready,
    /// Fetching, decoding, or registering the image failed with this reason.
    Failed(Str),
}

impl PhotoStatus {
    /// Returns whether the photo is currently loading.
    #[must_use]
    pub const fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// Returns whether the photo's final frame is registered and ready.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    /// Returns the failure reason when loading failed.
    #[must_use]
    pub const fn failure(&self) -> Option<&str> {
        match self {
            Self::Failed(reason) => Some(reason.as_str()),
            Self::Loading | Self::Ready => None,
        }
    }
}

#[derive(Clone, Debug)]
struct CachedPhoto {
    image: Image,
    bytes: usize,
}

#[derive(Debug)]
struct PhotoCacheInner {
    max_bytes: usize,
    used_bytes: usize,
    entries: LruCache<Url, CachedPhoto>,
}

/// Shared bounded LRU cache of decoded [`Image`]s held in an [`Environment`].
///
/// Capacity is measured in decoded RGBA8 pixel bytes (`width * height * 4`) and
/// is an operator setting: install a custom budget with
/// `env.insert(PhotoCache::new(bytes))` or adjust it live with
/// [`PhotoCache::set_max_bytes`].
#[derive(Clone, Debug)]
pub struct PhotoCache {
    inner: Rc<RefCell<PhotoCacheInner>>,
}

impl Default for PhotoCache {
    fn default() -> Self {
        Self::new(DEFAULT_PHOTO_CACHE_MAX_BYTES)
    }
}

impl PhotoCache {
    /// Creates an empty cache bounded to `max_bytes` of decoded pixel data.
    #[must_use]
    pub fn new(max_bytes: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(PhotoCacheInner {
                max_bytes,
                used_bytes: 0,
                entries: LruCache::unbounded(),
            })),
        }
    }

    /// Current byte budget of this cache.
    #[must_use]
    pub fn max_bytes(&self) -> usize {
        self.inner.borrow().max_bytes
    }

    /// Adjusts the byte budget at runtime, immediately evicting least-recently
    /// used decoded images until [`Self::used_bytes`] fits within `max_bytes`.
    pub fn set_max_bytes(&self, max_bytes: usize) {
        let mut inner = self.inner.borrow_mut();
        inner.max_bytes = max_bytes;
        while inner.used_bytes > inner.max_bytes {
            let Some((_, evicted)) = inner.entries.pop_lru() else {
                inner.used_bytes = 0;
                break;
            };
            inner.used_bytes = inner.used_bytes.saturating_sub(evicted.bytes);
        }
    }

    /// Decoded pixel bytes currently retained in the cache.
    #[must_use]
    pub fn used_bytes(&self) -> usize {
        self.inner.borrow().used_bytes
    }

    /// Number of cached URLs currently retained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.borrow().entries.len()
    }

    /// Returns whether the cache holds no decoded images.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.borrow().entries.is_empty()
    }

    /// Drops all cached images.
    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.entries.clear();
        inner.used_bytes = 0;
    }

    pub(crate) fn contains(&self, url: &Url) -> bool {
        self.inner.borrow().entries.contains(url)
    }

    pub(crate) fn get(&self, url: &Url) -> Option<Image> {
        self.inner
            .borrow_mut()
            .entries
            .get(url)
            .map(|entry| entry.image.clone())
    }

    pub(crate) fn insert(&self, url: Url, image: Image) {
        let bytes = decoded_image_bytes(&image);
        let mut inner = self.inner.borrow_mut();
        if let Some(previous) = inner.entries.pop(&url) {
            inner.used_bytes = inner.used_bytes.saturating_sub(previous.bytes);
        }
        if bytes > inner.max_bytes {
            return;
        }
        while inner.used_bytes.saturating_add(bytes) > inner.max_bytes {
            let Some((_, evicted)) = inner.entries.pop_lru() else {
                inner.used_bytes = 0;
                break;
            };
            inner.used_bytes = inner.used_bytes.saturating_sub(evicted.bytes);
        }
        inner.used_bytes += bytes;
        inner.entries.put(url, CachedPhoto { image, bytes });
    }
}

fn decoded_image_bytes(image: &Image) -> usize {
    (image.width() as usize)
        .saturating_mul(image.height() as usize)
        .saturating_mul(RGBA8_BYTES_PER_PIXEL)
}

/// Static slot contract for [`Photo::placeholder`].
pub trait PlaceholderSlot: 'static {
    /// The concrete view type produced while loading.
    type View: View;
    /// Whether a placeholder view was configured.
    const PRESENT: bool;
    /// Builds the placeholder view for a loading source.
    fn build(&mut self) -> Self::View;
}

impl PlaceholderSlot for () {
    type View = ();
    const PRESENT: bool = false;

    fn build(&mut self) {}
}

impl<B, V> PlaceholderSlot for B
where
    B: FnMut() -> V + 'static,
    V: View,
{
    type View = V;
    const PRESENT: bool = true;

    fn build(&mut self) -> Self::View {
        (self)()
    }
}

/// Static slot contract for [`Photo::on_failure`].
pub trait FailureSlot: 'static {
    /// The concrete view type produced on load failure.
    type View: View;
    /// Whether a failure view was configured.
    const PRESENT: bool;
    /// Builds the failure view from the error message.
    fn build(&mut self, reason: &str) -> Self::View;
}

impl FailureSlot for () {
    type View = ();
    const PRESENT: bool = false;

    fn build(&mut self, _reason: &str) {}
}

impl<B, V> FailureSlot for B
where
    B: FnMut(&str) -> V + 'static,
    V: View,
{
    type View = V;
    const PRESENT: bool = true;

    fn build(&mut self, reason: &str) -> Self::View {
        (self)(reason)
    }
}

/// Source accepted by [`Photo::new`] and [`photo`]: a URL signal or an
/// environment-resolved handle (such as an `ImageAsset`).
pub struct PhotoSource(PhotoSourceInner);

type PhotoSourceResolver = Box<dyn FnOnce(&Environment) -> Option<Computed<Url>>>;

enum PhotoSourceInner {
    Url(Computed<Url>),
    Resolve(PhotoSourceResolver),
}

impl fmt::Debug for PhotoSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            PhotoSourceInner::Url(url) => f.debug_tuple("Url").field(url).finish(),
            PhotoSourceInner::Resolve(_) => f.write_str("Resolve(..)"),
        }
    }
}

impl PhotoSource {
    /// Creates a photo source from a URL or reactive signal of URLs.
    #[must_use]
    pub fn url(source: impl IntoComputed<Url>) -> Self {
        Self(PhotoSourceInner::Url(source.into_computed()))
    }

    /// Creates a photo source resolved from the view's [`Environment`] when
    /// the [`Photo`] mounts.
    #[must_use]
    pub fn from_env(
        resolver: impl FnOnce(&Environment) -> Option<Computed<Url>> + 'static,
    ) -> Self {
        Self(PhotoSourceInner::Resolve(Box::new(resolver)))
    }

    /// Resolves this photo source against `env`.
    #[must_use]
    pub fn resolve(self, env: &Environment) -> Option<Computed<Url>> {
        match self.0 {
            PhotoSourceInner::Url(url) => Some(url),
            PhotoSourceInner::Resolve(resolver) => resolver(env),
        }
    }
}

/// A handle that resolves to a photo [`Url`] against a context extracted once
/// from the rendering [`Environment`] (`()` for [`Url`] and `&'static str`;
/// `ResourceContext` for `ImageAsset`).
pub trait ResolvePhotoUrl: Clone + 'static {
    /// Context extracted once from [`Environment`] when the [`Photo`] mounts.
    type Context: Clone + 'static;

    /// Extracts the resolution context from `env`.
    fn context(env: &Environment) -> Option<Self::Context>;

    /// Resolves this handle into a photo [`Url`] using `context`.
    fn resolve(&self, context: &Self::Context) -> Url;
}

impl ResolvePhotoUrl for Url {
    type Context = ();

    fn context(_env: &Environment) -> Option<Self::Context> {
        Some(())
    }

    fn resolve(&self, (): &Self::Context) -> Url {
        self.clone()
    }
}

impl ResolvePhotoUrl for &'static str {
    type Context = ();

    fn context(_env: &Environment) -> Option<Self::Context> {
        Some(())
    }

    fn resolve(&self, (): &Self::Context) -> Url {
        Url::from(*self)
    }
}

/// Input accepted by [`Photo::new`] and [`photo`]: a URL (or signal of URLs)
/// or an asset handle resolved against the view's [`Environment`] at render time.
pub trait IntoPhotoSource {
    /// Converts this input into a [`PhotoSource`].
    fn into_photo_source(self) -> PhotoSource;
}

impl<S> IntoPhotoSource for S
where
    S: Signal + 'static,
    S::Output: ResolvePhotoUrl,
{
    fn into_photo_source(self) -> PhotoSource {
        PhotoSource::from_env(move |env| {
            let context = S::Output::context(env)?;
            Some(self.map(move |value| value.resolve(&context)).computed())
        })
    }
}

/// A photo component that displays an image from a URL or asset handle.
///
/// `Photo` fetches the image asynchronously, caches decoded frames in the
/// environment's [`PhotoCache`], and displays them using [`Image`] once loaded.
///
/// # Example
///
/// ```rust
/// use waterui_media::Photo;
/// use waterui_media::url::Url;
///
/// Photo::new(Url::parse("https://www.rust-lang.org/logos/rust-logo-512x512.png").unwrap());
/// ```
pub struct Photo<P = (), F = ()> {
    /// The image source to resolve and display.
    source: PhotoSource,
    /// Event handler for photo loading events.
    on_event: Option<waterui_core::handler::BoxedEventAction<Event>>,
    /// Caller-owned status binding updated across each source's load lifecycle.
    status: Option<Binding<PhotoStatus>>,
    /// Static placeholder builder shown while a source loads.
    placeholder: P,
    /// Static failure builder shown when a source fails.
    on_failure: F,
    /// Persistent image surface updated when decoded frames arrive.
    content: ReactiveImage,
    /// Handle for publishing decoded frames into `content`.
    content_handler: ReactiveImageHandle,
}

impl<P, F> core::fmt::Debug for Photo<P, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Photo")
            .field("source", &self.source)
            .field("on_event", &self.on_event.is_some())
            .field("status", &self.status.is_some())
            .field("content", &self.content)
            .field("content_handler", &self.content_handler)
            .finish_non_exhaustive()
    }
}

/// Events emitted by the Photo component.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// The picture is on screen: the source's final decoded frame registered
    /// with the scene backend. Registration settles after the frame is
    /// decoded — a frame the backend refuses reports [`Event::Error`]
    /// instead, never `Loaded`.
    Loaded,
    /// The image failed to load: the source did not decode, or the render
    /// target refused a frame's pixels. The message is the decode error or
    /// the refusal's [`Display`](core::fmt::Display).
    Error(String),
}

impl Photo<(), ()> {
    /// Creates a new `Photo` component from a URL signal or asset handle.
    #[must_use]
    pub fn new(source: impl IntoPhotoSource) -> Self {
        Self::from_source(source.into_photo_source())
    }

    /// Creates a new `Photo` component from an explicit [`PhotoSource`].
    #[must_use]
    pub fn from_source(source: PhotoSource) -> Self {
        let (content_handler, content) = reactive_image();
        Self {
            source,
            on_event: None,
            status: None,
            placeholder: (),
            on_failure: (),
            content,
            content_handler,
        }
    }

    /// Creates a `Photo` from a local filesystem path.
    #[must_use]
    pub fn from_path(path: impl Into<String>) -> Self {
        Self::new(Url::from_file_path_str(path.into()))
    }
}

impl<P, F> Photo<P, F> {
    /// Allows the decoded image to stretch to the bounds proposed by its parent.
    #[must_use]
    pub fn resizable(mut self) -> Self {
        self.content = self.content.resizable();
        self
    }

    /// Preserves the decoded picture's aspect ratio inside the bounds the
    /// parent proposes: [`ContentMode::Fit`] letterboxes, [`ContentMode::Fill`]
    /// covers the bounds and crops the overflow. Without it a resizable photo
    /// stretches on both axes independently.
    #[must_use]
    pub fn content_mode(mut self, mode: ContentMode) -> Self {
        self.content = self.content.content_mode(mode);
        self
    }

    /// Declares the placeholder view shown while a source is loading.
    #[must_use]
    pub fn placeholder<P2, V>(self, placeholder: P2) -> Photo<P2, F>
    where
        P2: FnMut() -> V + 'static,
        V: View,
    {
        Photo {
            source: self.source,
            on_event: self.on_event,
            status: self.status,
            placeholder,
            on_failure: self.on_failure,
            content: self.content,
            content_handler: self.content_handler,
        }
    }

    /// Declares the view shown when fetching, decoding, or registering a source
    /// fails, receiving the human-readable failure reason.
    #[must_use]
    pub fn on_failure<F2, V>(self, on_failure: F2) -> Photo<P, F2>
    where
        F2: FnMut(&str) -> V + 'static,
        V: View,
    {
        Photo {
            source: self.source,
            on_event: self.on_event,
            status: self.status,
            placeholder: self.placeholder,
            on_failure,
            content: self.content,
            content_handler: self.content_handler,
        }
    }

    /// Observes each source's load lifecycle through a caller-owned binding.
    ///
    /// Resets to [`PhotoStatus::Loading`] whenever the source URL changes.
    #[must_use]
    pub fn status(mut self, status: &Binding<PhotoStatus>) -> Self {
        self.status = Some(status.clone());
        self
    }

    /// Sets the event handler for the photo.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui_media::url::Url;
    /// use waterui_media::{Photo, photo::Event};
    ///
    /// # fn gallery() -> impl View {
    /// # let url = Url::parse("https://www.rust-lang.org/logos/rust-logo-512x512.png").unwrap();
    /// let photo = Photo::new(url.clone()).on_event(|event: Event| match event {
    ///     Event::Loaded => tracing::debug!("Image loaded!"),
    ///     Event::Error(msg) => tracing::debug!("Error: {msg}"),
    ///     _ => {}
    /// });
    ///
    /// let last_status: Binding<String> = binding(String::new());
    /// let photo = Photo::new(url)
    ///     .on_event(|event: Event, State(last): State<Binding<String>>| {
    ///         last.set(format!("{event:?}"));
    ///     })
    ///     .state(&last_status);
    /// # photo
    /// # }
    /// ```
    #[must_use]
    pub fn on_event<H, A>(mut self, handler: H) -> Self
    where
        H: waterui_core::handler::EventHandler<Event, A, ()> + 'static,
    {
        self.on_event = Some(waterui_core::handler::boxed_event_handler(handler));
        self
    }
}

trait PhotoUiSink: 'static {
    fn show_loading(&self, cached: bool);
    fn hide_overlay(&self);
    fn mark_ready(&self);
    fn mark_failed(&self, reason: &str);
}

type OverlayView<P, F> = OneOf3<(), <P as PlaceholderSlot>::View, <F as FailureSlot>::View>;

struct PhotoSlots<P: PlaceholderSlot, F: FailureSlot> {
    status: Option<Binding<PhotoStatus>>,
    overlay: Option<DynamicHandler<OverlayView<P, F>>>,
    placeholder: RefCell<P>,
    on_failure: RefCell<F>,
}

impl<P: PlaceholderSlot, F: FailureSlot> PhotoUiSink for PhotoSlots<P, F> {
    fn show_loading(&self, cached: bool) {
        if let Some(status) = &self.status {
            status.set(PhotoStatus::Loading);
        }
        if let Some(overlay) = &self.overlay {
            if P::PRESENT && !cached {
                overlay.set(OneOf3::B(self.placeholder.borrow_mut().build()));
            } else {
                overlay.set(OneOf3::A(()));
            }
        }
    }

    fn hide_overlay(&self) {
        if let Some(overlay) = &self.overlay {
            overlay.set(OneOf3::A(()));
        }
    }

    fn mark_ready(&self) {
        self.hide_overlay();
        if let Some(status) = &self.status {
            status.set(PhotoStatus::Ready);
        }
    }

    fn mark_failed(&self, reason: &str) {
        if let Some(status) = &self.status {
            status.set(PhotoStatus::Failed(Str::from(reason.to_owned())));
        }
        if let Some(overlay) = &self.overlay {
            if F::PRESENT {
                overlay.set(OneOf3::C(self.on_failure.borrow_mut().build(reason)));
            } else {
                overlay.set(OneOf3::A(()));
            }
        }
    }
}

impl<P: PlaceholderSlot, F: FailureSlot> View for Photo<P, F> {
    fn body(self, env: &Environment) -> impl View {
        let Self {
            source,
            on_event,
            status,
            placeholder,
            on_failure,
            content,
            content_handler,
        } = self;

        let source = source.resolve(env)?;

        let cache = env.get::<PhotoCache>().cloned();
        let (overlay_handler, overlay_view) = if P::PRESENT || F::PRESENT {
            let (handler, view) = Dynamic::<OneOf3<(), P::View, F::View>>::new();
            handler.set(OneOf3::A(()));
            (Some(handler), Some(view))
        } else {
            (None, None)
        };

        let ui_sink: Rc<dyn PhotoUiSink> = Rc::new(PhotoSlots {
            status,
            overlay: overlay_handler,
            placeholder: RefCell::new(placeholder),
            on_failure: RefCell::new(on_failure),
        });

        let env_for_event = env.clone();
        let on_event = Rc::new(RefCell::new(on_event));
        let generation = Rc::new(Cell::new(0));
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let start = {
            let load_state = Rc::clone(&load_state);
            let cache = cache.clone();
            let ui_sink = Rc::clone(&ui_sink);
            let content_handler = content_handler.clone();
            move |url| {
                start_load_task(
                    url,
                    Rc::clone(&on_event),
                    content_handler.clone(),
                    env_for_event.clone(),
                    generation.clone(),
                    Rc::clone(&load_state),
                    cache.clone(),
                    Rc::clone(&ui_sink),
                );
            }
        };
        let owe = {
            let env = env.clone();
            move |url: &Url| ContentLedger::owe::<Photo>(&env, url.as_str().to_owned())
        };
        let on_source_reset = {
            let content_handler = content_handler;
            let cache = cache;
            let ui_sink = ui_sink;
            move |url: &Url| {
                content_handler.clear();
                let is_cached = cache.as_ref().is_some_and(|c| c.contains(url));
                ui_sink.show_loading(is_cached);
            }
        };
        let guard =
            subscribe_photo_source(&source, &load_state, owe, on_source_reset, start.clone());

        let composed = match overlay_view {
            Some(overlay) => OneOf2::B(zstack((content, overlay))),
            None => OneOf2::A(content),
        };

        Some(Metadata::new(
            Metadata::new(
                composed,
                LifeCycleHook::new(LifeCycle::Appear, {
                    let load_state = Rc::clone(&load_state);
                    move || appear_photo(&load_state, &start)
                }),
            ),
            Retain::new((guard, source, load_state)),
        ))
    }
}

#[derive(Default)]
struct PhotoLoadState {
    appeared: bool,
    latest_source: Option<Url>,
    last_started_source: Option<Url>,
    /// The first frame of `latest_source`, owed to a capture until its
    /// status settles. Replacing it releases the previous source's entry.
    owed: Option<OwedContent>,
    /// The watch turning the current source's image status into its owed
    /// capture and events. Owned here so a source change drops it: a
    /// settlement for a replaced source never fires.
    status: Option<BoxWatcherGuard>,
}

impl PhotoLoadState {
    fn update_source(
        &mut self,
        source: Url,
        owe: impl FnOnce(&Url) -> Option<OwedContent>,
        on_reset: impl FnOnce(&Url),
    ) -> Option<Url> {
        if self.latest_source.as_ref() != Some(&source) {
            self.owed = owe(&source);
            self.status = None;
            on_reset(&source);
        }
        self.latest_source = Some(source.clone());
        self.start_if_ready(source)
    }

    fn appear(&mut self) -> Option<Url> {
        self.appeared = true;
        let source = self
            .latest_source
            .clone()
            .expect("Photo source must be initialized before its Appear lifecycle event");
        self.start_if_ready(source)
    }

    fn start_if_ready(&mut self, source: Url) -> Option<Url> {
        if !self.appeared || self.last_started_source.as_ref() == Some(&source) {
            return None;
        }
        self.last_started_source = Some(source.clone());
        Some(source)
    }
}

fn update_photo_source(
    state: &Rc<RefCell<PhotoLoadState>>,
    source: Url,
    owe: impl FnOnce(&Url) -> Option<OwedContent>,
    on_reset: impl FnOnce(&Url),
    start: &impl Fn(Url),
) {
    let source = state.borrow_mut().update_source(source, owe, on_reset);
    if let Some(source) = source {
        start(source);
    }
}

fn appear_photo(state: &Rc<RefCell<PhotoLoadState>>, start: &impl Fn(Url)) {
    let source = state.borrow_mut().appear();
    if let Some(source) = source {
        start(source);
    }
}

fn subscribe_photo_source<S, O, R, F>(
    source: &S,
    state: &Rc<RefCell<PhotoLoadState>>,
    owe: O,
    on_reset: R,
    start: F,
) -> S::Guard
where
    S: Signal<Output = Url>,
    O: Clone + Fn(&Url) -> Option<OwedContent> + 'static,
    R: Clone + Fn(&Url) + 'static,
    F: Clone + Fn(Url) + 'static,
{
    let guard = source.watch({
        let state = Rc::clone(state);
        let owe = owe.clone();
        let on_reset = on_reset.clone();
        let start = start.clone();
        move |context| update_photo_source(&state, context.into_value(), &owe, &on_reset, &start)
    });
    update_photo_source(state, source.snapshot(), owe, on_reset, &start);
    guard
}

/// Why a photo's source did not decode, as a capture reports it.
#[derive(Debug)]
struct PhotoLoadError(String);

impl core::fmt::Display for PhotoLoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for PhotoLoadError {}

type PhotoEventHandler = Rc<RefCell<Option<waterui_core::handler::BoxedEventAction<Event>>>>;

#[expect(
    clippy::too_many_arguments,
    reason = "internal helper wiring source URL, event/UI sinks, generation guard, and optional cache"
)]
fn start_load_task(
    url: Url,
    on_event: PhotoEventHandler,
    handler: ReactiveImageHandle,
    env: Environment,
    generation: Rc<Cell<usize>>,
    load_state: Rc<RefCell<PhotoLoadState>>,
    cache: Option<PhotoCache>,
    ui_sink: Rc<dyn PhotoUiSink>,
) {
    let (token, settlement) = begin_source_load(
        &handler,
        &on_event,
        &env,
        &generation,
        &load_state,
        &ui_sink,
    );

    if let Some(cached) = cache.as_ref().and_then(|c| c.get(&url)) {
        publish_decoded_frame(&handler, cached);
        decode_finished(
            &settlement,
            &generation,
            token,
            &load_state,
            &on_event,
            &env,
            &ui_sink,
        );
        return;
    }

    spawn_local(async move {
        let frame_generation = generation.clone();
        let handler_for_frames = handler.clone();
        let last_decoded: Rc<RefCell<Option<Image>>> = Rc::new(RefCell::new(None));
        let last_decoded_sink = Rc::clone(&last_decoded);
        match fetch_and_decode_streaming(url.clone(), move |image| {
            if frame_generation.get() == token {
                *last_decoded_sink.borrow_mut() = Some(image.clone());
                publish_decoded_frame(&handler_for_frames, image);
            }
        })
        .await
        {
            Ok(()) => {
                if generation.get() == token
                    && let (Some(cache), Some(final_image)) =
                        (&cache, last_decoded.borrow_mut().take())
                {
                    cache.insert(url, final_image);
                }
                decode_finished(
                    &settlement,
                    &generation,
                    token,
                    &load_state,
                    &on_event,
                    &env,
                    &ui_sink,
                );
            }
            Err(e) => {
                tracing::error!("[Photo] Failed to load: {}", e);
                if generation.get() == token {
                    let mut settlement = settlement.borrow_mut();
                    let fresh = !matches!(settlement.phase, Phase::Reported);
                    settlement.phase = Phase::Reported;
                    drop(settlement);
                    handler.clear();
                    ui_sink.mark_failed(&e);
                    let owed = load_state.borrow_mut().owed.take();
                    if let Some(owed) = owed {
                        owed.fail(PhotoLoadError(e.clone()));
                    }
                    if fresh && let Some(on_event) = on_event.borrow_mut().as_mut() {
                        on_event(Event::Error(e), &env).report_extraction();
                    }
                }
            }
        }
    })
    .detach();
}

/// What the current source's status settlements have come to, shared
/// between the status watch and the decode task that marks `finished`
/// once the stream ends.
#[derive(Debug)]
struct Settlement {
    latest: ImageStatus,
    phase: Phase,
    finished: bool,
}

#[derive(Debug)]
enum Phase {
    AwaitingFirst,
    WatchingFinal,
    Reported,
}

impl Settlement {
    const fn new() -> Self {
        Self {
            latest: ImageStatus::Pending,
            phase: Phase::AwaitingFirst,
            finished: false,
        }
    }
}

fn begin_source_load(
    handler: &ReactiveImageHandle,
    on_event: &PhotoEventHandler,
    env: &Environment,
    generation: &Rc<Cell<usize>>,
    load_state: &Rc<RefCell<PhotoLoadState>>,
    ui_sink: &Rc<dyn PhotoUiSink>,
) -> (usize, Rc<RefCell<Settlement>>) {
    let token = generation
        .get()
        .checked_add(1)
        .expect("Photo load generation overflowed");
    generation.set(token);
    handler.clear();
    let settlement = Rc::new(RefCell::new(Settlement::new()));
    let guard = handler.status().watch({
        let settlement = Rc::clone(&settlement);
        let load_state = Rc::clone(load_state);
        let on_event = Rc::clone(on_event);
        let env = env.clone();
        let generation = Rc::clone(generation);
        let handler = handler.clone();
        let ui_sink = Rc::clone(ui_sink);
        move |context| {
            if generation.get() != token {
                return;
            }
            settlement.borrow_mut().latest = context.into_value();
            settle_registration_inner(
                &settlement,
                &load_state,
                &on_event,
                &env,
                Some(&handler),
                &*ui_sink,
            );
        }
    });
    load_state.borrow_mut().status = Some(guard);
    (token, settlement)
}

fn decode_finished(
    settlement: &Rc<RefCell<Settlement>>,
    generation: &Rc<Cell<usize>>,
    token: usize,
    load_state: &Rc<RefCell<PhotoLoadState>>,
    on_event: &PhotoEventHandler,
    env: &Environment,
    ui_sink: &Rc<dyn PhotoUiSink>,
) {
    if generation.get() != token {
        return;
    }
    settlement.borrow_mut().finished = true;
    settle_registration_inner(settlement, load_state, on_event, env, None, &**ui_sink);
}

fn settle_registration_inner(
    settlement: &RefCell<Settlement>,
    load_state: &RefCell<PhotoLoadState>,
    on_event: &PhotoEventHandler,
    env: &Environment,
    handler: Option<&ReactiveImageHandle>,
    ui_sink: &dyn PhotoUiSink,
) {
    enum Outcome {
        None,
        Ready {
            first: bool,
            finished: bool,
        },
        Refused {
            first: bool,
            error: alloc::sync::Arc<dyn core::error::Error + Send + Sync>,
        },
        Unknown(ImageStatus),
    }

    let outcome = {
        let mut settlement = settlement.borrow_mut();
        if matches!(settlement.phase, Phase::Reported) {
            Outcome::None
        } else {
            match settlement.latest.clone() {
                ImageStatus::Pending => Outcome::None,
                ImageStatus::Ready => {
                    let first = matches!(settlement.phase, Phase::AwaitingFirst);
                    if first {
                        settlement.phase = Phase::WatchingFinal;
                    }
                    let finished = settlement.finished;
                    if finished {
                        settlement.phase = Phase::Reported;
                    }
                    Outcome::Ready { first, finished }
                }
                ImageStatus::Refused(error) => {
                    let first = matches!(settlement.phase, Phase::AwaitingFirst);
                    if first || settlement.finished {
                        settlement.phase = Phase::Reported;
                        Outcome::Refused { first, error }
                    } else {
                        Outcome::None
                    }
                }
                status => Outcome::Unknown(status),
            }
        }
    };

    match outcome {
        Outcome::None => {}
        Outcome::Ready { first, finished } => {
            if first {
                ui_sink.hide_overlay();
                if let Some(owed) = load_state.borrow_mut().owed.take() {
                    owed.fulfil();
                }
            }
            if finished {
                ui_sink.mark_ready();
                if let Some(on_event) = on_event.borrow_mut().as_mut() {
                    on_event(Event::Loaded, env).report_extraction();
                }
            }
        }
        Outcome::Refused { first, error } => {
            let message = error.to_string();
            if let Some(handler) = handler {
                handler.clear();
            }
            ui_sink.mark_failed(&message);
            if first && let Some(owed) = load_state.borrow_mut().owed.take() {
                owed.fail(error);
            }
            if let Some(on_event) = on_event.borrow_mut().as_mut() {
                on_event(Event::Error(message), env).report_extraction();
            }
        }
        Outcome::Unknown(status) => {
            contract_violation!("Photo has no settlement for the image status {status:?}");
        }
    }
}

/// Fetch and decode an image from a URL with streaming/progressive updates.
///
/// The callback is invoked for every published frame, including the final full-quality frame.
#[allow(
    clippy::future_not_send,
    reason = "WaterUI drives Photo loading on the main-thread executor; decoded frames update UI-local DynamicHandler state"
)]
async fn fetch_and_decode_streaming(
    url: Url,
    mut on_decoded_frame: impl FnMut(Image),
) -> Result<(), String> {
    let is_local = url.is_local();
    let url = url.as_str().to_string();
    if is_local {
        let path = url;
        #[cfg(target_arch = "wasm32")]
        let bytes = WaterFs::read(Path::new(&path))
            .await
            .map_err(|error| error.to_string())?;
        #[cfg(not(target_arch = "wasm32"))]
        let bytes = blocking::unblock(move || std::fs::read(&path))
            .await
            .map_err(|error| error.to_string())?;
        let image = Image::from_encoded(&bytes)?;
        on_decoded_frame(image);
        return Ok(());
    }

    #[cfg(feature = "remote")]
    {
        let mut client = zenwave::client();
        let response = client
            .method(Method::GET, &url)
            .map_err(|e| e.to_string())?
            .await
            .map_err(|e| e.to_string())?;

        if !response.status().is_success() {
            return Err(alloc::format!("HTTP error: {}", response.status()));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);

        let mut body = response.into_body().into_data_stream();
        let mut decoder = Image::stream_decoder(content_type.as_deref());

        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| e.to_string())?;
            if let Some(progressive_image) = decoder.push_chunk(&chunk) {
                on_decoded_frame(progressive_image);
            }
        }

        let final_image = decoder.finish()?;
        on_decoded_frame(final_image);
        Ok(())
    }

    #[cfg(not(feature = "remote"))]
    {
        let _ = on_decoded_frame;
        Err(alloc::format!(
            "'{url}' is a remote source; enable the `remote` feature of `waterui-media` (or `waterui/remote`) to fetch it"
        ))
    }
}

fn publish_decoded_frame(handler: &ReactiveImageHandle, image: Image) {
    handler.set(image);
}

/// Convenience constructor for building a `Photo` component inline.
#[must_use]
pub fn photo(source: impl IntoPhotoSource) -> Photo {
    Photo::new(source)
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[cfg(feature = "remote")]
    use super::fetch_and_decode_streaming;
    use super::{
        Event, Photo, PhotoCache, PhotoEventHandler, PhotoLoadState, PhotoStatus, PhotoUiSink,
        appear_photo, begin_source_load, decode_finished, subscribe_photo_source,
        update_photo_source,
    };
    use crate::Url;
    use image::ImageEncoder as _;
    use waterui::graphics::backend::draw::{Content, FontId, ImageId, ImageLimits, LayoutSize};
    use waterui::graphics::backend::{
        FontSource, Handle, ImageData, ResourceError, ResourceHandle, Rgba8, Rgba16F, SceneBackend,
        SceneContent, SceneResources,
    };
    use waterui::graphics::canvas::{Canvas, CanvasView};
    use waterui_capture::ContentLedger;
    use waterui_core::handler::BoxedEventAction;
    use waterui_core::{AnyView, Binding, Environment, Signal, View, reactive::watcher::Context};
    use waterui_image::{DecodePath, Image, ReactiveImage, ReactiveImageHandle, reactive_image};
    use waterui_layout::container::FixedContainer;

    struct NoopUiSink;

    impl PhotoUiSink for NoopUiSink {
        fn show_loading(&self, _cached: bool) {}
        fn hide_overlay(&self) {}
        fn mark_ready(&self) {}
        fn mark_failed(&self, _reason: &str) {}
    }

    fn noop_sink() -> Rc<dyn PhotoUiSink> {
        Rc::new(NoopUiSink)
    }

    #[derive(Clone)]
    struct EmitsDuringSubscription {
        source: Binding<Url>,
        replacement: Url,
    }

    impl Signal for EmitsDuringSubscription {
        type Output = Url;
        type Guard = <Binding<Url> as Signal>::Guard;

        fn snapshot(&self) -> Self::Output {
            self.source.snapshot()
        }

        fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
            self.source.set(self.replacement.clone());
            let watcher = Rc::new(watcher);
            watcher(Context::from(self.replacement.clone()));
            self.source.watch(move |context| watcher(context))
        }
    }

    #[test]
    fn source_subscription_and_appear_start_each_url_once() {
        let replacement = Url::from_file_path_str("/photos/replacement.png");
        let source = Binding::container(Url::from_file_path_str("/photos/initial.png"));
        let signal = EmitsDuringSubscription {
            source: source.clone(),
            replacement: replacement.clone(),
        };
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let started = Rc::new(RefCell::new(Vec::new()));
        let start = {
            let started = Rc::clone(&started);
            move |source| started.borrow_mut().push(source)
        };

        let _guard = subscribe_photo_source(
            &signal,
            &load_state,
            |_: &Url| None,
            |_: &Url| {},
            start.clone(),
        );
        assert!(
            started.borrow().is_empty(),
            "source updates before Appear must not start offscreen loading"
        );

        appear_photo(&load_state, &start);
        source.set(replacement.clone());
        let next = Url::from_file_path_str("/photos/next.png");
        source.set(next.clone());

        assert_eq!(*started.borrow(), vec![replacement, next]);
    }

    #[test]
    fn each_new_source_owes_a_capture_its_first_frame() {
        let mut env = Environment::new();
        let ledger = ContentLedger::new();
        env.insert(ledger.clone());
        let initial = Url::from_file_path_str("/photos/initial.png");
        let source = Binding::container(initial.clone());
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let owe = move |url: &Url| ContentLedger::owe::<Photo>(&env, url.as_str().to_owned());

        let _guard = subscribe_photo_source(&source, &load_state, owe, |_: &Url| {}, |_| {});
        assert_eq!(
            ledger
                .outstanding()
                .iter()
                .map(|entry| (entry.view(), entry.detail().to_string()))
                .collect::<Vec<_>>(),
            vec![("Photo", initial.as_str().to_owned())],
            "the initial source is owed from mount, before Appear"
        );

        source.set(initial);
        assert_eq!(
            ledger.outstanding().len(),
            1,
            "an unchanged source owes nothing new"
        );

        let next = Url::from_file_path_str("/photos/next.png");
        source.set(next.clone());
        assert_eq!(
            ledger
                .outstanding()
                .iter()
                .map(|entry| entry.detail().to_string())
                .collect::<Vec<_>>(),
            vec![next.as_str().to_owned()],
            "a new source releases the previous entry and owes its own"
        );

        load_state
            .borrow_mut()
            .owed
            .take()
            .expect("the current source is owed")
            .fulfil();
        assert_eq!(ledger.pending().snapshot(), 0);
    }

    #[test]
    fn photo_cache_evicts_least_recently_used_entries_and_adjusts_max_bytes() {
        // Each 2x2 RGBA8 image is 16 bytes.
        let cache = PhotoCache::new(32);
        let url_a = Url::from_file_path_str("/photos/a.png");
        let url_b = Url::from_file_path_str("/photos/b.png");
        let url_c = Url::from_file_path_str("/photos/c.png");
        let img = || Image::new(vec![255; 16], 2, 2);

        cache.insert(url_a.clone(), img());
        cache.insert(url_b.clone(), img());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.used_bytes(), 32);

        // Touch `url_a` so `url_b` becomes least-recently used.
        assert!(cache.get(&url_a).is_some());
        cache.insert(url_c.clone(), img());
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.used_bytes(), 32);
        assert!(cache.get(&url_a).is_some(), "url_a was recently accessed");
        assert!(cache.get(&url_b).is_none(), "url_b must be evicted");
        assert!(cache.get(&url_c).is_some(), "url_c was just inserted");

        // Operator lowers max_bytes at runtime: excess entries evict immediately.
        cache.set_max_bytes(16);
        assert_eq!(cache.max_bytes(), 16);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used_bytes(), 16);
        assert!(cache.get(&url_c).is_some());

        cache.set_max_bytes(0);
        assert!(cache.is_empty());
        assert_eq!(cache.used_bytes(), 0);
    }

    #[test]
    fn photo_status_helpers_report_loading_ready_and_failure_reason() {
        assert!(PhotoStatus::default().is_loading());
        assert!(!PhotoStatus::Loading.is_ready());
        assert_eq!(PhotoStatus::Loading.failure(), None);
        assert!(PhotoStatus::Ready.is_ready());
        assert_eq!(PhotoStatus::Ready.failure(), None);
        let failed = PhotoStatus::Failed("bad header".into());
        assert!(!failed.is_loading());
        assert!(!failed.is_ready());
        assert_eq!(failed.failure(), Some("bad header"));
    }

    /// A scene backend that admits every image registration, or refuses
    /// them all, so a published frame's settlement is reproducible without a
    /// GPU.
    struct TestBackend {
        admit: bool,
        attempts: Cell<u64>,
    }

    struct Texture(ImageId);

    impl ResourceHandle for Texture {
        type Id = ImageId;

        fn id(&self) -> ImageId {
            self.0
        }
    }

    impl TestBackend {
        fn admit_image(&self, width: u32, height: u32) -> Result<Handle<ImageId>, ResourceError> {
            self.attempts.set(self.attempts.get() + 1);
            if self.admit {
                Ok(Handle::new(Texture(ImageId::new(self.attempts.get()))))
            } else {
                Err(ResourceError::Image(format!(
                    "{width}x{height} refused by the test backend"
                )))
            }
        }
    }

    impl SceneBackend for TestBackend {
        fn register_font(&self, _source: FontSource) -> Result<Handle<FontId>, ResourceError> {
            Err(ResourceError::Unsupported("fonts"))
        }

        fn register_rgba8(&self, data: ImageData<Rgba8>) -> Result<Handle<ImageId>, ResourceError> {
            self.admit_image(data.width(), data.height())
        }

        fn register_rgba16f(
            &self,
            data: ImageData<Rgba16F>,
        ) -> Result<Handle<ImageId>, ResourceError> {
            self.admit_image(data.width(), data.height())
        }

        fn image_limits(&self) -> ImageLimits {
            ImageLimits::UNLIMITED
        }
    }

    fn mounted_content(image: ReactiveImage) -> Box<dyn SceneContent> {
        let env = Environment::new();
        let body = AnyView::new(image.body(&env));
        let container = body
            .downcast::<FixedContainer<(Canvas,)>>()
            .unwrap_or_else(|_| {
                panic!("a reactive image mounts a canvas inside a fixed container")
            });
        let (_, (canvas,)) = container.into_inner();

        (AnyView::new(canvas.body(&env))
            .downcast::<CanvasView>()
            .unwrap_or_else(|_| panic!("a canvas mounts as a canvas view"))) as _
    }

    fn settle_frame(content: &mut dyn SceneContent, resources: &SceneResources) {
        let mut recording = resources.recording();
        let recorded = Content::record(&LayoutSize::new(), |recorder| {
            content.build_scene(recorder, &mut recording, 8.0, 8.0);
        });
        let _held = recording.finish();
        drop(recorded);
    }

    fn recorded_events() -> (Rc<RefCell<Vec<String>>>, PhotoEventHandler) {
        let events = Rc::new(RefCell::new(Vec::new()));
        let handler: PhotoEventHandler = Rc::new(RefCell::new(Some(Box::new({
            let events = Rc::clone(&events);
            move |event: Event, _: &Environment| {
                events.borrow_mut().push(match event {
                    Event::Loaded => String::from("Loaded"),
                    Event::Error(message) => format!("Error({message})"),
                });
                Ok(())
            }
        })
            as BoxedEventAction<Event>)));
        (events, handler)
    }

    struct LoadTest {
        env: Environment,
        ledger: ContentLedger,
        handle: ReactiveImageHandle,
        content: Box<dyn SceneContent>,
        resources: SceneResources,
        generation: Rc<Cell<usize>>,
        load_state: Rc<RefCell<PhotoLoadState>>,
        events: Rc<RefCell<Vec<String>>>,
        on_event: PhotoEventHandler,
    }

    fn load_test_setup(admit: bool) -> LoadTest {
        waterui_testing::install_test_executor();
        let mut env = Environment::new();
        let ledger = ContentLedger::new();
        env.insert(ledger.clone());
        let (handle, image) = reactive_image();
        let content = mounted_content(image);
        let backend = Rc::new(TestBackend {
            admit,
            attempts: Cell::new(0),
        });
        let resources = SceneResources::new(backend);
        let generation = Rc::new(Cell::new(0));
        let load_state = Rc::new(RefCell::new(PhotoLoadState::default()));
        let (events, on_event) = recorded_events();
        LoadTest {
            env,
            ledger,
            handle,
            content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        }
    }

    #[test]
    fn a_refused_first_frame_fails_the_capture_and_reports_error() {
        let LoadTest {
            env,
            ledger,
            handle,
            mut content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        } = load_test_setup(false);
        let sink = noop_sink();
        load_state.borrow_mut().owed = ContentLedger::owe::<Photo>(&env, "refused.png".to_owned());
        let (token, settlement) =
            begin_source_load(&handle, &on_event, &env, &generation, &load_state, &sink);

        handle.set(Image::new(vec![255; 4], 1, 1));
        settle_frame(&mut *content, &resources);

        let fired = events.borrow();
        let [event] = fired.as_slice() else {
            panic!("a refused first frame reports one error: {fired:?}");
        };
        assert!(
            event.starts_with("Error(") && event.contains("refused by the test backend"),
            "{event}"
        );
        drop(fired);
        let failure = futures_lite::future::block_on(ledger.settled())
            .expect_err("the owed capture fails with the refusal");
        assert!(
            failure
                .get_source()
                .to_string()
                .contains("refused by the test backend"),
            "{failure}"
        );

        decode_finished(
            &settlement,
            &generation,
            token,
            &load_state,
            &on_event,
            &env,
            &sink,
        );
        assert_eq!(
            events.borrow().len(),
            1,
            "a refused source never reports Loaded"
        );
    }

    #[test]
    fn a_registered_final_frame_reports_loaded_once() {
        let LoadTest {
            env,
            ledger,
            handle,
            mut content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        } = load_test_setup(true);
        let sink = noop_sink();
        load_state.borrow_mut().owed =
            ContentLedger::owe::<Photo>(&env, "registered.png".to_owned());
        let (token, settlement) =
            begin_source_load(&handle, &on_event, &env, &generation, &load_state, &sink);

        handle.set(Image::new(vec![255; 4], 1, 1));
        settle_frame(&mut *content, &resources);
        assert_eq!(
            ledger.pending().snapshot(),
            0,
            "the first registered frame fulfils the owed capture"
        );
        assert!(
            events.borrow().is_empty(),
            "Loaded waits for the final frame"
        );

        decode_finished(
            &settlement,
            &generation,
            token,
            &load_state,
            &on_event,
            &env,
            &sink,
        );
        assert_eq!(*events.borrow(), vec![String::from("Loaded")]);

        settle_frame(&mut *content, &resources);
        assert_eq!(events.borrow().len(), 1);
    }

    #[test]
    fn a_settlement_for_a_replaced_source_reports_nothing() {
        let LoadTest {
            env,
            ledger,
            handle,
            mut content,
            resources,
            generation,
            load_state,
            events,
            on_event,
        } = load_test_setup(false);
        let sink = noop_sink();

        load_state.borrow_mut().appeared = true;
        load_state.borrow_mut().owed = ContentLedger::owe::<Photo>(&env, "replaced.png".to_owned());
        let _ = begin_source_load(&handle, &on_event, &env, &generation, &load_state, &sink);
        handle.set(Image::new(vec![255; 4], 1, 1));

        let next = Url::from_file_path_str("/photos/next.png");
        update_photo_source(
            &load_state,
            next.clone(),
            |url| ContentLedger::owe::<Photo>(&env, url.as_str().to_owned()),
            |_: &Url| {},
            &{
                let handle = handle.clone();
                let on_event = Rc::clone(&on_event);
                let env = env.clone();
                let generation = Rc::clone(&generation);
                let load_state = Rc::clone(&load_state);
                let sink = Rc::clone(&sink);
                move |_| {
                    let _ = begin_source_load(
                        &handle,
                        &on_event,
                        &env,
                        &generation,
                        &load_state,
                        &sink,
                    );
                }
            },
        );

        settle_frame(&mut *content, &resources);
        assert!(
            events.borrow().is_empty(),
            "a frame of a replaced source reports nothing"
        );
        assert_eq!(
            ledger
                .outstanding()
                .iter()
                .map(|entry| entry.detail().to_string())
                .collect::<Vec<_>>(),
            vec![next.as_str().to_owned()],
            "the previous source's entry was released"
        );

        handle.set(Image::new(vec![255; 4], 1, 1));
        settle_frame(&mut *content, &resources);
        assert_eq!(events.borrow().len(), 1);
    }

    fn sample_png_bytes() -> Vec<u8> {
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &[
                    255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                ],
                2,
                2,
                image::ExtendedColorType::Rgba8,
            )
            .expect("sample PNG should encode");
        png
    }

    #[test]
    fn png_decode_path_smoke() {
        let bytes = sample_png_bytes();
        let (image, path) =
            Image::from_encoded_with_path(&bytes).expect("png should decode from software path");
        assert_eq!(path, DecodePath::SoftwareFallback);
        assert!(image.width() > 0 && image.height() > 0);
    }

    #[cfg(feature = "remote")]
    #[test]
    #[ignore = "requires network access to real image URLs"]
    #[allow(
        clippy::vec_init_then_push,
        reason = "platform-gated pushes make a `vec![]` initializer impractical (it would leave `mut` unused where neither gated case applies)"
    )]
    fn streaming_decode_real_images_smoke() {
        futures_lite::future::block_on(async {
            let mut cases = Vec::new();
            cases.push((
                "jpeg",
                "https://raw.githubusercontent.com/libjpeg-turbo/libjpeg-turbo/main/testimages/testorig.jpg",
            ));
            cases.push((
                "tiff",
                "https://raw.githubusercontent.com/python-pillow/Pillow/main/Tests/images/hopper.tif",
            ));
            #[cfg(not(target_vendor = "apple"))]
            {
                cases.push((
                    "avif",
                    "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.8bpc.yuv420.avif",
                ));
            }
            #[cfg(target_os = "android")]
            {
                cases.push((
                    "heic",
                    "https://raw.githubusercontent.com/strukturag/libheif/master/examples/example.heic",
                ));
            }

            for (name, raw_url) in cases {
                let url: Url = raw_url.parse().expect("url should parse");
                let mut frames = 0usize;
                fetch_and_decode_streaming(url, |_| {
                    frames += 1;
                })
                .await
                .unwrap_or_else(|error| panic!("{name} streaming decode failed: {error}"));
                assert!(frames >= 1, "{name} should publish at least one frame");
            }
        });
    }

    #[cfg(all(
        feature = "remote",
        any(target_vendor = "apple", target_os = "android")
    ))]
    #[test]
    #[ignore = "requires network access and platform HDR AVIF decode support"]
    fn hdr_avif_decode_real_image_smoke() {
        futures_lite::future::block_on(async {
            use zenwave::{Client, Method};

            let candidates = [
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.10bpc.yuv420.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/hato.profile0.10bpc.yuv420.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile1.10bpc.yuv444.avif",
                "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile2.10bpc.yuv422.avif",
            ];

            let mut selected = None;
            for url in candidates {
                let mut client = zenwave::client();
                let response = client
                    .method(Method::GET, url)
                    .expect("HDR AVIF sample request should build")
                    .await
                    .expect("HDR AVIF sample request should succeed");
                assert!(
                    response.status().is_success(),
                    "status={}",
                    response.status()
                );
                let bytes = response
                    .into_body()
                    .into_bytes()
                    .await
                    .expect("body should load");
                let Ok(decoded) = waterkit_codec::decode_image(&bytes) else {
                    continue;
                };
                if decoded.pixel_format() != waterkit_codec::DecodedPixelFormat::Rgba16Float
                    || !decoded.hdr()
                {
                    continue;
                }
                selected = Some(decoded);
                break;
            }

            let decoded = selected.expect("no HDR AVIF sample decoded to RGBA16F HDR data");
            assert_eq!(
                decoded.pixel_format(),
                waterkit_codec::DecodedPixelFormat::Rgba16Float,
                "10-bit AVIF should decode to RGBA16F on HDR-capable platform decoder path"
            );
            assert!(
                decoded.hdr(),
                "10-bit AVIF should be marked as HDR from platform decoder"
            );
        });
    }

    #[cfg(all(feature = "remote", target_vendor = "apple"))]
    #[test]
    #[ignore = "requires network access and Apple HEIC platform decode support"]
    fn heic_h265_decode_real_image_smoke() {
        futures_lite::future::block_on(async {
            use zenwave::{Client, Method};

            let mut client = zenwave::client();
            let response = client
                .method(
                    Method::GET,
                    "https://raw.githubusercontent.com/strukturag/libheif/master/examples/example.heic",
                )
                .expect("HEIC sample request should build")
                .await
                .expect("HEIC sample request should succeed");
            assert!(
                response.status().is_success(),
                "status={}",
                response.status()
            );
            let bytes = response
                .into_body()
                .into_bytes()
                .await
                .expect("body should load");

            let image =
                Image::from_encoded(&bytes).expect("HEIC/H265 decode should succeed on Apple");
            assert!(image.width() > 0 && image.height() > 0);
        });
    }

    #[cfg(feature = "remote")]
    #[test]
    #[ignore = "requires network access and AV1/AVIF decode support"]
    fn av1_decode_fallback_probe_smoke() {
        futures_lite::future::block_on(async {
            use zenwave::{Client, Method};

            let mut client = zenwave::client();
            let response = client
                .method(
                    Method::GET,
                    "https://raw.githubusercontent.com/link-u/avif-sample-images/master/fox.profile0.8bpc.yuv420.avif",
                )
                .expect("AVIF sample request should build")
                .await
                .expect("AV1 sample request should succeed");
            assert!(
                response.status().is_success(),
                "status={}",
                response.status()
            );
            let bytes = response
                .into_body()
                .into_bytes()
                .await
                .expect("body should load");

            let platform_probe = waterkit_codec::decode_image_platform(&bytes);
            let platform_ok = platform_probe.is_ok();

            let final_image = Image::from_encoded(&bytes)
                .expect("AV1 should decode via platform or software path");
            let (decoded_with_path, path) =
                Image::from_encoded_with_path(&bytes).expect("AV1 should decode via selected path");
            let decoded_source =
                waterkit_codec::decode_image(&bytes).expect("AV1 sample should decode to pixels");
            assert!(
                !decoded_source.hdr(),
                "8bpc AVIF sample must stay SDR (hdr=false)"
            );
            assert_eq!(final_image.width(), decoded_with_path.width());
            assert_eq!(final_image.height(), decoded_with_path.height());
            if platform_ok {
                assert_eq!(
                    path,
                    DecodePath::Platform,
                    "decode path should stay platform when platform probe succeeds"
                );
            } else {
                assert_eq!(
                    path,
                    DecodePath::SoftwareFallback,
                    "decode path should fall back to software when platform probe fails"
                );
                let software_probe = waterkit_codec::decode_image(&bytes)
                    .expect("AV1 software fallback should decode on platform AV1 decode miss");
                assert!(software_probe.width() > 0 && software_probe.height() > 0);
            }
        });
    }
}
