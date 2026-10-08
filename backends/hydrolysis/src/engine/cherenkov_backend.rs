//! The concrete Cherenkov GPU runtime (water-rs/hydrolysis#205, H1).
//!
//! One [`cherenkov::Engine<cherenkov_gpu::Gpu>`] per shared GPU context: every
//! surface a provider hands out carries a [`gpu_context_id`](crate::platform::SurfaceProvider)
//! naming the device creation chain its handles came from, and the pool here
//! binds one engine to each. Surfaces a window presents through are
//! [`CherenkovSurface`]s — an engine `TextureTarget` whose premultiplied
//! linear Display P3 texture the presenter converts into the host-acquired
//! frame. Acquisition stays host-owned: the provider acquires, the presenter
//! blits, the provider presents exactly once per presented frame.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::mpsc;

use rustc_hash::FxHashMap;

/// Awaits `expr` on wasm32, where the Cherenkov engine lifecycle is async;
/// evaluates it directly on native, where the same calls are synchronous —
/// one code path, two compile modes (water-rs/hydrolysis#205).
#[cfg(target_arch = "wasm32")]
macro_rules! engine_await {
    ($expr:expr) => {
        $expr.await
    };
}

#[cfg(not(target_arch = "wasm32"))]
macro_rules! engine_await {
    ($expr:expr) => {
        $expr
    };
}

// `macro_rules!` items have no visibility beyond the crate without
// `#[macro_export]` — `pub(crate)` is the maximum legal re-export level.
pub(crate) use engine_await;

/// Declares one body `async fn` on wasm32 and `fn` on native: the async
/// engine lifecycle the wasm build awaits forces the keyword onto every frame
/// between the caller and an engine call; native keeps the synchronous
/// signature so no second code path exists there.
///
/// The wasm futures this macro emits are intentionally `!Send`: wasm32 is
/// single-threaded, the engine's `Rc`/`RefCell` handles and thread-local
/// pools never cross a thread, and `future_not_send` exists to catch
/// `Send`-promising public futures — which none of these are. The allow
/// lives on the macro arm rather than per function so a future added to
/// this list inherits the same justification.
///
/// Two forms: `fn name(args) -> ret { body }` shares the signature across
/// targets; `fn name {native-args} {wasm-args} -> ret { body }` diverges the
/// parameter lists where a callback becomes an `AsyncFn` on wasm. A third
/// form, `impl Ty { fn ... }`, does the same for a method inside its `impl`.
macro_rules! cfg_async_fn {
    (impl $ty:ty {
        $(#[$meta:meta])*
        $vis:vis fn $name:ident $(<$($gen:ident $(: $bound:ident)?),* $(,)?>)? ($($args:tt)*) $(-> $ret:ty)? $body:block
    }) => {
        impl $ty {
            #[cfg(not(target_arch = "wasm32"))]
            $(#[$meta])* $vis fn $name $(<$($gen $(: $bound)?),*>)? ($($args)*) $(-> $ret)? $body
            #[cfg(target_arch = "wasm32")]
            #[allow(clippy::future_not_send, reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread")]
            $(#[$meta])* $vis async fn $name $(<$($gen $(: $bound)?),*>)? ($($args)*) $(-> $ret)? $body
        }
    };
    ($(#[$meta:meta])*
     $vis:vis fn $name:ident $(<$($gen:ident $(: $bound:ident)?),* $(,)?>)? { $($native_args:tt)* } { $($wasm_args:tt)* }
     $(-> $ret:ty)? $body:block) => {
        #[cfg(not(target_arch = "wasm32"))]
        $(#[$meta])* $vis fn $name $(<$($gen $(: $bound)?),*>)? ($($native_args)*) $(-> $ret)? $body
        #[cfg(target_arch = "wasm32")]
        #[allow(clippy::future_not_send, reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread")]
        $(#[$meta])* $vis async fn $name $(<$($gen $(: $bound)?),*>)? ($($wasm_args)*) $(-> $ret)? $body
    };
    ($(#[$meta:meta])*
     $vis:vis fn $name:ident $(<$($gen:ident $(: $bound:ident)?),* $(,)?>)? ($($args:tt)*) $(-> $ret:ty)? $body:block) => {
        #[cfg(not(target_arch = "wasm32"))]
        /// Generates the function twice: synchronously for native targets and `async` for wasm32.
        $(#[$meta])* $vis fn $name $(<$($gen $(: $bound)?),*>)? ($($args)*) $(-> $ret)? $body
        #[cfg(target_arch = "wasm32")]
        #[allow(clippy::future_not_send, reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread")]
        $(#[$meta])* $vis async fn $name $(<$($gen $(: $bound)?),*>)? ($($args)*) $(-> $ret)? $body
    };
}

pub(crate) use cfg_async_fn;

/// The output colour of a target whose colour space was not negotiated
/// (`SurfaceColorSpace::Auto`, or an offscreen texture): a float format
/// receives extended linear Display P3, every other format sRGB. A surface
/// that negotiates its (format, colour space) pair presents the colour of
/// that pair instead.
#[must_use]
pub fn format_output_color(format: wgpu::TextureFormat) -> cherenkov_gpu::interop::OutputColor {
    if matches!(
        format.remove_srgb_suffix(),
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    ) {
        cherenkov_gpu::interop::OutputColor::LinearDisplayP3
    } else {
        cherenkov_gpu::interop::OutputColor::Srgb
    }
}

/// The production backend: the concrete wgpu engine, never a second
/// rendering stack or a runtime-selected one.
pub type GpuEngine = cherenkov::Engine<cherenkov_gpu::Gpu>;

/// The private engine state pooled per GPU context: the `Rc<GpuEngine>` and
/// the ONE `Rc<SceneResources>` registration table every window and capture
/// mount on this context's device shares. Retained `SceneContent` associates
/// its engine-bound state with this table's identity, so the pair is created
/// together and expires together — a replacement context makes a fresh pair.
pub struct SharedEngineState {
    pub engine: Rc<GpuEngine>,
    pub resources: Rc<crate::renderer::recording::SceneResources>,
}

thread_local! {
    /// Engines alive on this thread, keyed by GPU-context identity. Weak so a
    /// context whose surfaces have all dropped releases its engine instead of
    /// pinning it for the thread's lifetime.
    static ENGINES: RefCell<FxHashMap<u64, Weak<SharedEngineState>>> = RefCell::new(FxHashMap::default());
}

cfg_async_fn! {
    /// The shared engine state for `context_id`'s GPU context, created on
    /// first use: one engine and one resource table, alive while any window
    /// or capture mount on this context holds it. Each window's surface on
    /// it carries that window's own host wake ([`CherenkovSurface::new`]).
    ///
    /// Async on wasm32, where `Engine::new` awaits the browser's GPU device.
    pub fn shared_engine_state(
        context_id: u64,
        adapter: &wgpu::Adapter,
        shared_device: cherenkov_gpu::interop::SharedDevice,
    ) -> Rc<SharedEngineState> {
        let pooled =
            ENGINES.with(|pool| pool.borrow().get(&context_id).and_then(Weak::upgrade));
        if let Some(state) = pooled {
            return state;
        }
        let config = cherenkov_gpu::GpuConfig {
            device: Some(shared_device),
            pipeline_cache: pipeline_cache_path(adapter),
            ..cherenkov_gpu::GpuConfig::default()
        };
        let engine = Rc::new(
            engine_await!(GpuEngine::new(config))
                .expect("hydrolysis renderer: failed to create the Cherenkov engine"),
        );
        let state = Rc::new(SharedEngineState {
            engine: Rc::clone(&engine),
            resources: Rc::new(crate::renderer::recording::SceneResources::new(&engine)),
        });
        ENGINES.with(|pool| pool.borrow_mut().insert(context_id, Rc::downgrade(&state)));
        state
    }
}

/// The persistent pipeline-cache path for `adapter`, where the platform has a
/// writable cache directory; the engine loads, feeds and persists it itself.
#[cfg(hydrolysis_pipeline_cache)]
fn pipeline_cache_path(adapter: &wgpu::Adapter) -> Option<std::path::PathBuf> {
    crate::pipeline_cache::path(adapter)
}

/// No persistent pipeline cache exists on targets without a platform cache
/// directory; the engine accepts `None` the same way.
#[cfg(not(hydrolysis_pipeline_cache))]
const fn pipeline_cache_path(_adapter: &wgpu::Adapter) -> Option<std::path::PathBuf> {
    None
}

/// A window's output surface — one of the two presentation kinds, fixed
/// when the surface is created:
///
/// - [`Self::Texture`]: the engine renders into an engine-owned
///   `Rgba16Float` linear-P3 texture whose notification channel fires on
///   creation and on every resize; the host acquires, presents into and
///   presents its own surface frame.
/// - [`Self::Window`]: the engine presents through
///   `cherenkov_gpu::WindowTarget` (the macOS winit window, #2223). No
///   texture channel exists and [`TextureCherenkovSurface::present_into`]
///   is unreachable in types: the window kind has no `present_into`,
///   acquire or present calls.
pub enum CherenkovSurface {
    /// Texture-backed presentation: the host presents a copy of the
    /// rendered texture.
    Texture(Box<TextureCherenkovSurface>),
    /// Window-backed presentation: `Engine::render` presents the frame
    /// through the window's own layers.
    #[cfg(all(target_os = "macos", hydrolysis_winit))]
    Window(WindowCherenkovSurface),
}

/// The fields both [`CherenkovSurface`] kinds share: engine, surface and
/// the size whose change queues a resize.
struct SurfaceCore {
    engine: Rc<GpuEngine>,
    surface: Rc<cherenkov::Surface<cherenkov_gpu::Gpu>>,
    size: (u32, u32),
}

/// Re-samples `probe`'s headroom into `last`: the probe is kept, never
/// consumed, a `None` report keeps `last`, and `last` is the result either
/// way. Generic so a test can drive the state machine with a fake probe —
/// a real [`DisplayProbe`] holds a live `wgpu::Surface`.
#[cfg(all(target_os = "macos", hydrolysis_winit))]
fn sample_headroom<P>(
    slot: &std::cell::Cell<Option<P>>,
    last: &std::cell::Cell<f32>,
    headroom_of: impl Fn(&P) -> Option<f32>,
) -> f32 {
    if let Some(headroom) = slot.take().and_then(|probe| {
        let headroom = headroom_of(&probe);
        slot.set(Some(probe));
        headroom
    }) {
        last.set(headroom);
    }
    last.get()
}

/// The texture-backed [`CherenkovSurface`]: engine output in a texture the
/// host presents a copy of.
pub struct TextureCherenkovSurface {
    core: SurfaceCore,
    /// The offscreen texture notifications — fires on creation and on
    /// every resize; [`Self::render`] drains it before the presenter
    /// samples — sampling post-resize without draining is the
    /// stale-attachment case the plan bans.
    textures: mpsc::Receiver<wgpu::Texture>,
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    presenter: cherenkov_gpu::interop::Presenter,
}

/// The window-backed [`CherenkovSurface`]: the engine presents through
/// `WindowTarget`, so the host's only live display input is the output
/// probe it keeps and re-samples every frame.
#[cfg(all(target_os = "macos", hydrolysis_winit))]
pub struct WindowCherenkovSurface {
    core: SurfaceCore,
    /// The channel `WindowTarget` pushes a `DisplayProbe` through per
    /// output configuration; drained on main by [`Self::headroom`].
    probe_rx: mpsc::Receiver<cherenkov_gpu::interop::DisplayProbe>,
    /// The latest probe, kept and re-sampled every frame — headroom
    /// follows brightness and display changes between pushes.
    probe: std::cell::Cell<Option<cherenkov_gpu::interop::DisplayProbe>>,
    /// The last headroom the probe reported; the engine's neutral
    /// `Display::default().headroom` until one has. A `None` from
    /// `tone_map_headroom` keeps it — SDR is never guessed.
    last_headroom: std::cell::Cell<f32>,
}

impl core::fmt::Debug for CherenkovSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CherenkovSurface")
            .field("size", &self.size())
            .finish_non_exhaustive()
    }
}

impl CherenkovSurface {
    /// Creates the texture-backed surface at `size` (physical pixels) and
    /// takes the presenter for `device`'s shader delivery. `wake` is the
    /// host's display-link wake: the surface calls it, from whichever
    /// thread the cause lands on, when its content — a GPU producer, a
    /// filter, a submitted frame, a live operand — asks for a frame
    /// between the renderer's own ([`TextureCherenkovSurface::begin_frame`],
    /// forwarded here).
    ///
    /// Async on wasm32, where `Engine::surface` awaits the browser device.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::Texture(Box::new(TextureCherenkovSurface::new(
            engine, device, backend, size, wake,
        )))
    }

    /// [`Self::new`], async on wasm32 where `Engine::surface` awaits the
    /// browser device.
    #[cfg(target_arch = "wasm32")]
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    pub async fn new(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::Texture(Box::new(
            TextureCherenkovSurface::new(engine, device, backend, size, wake).await,
        ))
    }

    /// The macOS winit window's surface (#2223): the engine presents
    /// through `target`'s `WindowTarget`, so no texture channel exists and
    /// `present_into` cannot exist on this kind. The probe channel the
    /// target registered is kept for [`WindowCherenkovSurface::headroom`].
    #[cfg(all(target_os = "macos", hydrolysis_winit))]
    pub fn new_window(
        engine: Rc<GpuEngine>,
        target: cherenkov_gpu::WindowTarget,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::Window(WindowCherenkovSurface::new(engine, target, wake))
    }

    /// The engine surface behind this output target — mount, edit and
    /// transaction calls route through it.
    pub fn engine_surface(&self) -> &cherenkov::Surface<cherenkov_gpu::Gpu> {
        &self.core().surface
    }

    /// The engine surface as the window's retained mount holds it: weak, so
    /// the mount never extends the surface past its window.
    pub fn engine_surface_weak(&self) -> std::rc::Weak<cherenkov::Surface<cherenkov_gpu::Gpu>> {
        Rc::downgrade(&self.core().surface)
    }

    /// Opens the renderer's frame: while the scope is held, the edits it
    /// makes to the surface before [`Self::render`] wake no host, because
    /// that render draws them.
    #[must_use = "dropping the scope ends the frame; keep it until the frame's render"]
    pub fn begin_frame(&self) -> cherenkov::FrameScope {
        self.core().surface.begin_frame()
    }

    /// Queues a resize when `size` changed.
    pub fn resize(&mut self, size: (u32, u32)) {
        let core = self.core_mut();
        if core.size == size {
            return;
        }
        core.size = size;
        core.surface
            .resize(size)
            .expect("hydrolysis renderer: engine surface resize failed");
    }

    /// The pixel size this surface last reported.
    const fn size(&self) -> (u32, u32) {
        self.core().size
    }

    /// The colour the surface clears to before content.
    pub fn clear_color(&self, color: waterui_graphics::draw::WorkingColor) {
        self.core().surface.clear_color(color);
    }

    /// The texture-backed kind of this surface.
    // Always `Some` off Apple, where `Window` is cfg'd out.
    #[allow(clippy::unnecessary_wraps)]
    pub const fn as_texture(&mut self) -> Option<&mut TextureCherenkovSurface> {
        match self {
            Self::Texture(surface) => Some(&mut **surface),
            #[cfg(all(target_os = "macos", hydrolysis_winit))]
            Self::Window(_) => None,
        }
    }

    const fn core(&self) -> &SurfaceCore {
        match self {
            Self::Texture(surface) => &surface.core,
            #[cfg(all(target_os = "macos", hydrolysis_winit))]
            Self::Window(surface) => &surface.core,
        }
    }

    const fn core_mut(&mut self) -> &mut SurfaceCore {
        match self {
            Self::Texture(surface) => &mut surface.core,
            #[cfg(all(target_os = "macos", hydrolysis_winit))]
            Self::Window(surface) => &mut surface.core,
        }
    }
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
impl WindowCherenkovSurface {
    fn new(
        engine: Rc<GpuEngine>,
        mut target: cherenkov_gpu::WindowTarget,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let size = target.size();
        let probe_rx = target.output_probe();
        let surface = engine
            .surface(target, wake)
            .expect("hydrolysis renderer: failed to create the Cherenkov window surface");
        Self {
            core: SurfaceCore {
                engine,
                surface: Rc::new(surface),
                size,
            },
            probe_rx,
            probe: std::cell::Cell::new(None),
            last_headroom: std::cell::Cell::new(cherenkov::Display::default().headroom),
        }
    }

    /// The display's headroom, re-sampled every frame on main: the channel
    /// delivers the newest `DisplayProbe`, the kept probe is asked for its
    /// `tone_map_headroom`, and a `None` — no report for the current
    /// output configuration — keeps the last known value rather than
    /// guessing SDR.
    pub fn headroom(&self) -> f32 {
        while let Ok(latest) = self.probe_rx.try_recv() {
            self.probe.set(Some(latest));
        }
        sample_headroom(
            &self.probe,
            &self.last_headroom,
            cherenkov_gpu::interop::DisplayProbe::tone_map_headroom,
        )
    }

    /// The display's properties: `scale` is the logical-to-physical factor
    /// applied exactly once at the surface root; the headroom is the
    /// retained probe's latest sample, kept between samples.
    pub fn display(&self, scale: f64) {
        self.core
            .surface
            .display(cherenkov::Display {
                scale,
                headroom: self.headroom(),
            })
            .expect("hydrolysis renderer: engine surface display update failed");
    }

    /// Announces a display move the platform observed
    /// (`NSWindowDidChangeScreenNotification`): the engine re-runs output
    /// negotiation and pushes a fresh probe.
    pub fn display_moved(&self) {
        self.core
            .surface
            .display_moved()
            .expect("hydrolysis renderer: engine surface display move failed");
    }

    /// Whether the frame's presentation is still pending — the drawable
    /// could not be acquired or a queued operation is presenting it — so
    /// the frame did not reach the display.
    pub fn presentation_pending(&self) -> bool {
        self.core.surface.presentation_pending()
    }
}

impl TextureCherenkovSurface {
    /// Creates the texture-backed surface (see [`CherenkovSurface::new`]).
    #[cfg(not(target_arch = "wasm32"))]
    fn new(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (target, textures) = cherenkov_gpu::interop::TextureTarget::new(size);
        let surface = engine
            .surface(target, wake)
            .expect("hydrolysis renderer: failed to create the Cherenkov surface");
        Self::build(engine, device, backend, size, surface, textures)
    }

    /// [`Self::new`], async on wasm32 where `Engine::surface` awaits the
    /// browser device.
    #[cfg(target_arch = "wasm32")]
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    async fn new(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (target, textures) = cherenkov_gpu::interop::TextureTarget::new(size);
        let surface = engine
            .surface(target, wake)
            .await
            .expect("hydrolysis renderer: failed to create the Cherenkov surface");
        Self::build(engine, device, backend, size, surface, textures)
    }

    /// The construction the sync native and async wasm32 [`Self::new`]
    /// variants share past `Engine::surface`.
    fn build(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
        surface: cherenkov::Surface<cherenkov_gpu::Gpu>,
        textures: mpsc::Receiver<wgpu::Texture>,
    ) -> Self {
        let delivery = cherenkov_gpu::interop::shader_delivery(backend, device)
            .expect("hydrolysis renderer: shader delivery unsupported on this device");
        Self {
            core: SurfaceCore {
                engine,
                surface: Rc::new(surface),
                size,
            },
            textures,
            texture: None,
            presenter: cherenkov_gpu::interop::Presenter::new(device, delivery),
        }
    }

    /// The display's properties: `scale` is the logical-to-physical factor
    /// applied exactly once at the surface root, `headroom` the HDR
    /// headroom the display reaches.
    pub fn display(&self, scale: f64, headroom: f32) {
        self.core
            .surface
            .display(cherenkov::Display { scale, headroom })
            .expect("hydrolysis renderer: engine surface display update failed");
    }

    /// Renders the committed change set and drains texture notifications,
    /// returning the engine's `Next` for the pump to schedule against.
    ///
    /// The rendered texture stays held in [`Self::texture`]; the presenter
    /// samples it through [`Self::present_into`].
    ///
    /// Async on wasm32, where `Engine::render` awaits the browser device.
    ///
    /// # Errors
    ///
    /// Returns the engine's [`cherenkov::RenderError`] when the frame fails
    /// to render.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(&mut self) -> Result<cherenkov::Next, cherenkov::RenderError> {
        let next = self.core.engine.render(cherenkov::FrameTime::now())?;
        Ok(self.render_inner(next))
    }

    /// [`Self::render`], async on wasm32 where `Engine::render` awaits the
    /// browser device.
    #[cfg(target_arch = "wasm32")]
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    pub async fn render(&mut self) -> Result<cherenkov::Next, cherenkov::RenderError> {
        let next = self.core.engine.render(cherenkov::FrameTime::now()).await?;
        Ok(self.render_inner(next))
    }

    /// The texture-notification drain and `Next` plumbing the two
    /// [`Self::render`] variants share past `Engine::render`.
    fn render_inner(&mut self, next: cherenkov::Next) -> cherenkov::Next {
        while let Ok(texture) = self.textures.try_recv() {
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.texture = Some((texture, view));
        }
        self.texture
            .as_ref()
            .expect("hydrolysis renderer: engine produced no surface texture");
        next
    }

    /// Converts the rendered engine texture into `output` through the
    /// presenter — the one presentation path the plan names. The host still
    /// owns acquire and `present`; this writes into the acquired texture
    /// exactly once.
    pub fn present_into(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        output: &wgpu::Texture,
        color: cherenkov_gpu::interop::OutputColor,
        alpha: cherenkov_gpu::interop::OutputAlpha,
        headroom: f32,
    ) {
        let (_, view) = self
            .texture
            .as_ref()
            .expect("hydrolysis renderer: present before the engine produced a texture");
        self.presenter.texture(
            device,
            queue,
            view,
            cherenkov_gpu::interop::TextureOutput {
                texture: output,
                color,
                alpha,
                headroom,
            },
        );
    }
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
impl WindowCherenkovSurface {
    /// Renders the committed change set; the engine presents the frame
    /// itself. Returns the engine's `Next` for the pump to schedule
    /// against.
    ///
    /// # Errors
    ///
    /// Returns the engine's [`cherenkov::RenderError`] when the frame fails
    /// to render.
    pub fn render(&self) -> Result<cherenkov::Next, cherenkov::RenderError> {
        self.core.engine.render(cherenkov::FrameTime::now())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl CherenkovSurface {
    /// Renders the committed change set — on a texture-backed surface this
    /// also drains the texture notifications (see
    /// [`TextureCherenkovSurface::render`]); on a window-backed surface the
    /// engine presents the frame itself.
    ///
    /// # Errors
    /// Returns the engine's [`cherenkov::RenderError`] when the frame fails
    /// to render.
    pub fn render(&mut self) -> Result<cherenkov::Next, cherenkov::RenderError> {
        match self {
            Self::Texture(surface) => surface.render(),
            #[cfg(all(target_os = "macos", hydrolysis_winit))]
            Self::Window(surface) => surface.render(),
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl CherenkovSurface {
    /// [`CherenkovSurface::render`], async on wasm32 where `Engine::render`
    /// awaits the browser device.
    ///
    /// # Errors
    /// Returns the engine's [`cherenkov::RenderError`] when the frame fails
    /// to render.
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    pub async fn render(&mut self) -> Result<cherenkov::Next, cherenkov::RenderError> {
        match self {
            Self::Texture(surface) => surface.render().await,
        }
    }
}

#[cfg(all(test, target_os = "macos", hydrolysis_winit))]
mod tests {
    use std::cell::Cell;

    use super::sample_headroom;

    /// A stand-in `DisplayProbe`: a real probe holds a live `wgpu::Surface`
    /// and needs a window, so the test drives the sampling state machine
    /// with a report it controls and a read counter it can assert on.
    struct FakeProbe {
        report: Cell<Option<f32>>,
        reads: Cell<u32>,
    }

    impl FakeProbe {
        fn headroom(&self) -> Option<f32> {
            self.reads.set(self.reads.get() + 1);
            self.report.get()
        }
    }

    #[test]
    fn the_probe_is_retained_and_resampled() {
        let probe = Cell::new(Some(FakeProbe {
            report: Cell::new(Some(2.0)),
            reads: Cell::new(0),
        }));
        let last = Cell::new(1.0);

        // Every call re-samples the kept probe — the first report lands.
        assert_eq!(sample_headroom(&probe, &last, FakeProbe::headroom), 2.0);
        let kept = probe.take().expect("the probe is kept, not consumed");
        assert_eq!(kept.reads.get(), 1);
        // Let the display report a new headroom: the next sample follows it
        // without a new probe arriving.
        kept.report.set(Some(4.0));
        probe.set(Some(kept));
        assert_eq!(sample_headroom(&probe, &last, FakeProbe::headroom), 4.0);
        assert_eq!(probe.take().expect("still kept").reads.get(), 2);
    }

    #[test]
    fn a_none_report_keeps_the_last_headroom() {
        let probe = Cell::new(Some(FakeProbe {
            report: Cell::new(None),
            reads: Cell::new(0),
        }));
        let last = Cell::new(3.0);
        assert_eq!(sample_headroom(&probe, &last, FakeProbe::headroom), 3.0);
        assert_eq!(last.get(), 3.0);
        // And an empty slot — before the first probe lands — still never
        // guesses SDR: it returns the initial `Display::default` value.
        let empty: Cell<Option<FakeProbe>> = Cell::new(None);
        assert_eq!(sample_headroom(&empty, &last, FakeProbe::headroom), 3.0);
    }

    #[test]
    fn a_newer_probe_replaces_the_kept_one() {
        let slot: Cell<Option<FakeProbe>> = Cell::new(Some(FakeProbe {
            report: Cell::new(Some(2.0)),
            reads: Cell::new(0),
        }));
        let last = Cell::new(1.0);
        // The drain loop delivers a new probe: `WindowCherenkovSurface`
        // overwrites the slot, and the next sample reads the new one.
        slot.set(Some(FakeProbe {
            report: Cell::new(Some(5.0)),
            reads: Cell::new(0),
        }));
        assert_eq!(sample_headroom(&slot, &last, FakeProbe::headroom), 5.0);
    }
}
