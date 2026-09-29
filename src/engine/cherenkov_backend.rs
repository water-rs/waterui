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

/// The production backend: the concrete wgpu engine, never a second
/// rendering stack or a runtime-selected one.
pub(crate) type GpuEngine = cherenkov::Engine<cherenkov_gpu::Gpu>;

thread_local! {
    /// Engines alive on this thread, keyed by GPU-context identity. Weak so a
    /// context whose surfaces have all dropped releases its engine instead of
    /// pinning it for the thread's lifetime.
    static ENGINES: RefCell<FxHashMap<u64, Weak<GpuEngine>>> = RefCell::new(FxHashMap::default());
}

/// The shared engine for `context_id`'s GPU context, created on first use.
///
/// `wake` is the host's display-link wake: it may be invoked from any thread
/// the engine or its producers run on. The callback passed on the creating
/// call wins; later calls for the same context leave it unchanged — every
/// window on the shared context wakes the same event loop.
pub(crate) fn shared_engine(
    context_id: u64,
    adapter: &wgpu::Adapter,
    shared_device: cherenkov_gpu::interop::SharedDevice,
    wake: impl Fn() + Send + Sync + 'static,
) -> Rc<GpuEngine> {
    ENGINES.with(|pool| {
        if let Some(engine) = pool.borrow().get(&context_id).and_then(Weak::upgrade) {
            return engine;
        }
        let config = cherenkov_gpu::GpuConfig {
            device: Some(shared_device),
            redraw: Some(cherenkov_gpu::interop::RedrawCallback::new(wake)),
            pipeline_cache: pipeline_cache_path(adapter),
            ..cherenkov_gpu::GpuConfig::default()
        };
        let engine = Rc::new(
            GpuEngine::new(config)
                .expect("hydrolysis renderer: failed to create the Cherenkov engine"),
        );
        pool.borrow_mut().insert(context_id, Rc::downgrade(&engine));
        engine
    })
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
fn pipeline_cache_path(_adapter: &wgpu::Adapter) -> Option<std::path::PathBuf> {
    None
}

/// A window's output surface: an engine surface rendering into an
/// engine-owned `Rgba16Float` linear-P3 texture the host presents.
///
/// The texture notification channel fires on creation and on every resize;
/// [`Self::render`] drains it before the presenter samples — sampling
/// post-resize without draining is the stale-attachment case the plan bans.
pub(crate) struct CherenkovSurface {
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
    pub(crate) fn new(
        engine: Rc<GpuEngine>,
        device: &wgpu::Device,
        backend: wgpu::Backend,
        size: (u32, u32),
    ) -> Self {
        let (target, textures) = cherenkov_gpu::interop::TextureTarget::new(size);
        let surface = engine
            .surface(target)
            .expect("hydrolysis renderer: failed to create the Cherenkov surface");
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

    /// The engine this surface renders with.
    pub(crate) fn engine(&self) -> &Rc<GpuEngine> {
        &self.engine
    }

    /// The engine surface behind this output target — mount, edit and
    /// transaction calls route through it.
    pub(crate) fn engine_surface(&self) -> &cherenkov::Surface<cherenkov_gpu::Gpu> {
        &self.surface
    }

    /// Queues a resize when `size` changed; the next [`Self::render`] drains
    /// the replacement texture notification before the presenter samples.
    pub(crate) fn resize(&mut self, size: (u32, u32)) {
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
    pub(crate) fn display(&self, scale: f64, headroom: f32) {
        self.surface
            .display(cherenkov::Display { scale, headroom })
            .expect("hydrolysis renderer: engine surface display update failed");
    }

    /// The colour the surface clears to before content.
    pub(crate) fn clear_color(&self, color: cherenkov::WorkingColor) {
        self.surface.clear_color(color);
    }

    /// Renders the committed change set and drains texture notifications,
    /// returning the engine's `Next` for the pump to schedule against.
    ///
    /// The rendered texture stays held in [`Self::texture`]; the presenter
    /// samples it through [`Self::present_into`].
    pub(crate) fn render(&mut self) -> cherenkov::Next {
        let next = self
            .engine
            .render(cherenkov::FrameTime::now())
            .expect("hydrolysis renderer: engine render failed");
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
        premultiplied: bool,
        headroom: f32,
    ) {
        let (_, view) = self
            .texture
            .as_ref()
            .expect("hydrolysis renderer: present before the engine produced a texture");
        let color = if output.format().is_srgb() {
            cherenkov_gpu::interop::OutputColor::Srgb
        } else {
            cherenkov_gpu::interop::OutputColor::LinearDisplayP3
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
