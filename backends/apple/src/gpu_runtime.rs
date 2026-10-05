//! The process GPU runtime: created asynchronously on the shared executor,
//! installed into the environment before the fallback's services — the Rust
//! half of what `WuiGpuRuntime.swift` + the `waterui_gpu_runtime_*` FFI did.

#[cfg(feature = "gpu_surface")]
use cocoa_ui::Retained;
use executor_core::{spawn, spawn_local};
#[cfg(feature = "gpu_surface")]
use objc2::runtime::ProtocolObject;
#[cfg(feature = "gpu_surface")]
use objc2_metal::MTLDevice;
#[cfg(feature = "gpu_surface")]
use std::cell::{Cell, RefCell};
#[cfg(feature = "gpu_surface")]
use std::rc::{Rc, Weak};
#[cfg(feature = "gpu_surface")]
use std::sync::Arc;
use waterui_backend_core::Environment;
#[cfg(feature = "gpu_surface")]
use waterui_graphics::cherenkov::{Engine, EngineError, FrameTime, RenderError, SurfaceError};
#[cfg(feature = "gpu_surface")]
use waterui_graphics::cherenkov_gpu::Gpu;
use waterui_graphics::gpu::GpuRuntime;
#[cfg(feature = "gpu_surface")]
use waterui_graphics::gpu::SharedGpuContext;
#[cfg(feature = "gpu_surface")]
use wgpu_hal::api::Metal as MetalApi;

/// Creates the runtime on the shared executor, installs it into `env` on the
/// main thread, then runs `then`. The owner retains `env` until setup and
/// the completion callback finish.
///
/// # Safety
///
/// `env` must be a valid, live `Environment` until `then` finishes and
/// `then` runs on the main thread.
pub unsafe fn prepare(env: *mut Environment, then: impl FnOnce() + 'static) {
    let (sender, receiver) = async_channel::bounded(1);
    spawn(async move {
        let runtime = GpuRuntime::new().await;
        let _ = sender.send(runtime).await;
    })
    .detach();
    spawn_local(async move {
        let runtime = receiver
            .recv()
            .await
            .expect("GPU runtime creation task ended without producing a runtime")
            .unwrap_or_else(|error| panic!("GPU runtime creation failed: {error}"));
        // SAFETY: `env` is lent for the process and this task is pinned to the
        // main executor — the same thread the launch handler runs on.
        let env = unsafe { &mut *env };
        env.insert(runtime);
        #[cfg(feature = "gpu_surface")]
        {
            // The shared presentation anchor and the per-generation scene
            // engine sit beside the runtime: every presenter maps the same
            // media timestamp to the same instant, and every mounted
            // `SceneView` shares one engine per context generation.
            crate::presentation_time::PresentationTime::install(env);
            env.insert(Rc::new(SceneEngine::new()));
        }
        then();
    })
    .detach();
}

/// The environment's GPU runtime.
///
/// # Panics
///
/// When no runtime was installed — [`prepare`] runs before any surface or
/// effect can render, so a missing runtime is a launch error.
#[cfg(feature = "gpu_surface")]
pub fn runtime(env: &Environment) -> GpuRuntime {
    env.get::<GpuRuntime>()
        .expect("GPU runtime is not installed in the WaterUI environment")
        .clone()
}

/// The `MTLDevice` a context generation's wgpu device wraps.
///
/// # Panics
///
/// When the runtime's device is not Metal or the device pointer is null.
#[cfg(feature = "gpu_surface")]
pub fn raw_metal_device(context: &SharedGpuContext) -> Retained<ProtocolObject<dyn MTLDevice>> {
    // SAFETY: `raw_device` is the `MTLDevice` the runtime created and still
    // owns; `retain` takes our own reference on it.
    unsafe {
        Retained::retain(
            Retained::as_ptr(
                context
                    .device()
                    .as_hal::<MetalApi>()
                    .expect("the Apple runtime's device is Metal")
                    .raw_device(),
            )
            .cast_mut(),
        )
        .expect("the Metal device is non-null")
    }
}

/// What creating, preparing or rendering through a scene engine generation
/// can fail with — a typed result on the frame path, where a panic is a
/// process abort, instead of an `expect` across an Objective-C callback.
#[cfg(feature = "gpu_surface")]
#[derive(Debug)]
#[non_exhaustive]
pub enum SceneError {
    /// The shared engine could not be created on this context generation.
    Engine(EngineError),
    /// The presentation shader set could not be built.
    Shaders(EngineError),
    /// The scene's surface rejected a resize or display update.
    Surface(SurfaceError),
    /// The produced batch's shared engine render failed.
    Render(RenderError),
    /// The created surface published no texture — the engine/target
    /// contract is broken, not a recoverable state.
    MissingTexture,
}

#[cfg(feature = "gpu_surface")]
impl core::fmt::Display for SceneError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Engine(error) => write!(f, "scene engine creation failed: {error}"),
            Self::Shaders(error) => write!(f, "scene presentation shaders failed: {error}"),
            Self::Surface(error) => write!(f, "scene surface failed: {error}"),
            Self::Render(error) => write!(f, "scene engine render failed: {error}"),
            Self::MissingTexture => write!(f, "scene surface published no texture"),
        }
    }
}

#[cfg(feature = "gpu_surface")]
impl std::error::Error for SceneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Engine(error) | Self::Shaders(error) => Some(error),
            Self::Surface(error) => Some(error),
            Self::Render(error) => Some(error),
            Self::MissingTexture => None,
        }
    }
}

#[cfg(feature = "gpu_surface")]
impl From<SurfaceError> for SceneError {
    fn from(error: SurfaceError) -> Self {
        Self::Surface(error)
    }
}

#[cfg(feature = "gpu_surface")]
impl From<RenderError> for SceneError {
    fn from(error: RenderError) -> Self {
        Self::Render(error)
    }
}

/// A mounted scene's preparation hook inside an [`EngineGeneration`]:
/// applies the scene's pending content/geometry/display changes onto its
/// own surface before the shared batch renders. Each scene implements it
/// on its own surface state; the generation holds it weakly, so an
/// unmounted scene leaves the batch on its own.
#[cfg(feature = "gpu_surface")]
pub trait SceneParticipant {
    /// Applies everything staged for this scene into its own surface and
    /// records `time` as the batch it prepared for — on failure the scene
    /// keeps the error for its own presenter to report and is simply not
    /// produced at `time`; it does not poison the batch.
    fn prepare(&self, time: FrameTime);
    /// Whether the batch produced at `time` contains this scene's latest
    /// staged state. `false` after a later mount, restage or failed
    /// prepare — the scene is then explicitly owed a later frame and must
    /// not composite a stale texture into this timestamp.
    fn produced_at(&self, time: FrameTime) -> bool;
    /// Routes a batch-level failure to this participant — a shared render
    /// failure belongs to every mounted scene, not only the requester that
    /// happened to drive `produce`.
    fn note_failure(&self, failure: Rc<SceneError>);
    /// The scene's own failure — its prepare error or a batch failure the
    /// generation routed to it — once; a repeated read returns `None`.
    fn take_failure(&self) -> Option<Rc<SceneError>>;
}

/// One engine generation: the shared cherenkov engine, the exact
/// [`SharedGpuContext`] generation it was created on, the mounted scenes'
/// weak preparation hooks, and the timestamp the last batch produced.
/// Retained through `Rc` by the owner and every mounted scene — when a new
/// context generation replaces this one, the engine and every device-bound
/// resource die with its last user.
#[cfg(feature = "gpu_surface")]
pub struct EngineGeneration {
    /// The shared engine every mounted `SceneView` renders through — one
    /// engine thread total, not one per view.
    engine: Rc<Engine<Gpu>>,
    /// The exact context generation this engine belongs to — a retired
    /// engine can never present into a new generation's drawable.
    context: Arc<SharedGpuContext>,
    /// The mounted scenes' preparation hooks, weak so a dropped scene is
    /// pruned on the next batch rather than kept prepared.
    participants: RefCell<Vec<Weak<dyn SceneParticipant>>>,
    /// The production target timestamp the last batch rendered — exact
    /// equality: two requests at the same timestamp are one engine frame.
    produced: Cell<Option<FrameTime>>,
    /// The shared render failure once it settles: every later `produce`
    /// answers the same `Rc`'d failure without rerunning the frame — a
    /// generation never recovers, only a new context generation does.
    failure: RefCell<Option<Rc<SceneError>>>,
}

#[cfg(feature = "gpu_surface")]
impl EngineGeneration {
    /// The shared engine this generation renders through.
    pub const fn engine(&self) -> &Rc<Engine<Gpu>> {
        &self.engine
    }

    /// The exact context generation this engine is bound to.
    pub const fn context(&self) -> &Arc<SharedGpuContext> {
        &self.context
    }

    /// Registers a mounted scene's preparation hook, held weakly.
    pub fn mount(&self, participant: &Rc<dyn SceneParticipant>) {
        self.participants
            .borrow_mut()
            .push(Rc::downgrade(participant));
    }

    /// The live mounted scenes, dead hooks pruned.
    fn live(&self) -> Vec<Rc<dyn SceneParticipant>> {
        let mut participants = self.participants.borrow_mut();
        participants.retain(|weak| weak.upgrade().is_some());
        participants.iter().filter_map(Weak::upgrade).collect()
    }

    /// Settles a shared batch failure: retained on the generation so every
    /// later `produce` answers the same `Rc`'d failure without rerunning,
    /// and routed to every live participant so no requester waits forever
    /// or retries a frame another presenter already saw fail.
    fn settle_failed(&self, error: SceneError) -> Rc<SceneError> {
        let failure = Rc::new(error);
        *self.failure.borrow_mut() = Some(failure.clone());
        for participant in self.live() {
            participant.note_failure(failure.clone());
        }
        failure
    }

    /// The retained typed failure this generation was sealed with, if
    /// any — immutable once set; a retained old generation stays failed.
    pub fn failure(&self) -> Option<Rc<SceneError>> {
        self.failure.borrow().clone()
    }

    /// The one engine frame for `time`: on the first request at this exact
    /// timestamp every live participant applies its staged content,
    /// geometry and display changes and the engine renders once; later
    /// requests at the same timestamp reuse that batch — a scene mounted or
    /// invalidated after it stays unproduced and is owed a later frame,
    /// never silently treated as rendered.
    ///
    /// # Errors
    ///
    /// The shared [`Rc<SceneError>`] — [`SceneError::Render`] — when the
    /// shared frame fails: the failure is retained on the generation,
    /// routed to every live participant through `note_failure`, and
    /// answered unchanged to every later `produce` — a failed generation
    /// never retries; only a new context generation produces fresh
    /// resources. `produced` is not stamped, so no participant consumes
    /// the failed batch as rendered.
    pub fn produce(&self, time: FrameTime) -> Result<(), Rc<SceneError>> {
        if let Some(failure) = self.failure.borrow().as_ref() {
            return Err(failure.clone());
        }
        if self.produced.get() == Some(time) {
            return Ok(());
        }
        for participant in &self.live() {
            participant.prepare(time);
        }
        match self.engine.render(time) {
            Ok(_) => {
                self.produced.set(Some(time));
                Ok(())
            }
            Err(error) => Err(self.settle_failed(SceneError::Render(error))),
        }
    }

    /// Settles the failed state exactly as a failed shared render does —
    /// retained, routed to every live participant, returned to every later
    /// `produce`. Test-only entry into the same path.
    #[cfg(test)]
    pub fn fail_for_testing(&self, error: SceneError) -> Rc<SceneError> {
        self.settle_failed(error)
    }
}

/// The scene engine owner installed beside the runtime (#1725): one
/// cherenkov engine per exact [`SharedGpuContext`] generation, shared by
/// every mounted `SceneView`. Scenes keep their own surface, texture
/// target, held registrations, resources and presenter — the generation
/// owns only the engine, the weak participants and the timestamp batching,
/// so one engine thread drives all scenes while each keeps independent
/// frame demand.
#[cfg(feature = "gpu_surface")]
pub struct SceneEngine {
    /// The settled outcome for the context generation last seen — the
    /// shared generation, or the `Rc`'d creation failure that generation
    /// produced — keyed by the exact generation number. `None` until the
    /// first mount asks; a new context generation replaces the entry,
    /// everything else reuses it without rerunning creation.
    current: RefCell<Option<(u64, GenerationOutcome)>>,
}

/// What one context generation settled to — the shared engine generation,
/// or the `Rc`'d creation failure every mount on it receives unchanged.
#[cfg(feature = "gpu_surface")]
type GenerationOutcome = Result<Rc<EngineGeneration>, Rc<SceneError>>;

#[cfg(feature = "gpu_surface")]
impl SceneEngine {
    /// An owner with no generation yet — the first mount creates it.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current: RefCell::new(None),
        }
    }

    /// The engine generation `context` belongs to: identical while the
    /// exact context generation stands, replaced when the runtime publishes
    /// a new one — a retained old generation cannot present into a new
    /// drawable and dies with its last scene.
    ///
    /// # Errors
    ///
    /// The shared [`Rc<SceneError>`] — [`SceneError::Engine`] — when the
    /// engine cannot be created on this context. The failure is cached
    /// against this exact generation: every later mount under it receives
    /// the same typed failure rather than rerunning creation, and a failed
    /// generation is never a ready one. Only a new context generation
    /// attempts creation again.
    pub fn generation(
        &self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
    ) -> Result<Rc<EngineGeneration>, Rc<SceneError>> {
        let key = context.generation();
        if let Some((cached_key, outcome)) = self.current.borrow().as_ref()
            && *cached_key == key
        {
            return outcome.clone();
        }
        let outcome = runtime
            .engine_on(context)
            .map(|engine| {
                Rc::new(EngineGeneration {
                    engine: Rc::new(engine),
                    context: context.clone(),
                    participants: RefCell::new(Vec::new()),
                    produced: Cell::new(None),
                    failure: RefCell::new(None),
                })
            })
            .map_err(|error| Rc::new(SceneError::Engine(error)));
        *self.current.borrow_mut() = Some((key, outcome.clone()));
        outcome
    }

    /// Installs the outcome a failed creation cached for `key` — the same
    /// entry `generation` writes when `engine_on` returns `Err`, so a test
    /// can verify every later mount receives that exact typed failure
    /// without rerunning creation.
    #[cfg(test)]
    pub fn install_failure_for_testing(&self, key: u64, error: SceneError) {
        *self.current.borrow_mut() = Some((key, Err(Rc::new(error))));
    }
}

/// The environment's shared scene engine owner.
///
/// # Panics
///
/// When no owner was installed — [`prepare`] installs it beside the runtime
/// before any mount can render.
#[cfg(feature = "gpu_surface")]
pub fn scene_engine(env: &Environment) -> Rc<SceneEngine> {
    env.get::<Rc<SceneEngine>>()
        .expect("scene engine is not installed in the WaterUI environment")
        .clone()
}

#[cfg(all(test, target_os = "macos", feature = "gpu_surface"))]
mod tests {
    use super::*;
    use waterui_graphics::cherenkov::Instant;

    /// A mounted scene's contract reduced to observation: whether `prepare`
    /// ran and for which timestamp, whether it is owed a later frame, and
    /// the failure a batch routed to it.
    struct Probe {
        prepares: Cell<u32>,
        prepared: Cell<Option<FrameTime>>,
        pending: Cell<bool>,
        failure: RefCell<Option<Rc<SceneError>>>,
    }

    impl Probe {
        fn mount(generation: &Rc<EngineGeneration>) -> Rc<Self> {
            let probe = Rc::new(Self {
                prepares: Cell::new(0),
                prepared: Cell::new(None),
                pending: Cell::new(false),
                failure: RefCell::new(None),
            });
            let participant: Rc<dyn SceneParticipant> = probe.clone();
            generation.mount(&participant);
            probe
        }
    }

    impl SceneParticipant for Probe {
        fn prepare(&self, time: FrameTime) {
            self.prepares.set(self.prepares.get() + 1);
            self.prepared.set(Some(time));
            self.pending.set(false);
        }
        fn produced_at(&self, time: FrameTime) -> bool {
            self.prepared.get() == Some(time) && !self.pending.get()
        }
        fn note_failure(&self, failure: Rc<SceneError>) {
            *self.failure.borrow_mut() = Some(failure);
        }
        fn take_failure(&self) -> Option<Rc<SceneError>> {
            self.failure.borrow_mut().take()
        }
    }

    fn gpu() -> (GpuRuntime, Arc<SharedGpuContext>) {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        (runtime, context)
    }

    /// A failed initialization is never a ready generation: two mounts on
    /// the same exact context generation receive the same typed failure —
    /// the same `Rc`, so no second creation attempt ran — and only a new
    /// context generation settles successfully again.
    #[test]
    fn failed_creation_serves_one_typed_failure_to_every_mount() {
        let (runtime, context) = gpu();
        let engines = SceneEngine::new();
        engines.install_failure_for_testing(context.generation(), SceneError::MissingTexture);
        let first = engines
            .generation(&runtime, &context)
            .err()
            .expect("the cached failure reaches the first mount");
        let second = engines
            .generation(&runtime, &context)
            .err()
            .expect("the same failure reaches the second mount");
        assert!(
            Rc::ptr_eq(&first, &second),
            "creation ran once: both mounts carry the same owned failure"
        );
        assert!(
            matches!(*first, SceneError::MissingTexture),
            "the typed failure survives unchanged"
        );
    }

    /// One shared render failure settles once for the whole batch: both
    /// requesters see the same `Rc`'d failure, the generation retains it so
    /// no later `produce` reruns the frame, and every live participant's
    /// readiness owner receives it — the same failure each mount reports
    /// through `take_failure`.
    #[test]
    fn failed_batch_settles_every_requester_without_retry() {
        let (runtime, context) = gpu();
        let engines = SceneEngine::new();
        let generation = engines
            .generation(&runtime, &context)
            .expect("the generation settles");
        let first_part = Probe::mount(&generation);
        let second_part = Probe::mount(&generation);
        let time = FrameTime(Instant::now());

        generation.produce(time).expect("the first batch produces");
        assert_eq!(first_part.prepares.get(), 1);
        // The shared render then fails — settled exactly once.
        let routed = generation.fail_for_testing(SceneError::MissingTexture);

        let first = generation
            .produce(time)
            .expect_err("a settled generation never retries");
        let second = generation
            .produce(time)
            .expect_err("the second requester gets the same failure");
        assert!(Rc::ptr_eq(&first, &second));
        assert!(Rc::ptr_eq(&first, &routed));
        assert_eq!(
            first_part.prepares.get(),
            1,
            "no frame reran after the failure settled"
        );
        assert!(
            Rc::ptr_eq(&first_part.take_failure().expect("routed"), &routed),
            "the first participant's readiness settles on the shared failure"
        );
        assert!(
            Rc::ptr_eq(&second_part.take_failure().expect("routed"), &routed),
            "the second participant settles identically"
        );
        assert!(
            first_part.take_failure().is_none(),
            "a settled failure reports once"
        );
        // The retained generation stays failed — not even a new timestamp
        // reruns a frame on it.
        let later = FrameTime(Instant::now());
        assert!(
            generation
                .produce(later)
                .is_err_and(|error| Rc::ptr_eq(&error, &routed)),
            "only a new context generation produces again"
        );
    }

    /// A real context replacement is the only recovery: a new
    /// `SharedGpuContext` generation yields a fresh engine generation that
    /// produces again, while the retained failed one stays settled.
    #[test]
    fn only_a_new_context_generation_recovers() {
        let (runtime, context) = gpu();
        let engines = SceneEngine::new();
        let generation = engines
            .generation(&runtime, &context)
            .expect("the generation settles");
        generation.fail_for_testing(SceneError::MissingTexture);

        context.mark_device_lost_for_testing("test device loss");
        let fresh = pollster::block_on(runtime.context_after(context.generation()));
        assert!(fresh.generation() > context.generation());
        let recovered = engines
            .generation(&runtime, &fresh)
            .expect("a new generation creates fresh resources");
        assert!(
            !Rc::ptr_eq(&generation, &recovered),
            "the replacement is a genuinely new generation"
        );
        let probe = Probe::mount(&recovered);
        let time = FrameTime(Instant::now());
        recovered
            .produce(time)
            .expect("the fresh generation produces");
        assert!(probe.produced_at(time));
        assert!(
            generation
                .produce(time)
                .is_err_and(|error| matches!(*error, SceneError::MissingTexture)),
            "the retained generation stays failed"
        );
    }

    /// Two requests at the same exact target timestamp are one engine
    /// frame: each participant prepared once, and both read the batch as
    /// produced — late restaging keeps the surface owed a later frame.
    #[test]
    fn one_timestamp_one_frame_and_late_dirt_is_owed() {
        let (runtime, context) = gpu();
        let engines = SceneEngine::new();
        let generation = engines
            .generation(&runtime, &context)
            .expect("the generation settles");
        let first_part = Probe::mount(&generation);
        let second_part = Probe::mount(&generation);
        let time = FrameTime(Instant::now());

        generation
            .produce(time)
            .expect("first requester runs the batch");
        generation
            .produce(time)
            .expect("the same timestamp reuses the batch");
        assert_eq!(first_part.prepares.get(), 1, "one engine frame ran");
        assert_eq!(second_part.prepares.get(), 1);
        assert!(first_part.produced_at(time));
        assert!(second_part.produced_at(time));

        // Dirt arriving after the produced timestamp is owed a later frame,
        // not silently acknowledged by the reused batch.
        second_part.pending.set(true);
        assert!(
            !second_part.produced_at(time),
            "late dirt stays unproduced at this timestamp"
        );
        let later = FrameTime(Instant::now());
        generation
            .produce(later)
            .expect("the owed frame produces at its own timestamp");
        assert!(second_part.produced_at(later));
        assert_eq!(second_part.prepares.get(), 2);
    }
}
