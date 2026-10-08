//! The concrete Cherenkov GPU runtime (water-rs/hydrolysis#205, H1).
//!
//! One [`cherenkov::Engine<cherenkov_gpu::Gpu>`] per shared GPU context: every
//! surface's `DeviceLoss` carries the device creation chain its handles came
//! from, and the pool here
//! binds one engine to each. Surfaces a window presents through are
//! `TextureCherenkovSurface`s — engine `TextureTarget`s whose premultiplied
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
    /// it carries that window's own host wake.
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

/// Engine, surface and size shared by both typed surface wrappers.
pub struct SurfaceCore {
    engine: Rc<GpuEngine>,
    surface: Rc<cherenkov::Surface<cherenkov_gpu::Gpu>>,
    size: (u32, u32),
}

impl SurfaceCore {
    pub(crate) fn engine_surface(&self) -> &cherenkov::Surface<cherenkov_gpu::Gpu> {
        &self.surface
    }

    pub(crate) fn engine_surface_weak(
        &self,
    ) -> std::rc::Weak<cherenkov::Surface<cherenkov_gpu::Gpu>> {
        Rc::downgrade(&self.surface)
    }

    pub(crate) fn begin_frame(&self) -> cherenkov::FrameScope {
        self.surface.begin_frame()
    }

    pub(crate) fn resize(&mut self, size: (u32, u32)) {
        if self.size != size {
            self.size = size;
            self.surface
                .resize(size)
                .expect("hydrolysis renderer: engine surface resize failed");
        }
    }

    pub(crate) const fn size(&self) -> (u32, u32) {
        self.size
    }

    pub(crate) fn clear_color(&self, color: waterui_graphics::draw::WorkingColor) {
        self.surface.clear_color(color);
    }
}

pub trait SurfaceCoreAccess {
    fn core(&self) -> &SurfaceCore;
    fn core_mut(&mut self) -> &mut SurfaceCore;
    fn display(&mut self, scale: f64, headroom: f32);

    fn engine_surface(&self) -> &cherenkov::Surface<cherenkov_gpu::Gpu> {
        self.core().engine_surface()
    }

    fn engine_surface_weak(&self) -> std::rc::Weak<cherenkov::Surface<cherenkov_gpu::Gpu>> {
        self.core().engine_surface_weak()
    }

    fn begin_frame(&self) -> cherenkov::FrameScope {
        self.core().begin_frame()
    }

    fn resize(&mut self, size: (u32, u32)) {
        self.core_mut().resize(size);
    }

    fn clear_color(&self, color: waterui_graphics::draw::WorkingColor) {
        self.core().clear_color(color);
    }
}

/// Re-samples `probe`'s headroom into `last`: the probe is kept, never
/// consumed, a `None` report keeps `last`, and `last` is the result either
/// way. Generic so a test can drive the state machine with a fake probe —
/// a real [`DisplayProbe`] holds a live `wgpu::Surface`.
#[cfg(all(target_os = "macos", hydrolysis_winit))]
fn drain_and_sample<P>(
    receiver: &mpsc::Receiver<P>,
    slot: &mut Option<P>,
    last: &mut f32,
    headroom_of: impl Fn(&P) -> Option<f32>,
) -> (f32, bool) {
    let mut arrived = false;
    while let Ok(probe) = receiver.try_recv() {
        *slot = Some(probe);
        arrived = true;
    }
    let previous = *last;
    if let Some(headroom) = slot.as_ref().and_then(headroom_of) {
        *last = headroom;
    }
    (*last, arrived || previous.to_bits() != last.to_bits())
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
fn update_display_after_render(
    scale: f64,
    (headroom, changed): (f32, bool),
    display: impl FnOnce(cherenkov::Display),
    wake: impl FnOnce(),
) {
    if changed {
        display(cherenkov::Display { scale, headroom });
        wake();
    }
}

/// Engine output in a texture the host presents a copy of.
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

/// The macOS window target: the engine presents through `WindowTarget`, and
/// the host keeps and re-samples the output probe.
#[cfg(all(target_os = "macos", hydrolysis_winit))]
pub struct WindowCherenkovSurface {
    core: SurfaceCore,
    /// The channel `WindowTarget` pushes a `DisplayProbe` through per
    /// output configuration; drained on main by [`Self::headroom`].
    probe_rx: mpsc::Receiver<cherenkov_gpu::interop::DisplayProbe>,
    /// The latest probe, kept and re-sampled every frame — headroom
    /// follows brightness and display changes between pushes.
    probe: Option<cherenkov_gpu::interop::DisplayProbe>,
    /// The last headroom the probe reported; the engine's neutral
    /// `Display::default().headroom` until one has. A `None` from
    /// `tone_map_headroom` keeps it — SDR is never guessed.
    last_headroom: f32,
    wake: std::sync::Arc<dyn Fn() + Send + Sync>,
}

impl core::fmt::Debug for TextureCherenkovSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextureCherenkovSurface")
            .field("size", &self.core.size())
            .finish_non_exhaustive()
    }
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
impl WindowCherenkovSurface {
    pub(crate) fn new(
        engine: Rc<GpuEngine>,
        mut target: cherenkov_gpu::WindowTarget,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let size = target.size();
        let probe_rx = target.output_probe();
        let wake = std::sync::Arc::new(wake);
        let engine_wake = std::sync::Arc::clone(&wake);
        let surface = engine
            .surface(target, move || engine_wake())
            .expect("hydrolysis renderer: failed to create the Cherenkov window surface");
        Self {
            core: SurfaceCore {
                engine,
                surface: Rc::new(surface),
                size,
            },
            probe_rx,
            probe: None,
            last_headroom: cherenkov::Display::default().headroom,
            wake,
        }
    }

    /// The display's headroom, re-sampled every frame on main: the channel
    /// delivers the newest `DisplayProbe`, the kept probe is asked for its
    /// `tone_map_headroom`, and a `None` — no report for the current
    /// output configuration — keeps the last known value rather than
    /// guessing SDR.
    pub(crate) fn headroom(&mut self) -> (f32, bool) {
        drain_and_sample(
            &self.probe_rx,
            &mut self.probe,
            &mut self.last_headroom,
            cherenkov_gpu::interop::DisplayProbe::tone_map_headroom,
        )
    }

    /// The display's properties: `scale` is the logical-to-physical factor
    /// applied exactly once at the surface root; the headroom is the
    /// retained probe's latest sample, kept between samples.
    pub(crate) fn display(&mut self, scale: f64) {
        let (headroom, _) = self.headroom();
        self.core
            .surface
            .display(cherenkov::Display { scale, headroom })
            .expect("hydrolysis renderer: engine surface display update failed");
    }

    /// Announces a display move the platform observed
    /// (`NSWindowDidChangeScreenNotification`): the engine re-runs output
    /// negotiation and pushes a fresh probe.
    pub(crate) fn display_moved(&self) {
        self.core
            .surface
            .display_moved()
            .expect("hydrolysis renderer: engine surface display move failed");
    }
}

impl SurfaceCoreAccess for TextureCherenkovSurface {
    fn core(&self) -> &SurfaceCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut SurfaceCore {
        &mut self.core
    }

    fn display(&mut self, scale: f64, headroom: f32) {
        Self::display(self, scale, headroom);
    }
}

#[cfg(all(target_os = "macos", hydrolysis_winit))]
impl SurfaceCoreAccess for WindowCherenkovSurface {
    fn core(&self) -> &SurfaceCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut SurfaceCore {
        &mut self.core
    }

    fn display(&mut self, scale: f64, _headroom: f32) {
        Self::display(self, scale);
    }
}

impl TextureCherenkovSurface {
    /// Creates the texture-backed surface for the shared engine.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new(
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
    pub(crate) async fn new(
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
    pub(crate) fn display(&self, scale: f64, headroom: f32) {
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
    pub(crate) fn render(&mut self) -> Result<cherenkov::Next, cherenkov::RenderError> {
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
    pub(crate) async fn render(&mut self) -> Result<cherenkov::Next, cherenkov::RenderError> {
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
    pub(crate) fn present_into(
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
    pub(crate) fn render(&mut self, scale: f64) -> Result<cherenkov::Next, cherenkov::RenderError> {
        let next = self.core.engine.render(cherenkov::FrameTime::now())?;
        update_display_after_render(
            scale,
            self.headroom(),
            |display| {
                self.core
                    .surface
                    .display(display)
                    .expect("hydrolysis renderer: engine surface display update failed");
            },
            || (self.wake)(),
        );
        Ok(next)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg(all(test, target_os = "macos", hydrolysis_winit))]
mod tests {
    use std::cell::Cell;

    use super::{drain_and_sample, update_display_after_render};

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
        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(FakeProbe {
                report: Cell::new(Some(2.0)),
                reads: Cell::new(0),
            })
            .unwrap();
        let mut probe = None;
        let mut last = 1.0;

        // Every call re-samples the kept probe — the first report lands.
        assert_eq!(
            drain_and_sample(&receiver, &mut probe, &mut last, FakeProbe::headroom),
            (2.0, true)
        );
        let kept = probe.take().expect("the probe is kept, not consumed");
        assert_eq!(kept.reads.get(), 1);
        // Let the display report a new headroom: the next sample follows it
        // without a new probe arriving.
        kept.report.set(Some(4.0));
        probe = Some(kept);
        assert_eq!(
            drain_and_sample(&receiver, &mut probe, &mut last, FakeProbe::headroom),
            (4.0, true)
        );
        assert_eq!(probe.take().expect("still kept").reads.get(), 2);
    }

    #[test]
    fn a_none_report_keeps_the_last_headroom() {
        let (_sender, receiver) = std::sync::mpsc::channel();
        let mut probe = Some(FakeProbe {
            report: Cell::new(None),
            reads: Cell::new(0),
        });
        let mut last = 3.0;
        assert_eq!(
            drain_and_sample(&receiver, &mut probe, &mut last, FakeProbe::headroom),
            (3.0, false)
        );
        assert_eq!(last.to_bits(), 3.0_f32.to_bits());
        // And an empty slot — before the first probe lands — still never
        // guesses SDR: it returns the initial `Display::default` value.
        let mut empty: Option<FakeProbe> = None;
        assert_eq!(
            drain_and_sample(&receiver, &mut empty, &mut last, FakeProbe::headroom),
            (3.0, false)
        );
    }

    #[test]
    fn a_newer_probe_replaces_the_kept_one() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let mut slot: Option<FakeProbe> = Some(FakeProbe {
            report: Cell::new(Some(2.0)),
            reads: Cell::new(0),
        });
        let mut last = 1.0;
        // The drain loop delivers every queued probe: the newest replaces
        // the kept one and is the only one sampled.
        sender
            .send(FakeProbe {
                report: Cell::new(Some(4.0)),
                reads: Cell::new(0),
            })
            .unwrap();
        sender
            .send(FakeProbe {
                report: Cell::new(Some(5.0)),
                reads: Cell::new(0),
            })
            .unwrap();
        assert_eq!(
            drain_and_sample(&receiver, &mut slot, &mut last, FakeProbe::headroom),
            (5.0, true)
        );
    }

    #[test]
    fn first_probe_requests_an_update_even_when_headroom_is_unchanged() {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(FakeProbe {
                report: Cell::new(Some(2.0)),
                reads: Cell::new(0),
            })
            .unwrap();
        let mut probe = None;
        let mut last = 2.0;

        assert_eq!(
            drain_and_sample(&receiver, &mut probe, &mut last, FakeProbe::headroom),
            (2.0, true)
        );
    }

    #[test]
    fn a_probe_arriving_after_render_updates_display_and_requests_redraw() {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender
            .send(FakeProbe {
                report: Cell::new(Some(2.0)),
                reads: Cell::new(0),
            })
            .unwrap();
        let mut probe = None;
        let mut last = 2.0;
        let sample = drain_and_sample(&receiver, &mut probe, &mut last, FakeProbe::headroom);
        let display_updates = Cell::new(0);
        let redraws = Cell::new(0);

        update_display_after_render(
            1.5,
            sample,
            |_| display_updates.set(display_updates.get() + 1),
            || redraws.set(redraws.get() + 1),
        );

        assert_eq!(display_updates.get(), 1);
        assert_eq!(redraws.get(), 1);
    }
}
