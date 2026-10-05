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
    /// or capture mount on this context holds it.
    ///
    /// `wake` is the host's display-link wake: it may be invoked from any thread
    /// the engine or its producers run on. The callback passed on the creating
    /// call wins; later calls for the same context leave it unchanged — every
    /// window on the shared context wakes the same event loop.
    ///
    /// Async on wasm32, where `Engine::new` awaits the browser's GPU device.
    pub fn shared_engine_state(
        context_id: u64,
        adapter: &wgpu::Adapter,
        shared_device: cherenkov_gpu::interop::SharedDevice,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Rc<SharedEngineState> {
        let pooled =
            ENGINES.with(|pool| pool.borrow().get(&context_id).and_then(Weak::upgrade));
        if let Some(state) = pooled {
            return state;
        }
        let config = cherenkov_gpu::GpuConfig {
            device: Some(shared_device),
            redraw: Some(cherenkov_gpu::interop::RedrawCallback::new(wake)),
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

/// A window's output surface: an engine surface rendering into an
/// engine-owned `Rgba16Float` linear-P3 texture the host presents.
///
/// The texture notification channel fires on creation and on every resize;
/// [`Self::render`] drains it before the presenter samples — sampling
/// post-resize without draining is the stale-attachment case the plan bans.
pub struct CherenkovSurface {
    engine: Rc<GpuEngine>,
    surface: cherenkov::Surface<cherenkov_gpu::Gpu>,
    textures: mpsc::Receiver<wgpu::Texture>,
    texture: Option<(wgpu::Texture, wgpu::TextureView)>,
    presenter: cherenkov_gpu::interop::Presenter,
    size: (u32, u32),
}

impl core::fmt::Debug for CherenkovSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CherenkovSurface")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl CherenkovSurface {
    /// Creates the engine surface at `size` (physical pixels) and takes the
    /// presenter for `device`'s shader delivery.
    ///
    /// Async on wasm32, where `Engine::surface` awaits the browser device.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
    ) -> Self {
        let (target, textures) = cherenkov_gpu::interop::TextureTarget::new(size);
        let surface = engine
            .surface(target)
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
    pub async fn new(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
    ) -> Self {
        let (target, textures) = cherenkov_gpu::interop::TextureTarget::new(size);
        let surface = engine
            .surface(target)
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
            engine,
            surface,
            textures,
            texture: None,
            presenter: cherenkov_gpu::interop::Presenter::new(device, delivery),
            size,
        }
    }

    /// The engine surface behind this output target — mount, edit and
    /// transaction calls route through it.
    pub const fn engine_surface(&self) -> &cherenkov::Surface<cherenkov_gpu::Gpu> {
        &self.surface
    }

    /// Queues a resize when `size` changed; the next [`Self::render`] drains
    /// the replacement texture notification before the presenter samples.
    pub fn resize(&mut self, size: (u32, u32)) {
        if self.size == size {
            return;
        }
        self.size = size;
        self.surface
            .resize(size)
            .expect("hydrolysis renderer: engine surface resize failed");
    }

    /// The display's properties: `scale` is the logical-to-physical factor
    /// applied exactly once at the surface root, `headroom` the HDR headroom
    /// the display reaches.
    pub fn display(&self, scale: f64, headroom: f32) {
        self.surface
            .display(cherenkov::Display { scale, headroom })
            .expect("hydrolysis renderer: engine surface display update failed");
    }

    /// The colour the surface clears to before content.
    pub fn clear_color(&self, color: cherenkov::WorkingColor) {
        self.surface.clear_color(color);
    }

    /// Renders the committed change set and drains texture notifications,
    /// returning the engine's `Next` for the pump to schedule against.
    ///
    /// The rendered texture stays held in [`Self::texture`]; the presenter
    /// samples it through [`Self::present_into`].
    ///
    /// Async on wasm32, where `Engine::render` awaits the browser device.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(&mut self) -> cherenkov::Next {
        self.render_inner(
            self.engine
                .render(cherenkov::FrameTime::now())
                .expect("hydrolysis renderer: engine render failed"),
        )
    }

    /// [`Self::render`], async on wasm32 where `Engine::render` awaits the
    /// browser device.
    #[cfg(target_arch = "wasm32")]
    #[allow(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the engine's Rc handles never cross a thread"
    )]
    pub async fn render(&mut self) -> cherenkov::Next {
        let next = self
            .engine
            .render(cherenkov::FrameTime::now())
            .await
            .expect("hydrolysis renderer: engine render failed");
        self.render_inner(next)
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
        premultiplied: bool,
        headroom: f32,
    ) {
        let (_, view) = self
            .texture
            .as_ref()
            .expect("hydrolysis renderer: present before the engine produced a texture");
        let color = if matches!(
            output.format().remove_srgb_suffix(),
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
        ) {
            cherenkov_gpu::interop::OutputColor::LinearDisplayP3
        } else {
            cherenkov_gpu::interop::OutputColor::Srgb
        };
        let alpha = if premultiplied {
            cherenkov_gpu::interop::OutputAlpha::Premultiplied
        } else {
            cherenkov_gpu::interop::OutputAlpha::Straight
        };
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
