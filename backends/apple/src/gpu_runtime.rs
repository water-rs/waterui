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
use waterui_graphics::cherenkov::{Engine, FrameScope, FrameTime};
#[cfg(feature = "gpu_surface")]
use waterui_graphics::cherenkov_gpu::Gpu;
use waterui_graphics::gpu::GpuRuntime;
#[cfg(feature = "gpu_surface")]
use waterui_graphics::gpu::{HostedLayerError, SharedGpuContext};
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

/// The display's maximum frame rate for the view's current screen —
/// `None` when the view is not in a window.
#[cfg(feature = "gpu_surface")]
pub fn display_rate(view: &cocoa_ui::PlatformView) -> Option<f32> {
    let window = cocoa_ui::view::window(view)?;
    #[cfg(target_os = "macos")]
    {
        window.screen().map(|screen| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "frame rates fit comfortably in f32"
            )]
            let rate = screen.maximumFramesPerSecond() as f32;
            rate
        })
    }
    #[cfg(target_os = "ios")]
    {
        window.windowScene().map(|scene| {
            #[expect(
                clippy::cast_precision_loss,
                reason = "frame rates fit comfortably in f32"
            )]
            let rate = scene.screen().maximumFramesPerSecond() as f32;
            rate
        })
    }
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

/// A mounted scene's preparation hook inside an [`EngineGeneration`]:
/// applies the scene's pending content/geometry/display changes onto its
/// own surface before the shared batch renders. Each scene implements it
/// on its own surface state; the generation holds it weakly, so an
/// unmounted scene leaves the batch on its own.
#[cfg(feature = "gpu_surface")]
pub trait SceneParticipant {
    /// Applies everything staged for this scene into its own surface and
    /// records `time` as the batch it prepared for — on failure the scene
    /// routes the error to its owner through the same sink a batch
    /// failure takes and is simply not produced at `time`; it does not
    /// poison the batch.
    ///
    /// Returns the scene surface's frame scope, which the batch holds
    /// across its render, so the edits made here wake no host; a failed
    /// preparation's scope has already ended.
    ///
    /// # Errors
    ///
    /// The scene's own typed failure — `resize`'s rejected extent or a
    /// lost render thread — shared as the `Arc`'d carrier the sink
    /// delivers. [`EngineGeneration::produce`] returns it to the caller
    /// when that caller is the participant that failed, so a requester
    /// never reports a frame it never wrote.
    fn prepare(&self, time: FrameTime) -> Result<FrameScope, Arc<HostedLayerError>>;
    /// Whether the batch produced at `time` contains this scene's latest
    /// staged state. `false` after a later mount, restage or failed
    /// prepare — the scene is then explicitly owed a later frame and must
    /// not composite a stale texture into this timestamp.
    fn produced_at(&self, time: FrameTime) -> bool;
    /// Routes a batch-level failure to this participant — a shared render
    /// failure belongs to every mounted scene, not only the requester that
    /// happened to drive `produce`. The participant forwards it to its
    /// owner; it is never stored here for a poll.
    fn note_failure(&self, failure: Arc<HostedLayerError>);
}

/// One engine generation bound to an exact [`SharedGpuContext`]
/// generation.
///
/// Holds the shared cherenkov engine, the mounted scenes' weak
/// preparation hooks, and the timestamp the last batch produced.
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
    /// The prepare outcomes the produced batch's failed participants
    /// reported — kept beside the stamp so a *later* requester at that
    /// same timestamp still answers its own `Err`, never the batch's
    /// `Ok` for a frame its scene never wrote.
    produced_outcomes: RefCell<ProducedOutcomes>,
    /// The shared render failure once it settles: every later `produce`
    /// answers the same `Arc`'d failure without rerunning the frame — a
    /// generation never recovers, only a new context generation does.
    failure: RefCell<Option<Arc<HostedLayerError>>>,
}

/// A produced batch's per-participant prepare outcomes — which failed
/// participant (weakly, like [`EngineGeneration::participants`]) rejected
/// with which `Arc`'d failure.
#[cfg(feature = "gpu_surface")]
type ProducedOutcomes = Vec<(Weak<dyn SceneParticipant>, Arc<HostedLayerError>)>;

#[cfg(feature = "gpu_surface")]
impl core::fmt::Debug for EngineGeneration {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EngineGeneration").finish_non_exhaustive()
    }
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
    /// later `produce` answers the same `Arc`'d failure without rerunning,
    /// and routed to every live participant so no requester waits forever
    /// or retries a frame another presenter already saw fail.
    fn settle_failed(&self, error: HostedLayerError) -> Arc<HostedLayerError> {
        let failure = Arc::new(error);
        *self.failure.borrow_mut() = Some(failure.clone());
        for participant in self.live() {
            participant.note_failure(failure.clone());
        }
        failure
    }

    /// The retained typed failure this generation was sealed with, if
    /// any — immutable once set; a retained old generation stays failed.
    pub fn failure(&self) -> Option<Arc<HostedLayerError>> {
        self.failure.borrow().clone()
    }

    /// Native-test seam: only a real `produce` failure seals a
    /// generation, so a trial that must capture over a foreign surface on
    /// a failed one drives the production settle directly — the retained
    /// `failure` every later `produce` reads, plus the `note_failure`
    /// routing to live participants.
    #[cfg(all(feature = "native-test", target_os = "macos"))]
    pub fn seal_failure_for_test(&self, error: HostedLayerError) -> Arc<HostedLayerError> {
        self.settle_failed(error)
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
    /// The shared [`Arc<HostedLayerError>`] — [`HostedLayerError::Render`] —
    /// when the shared frame fails: the failure is retained on the
    /// generation, routed to every live participant through `note_failure`,
    /// and answered unchanged to every later `produce` — a failed generation
    /// never retries; only a new context generation produces fresh
    /// resources. `produced` is not stamped, so no participant consumes
    /// the failed batch as rendered.
    ///
    /// The error is also the requesting `requester`'s own `prepare`
    /// failure — a scene's own rejected staged contract does not seal the
    /// batch, but the caller that failed still gets `Err`: a requester
    /// never reports `Ok` for a frame its own scene never wrote. The batch
    /// still produces for every other participant, the routed copy
    /// through the sink dedupes on the generation, and a failed
    /// participant leaves the batch so no later frame re-runs its
    /// rejected contract — it re-enters through `mount`.
    pub fn produce(
        &self,
        time: FrameTime,
        requester: &Rc<dyn SceneParticipant>,
    ) -> Result<(), Arc<HostedLayerError>> {
        if let Some(failure) = self.failure.borrow().as_ref() {
            return Err(failure.clone());
        }
        if self.produced.get() == Some(time) {
            return self
                .produced_outcomes
                .borrow()
                .iter()
                .find_map(|(weak, error)| {
                    (weak
                        .upgrade()
                        .is_some_and(|live| Rc::ptr_eq(&live, requester)))
                    .then(|| Err(error.clone()))
                })
                .unwrap_or(Ok(()));
        }
        let mut frames = Vec::new();
        let mut requester_error = None;
        let mut failed = Vec::new();
        let mut failed_outcomes = Vec::new();
        for participant in &self.live() {
            match participant.prepare(time) {
                Ok(frame) => frames.push(frame),
                Err(error) => {
                    if Rc::ptr_eq(participant, requester) {
                        requester_error = Some(error.clone());
                    }
                    failed_outcomes.push((Rc::downgrade(participant), error));
                    // A failed participant leaves the batch: its own settle
                    // is already enqueued through the sink, and re-running
                    // its rejected contract every later batch would only
                    // re-log a failure that already routed — it re-enters
                    // through `mount` on a new generation or remount.
                    failed.push(participant.clone());
                }
            }
        }
        if !failed.is_empty() {
            self.participants.borrow_mut().retain(|weak| {
                weak.upgrade()
                    .is_some_and(|live| !failed.iter().any(|gone| Rc::ptr_eq(&live, gone)))
            });
        }
        let outcome = match self.engine.render(time) {
            Ok(_) => {
                self.produced.set(Some(time));
                *self.produced_outcomes.borrow_mut() = failed_outcomes;
                requester_error.map_or(Ok(()), Err)
            }
            Err(error) => Err(self.settle_failed(HostedLayerError::Render(error))),
        };
        // The prepared scenes' frame scopes stay open across the shared
        // render, so the edits each made for this frame wake no host.
        drop(frames);
        outcome
    }
}

/// The scene engine owner installed beside the runtime (#1725).
///
/// One cherenkov engine per exact [`SharedGpuContext`] generation, shared
/// by every mounted `SceneView`. Scenes keep their own surface, texture
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
type GenerationOutcome = Result<Rc<EngineGeneration>, Arc<HostedLayerError>>;

#[cfg(feature = "gpu_surface")]
impl core::fmt::Debug for SceneEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SceneEngine").finish_non_exhaustive()
    }
}

#[cfg(feature = "gpu_surface")]
impl Default for SceneEngine {
    fn default() -> Self {
        Self::new()
    }
}

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
    /// The shared [`Arc<HostedLayerError>`] — [`HostedLayerError::Engine`] —
    /// when the engine cannot be created on this context. The failure is
    /// cached against this exact generation: every later mount under it
    /// receives the same typed failure rather than rerunning creation, and
    /// a failed generation is never a ready one. Only a new context generation
    /// attempts creation again.
    pub fn generation(
        &self,
        runtime: &GpuRuntime,
        context: &Arc<SharedGpuContext>,
    ) -> Result<Rc<EngineGeneration>, Arc<HostedLayerError>> {
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
                    produced_outcomes: RefCell::new(Vec::new()),
                    failure: RefCell::new(None),
                })
            })
            .map_err(|error| Arc::new(HostedLayerError::Engine(error)));
        *self.current.borrow_mut() = Some((key, outcome.clone()));
        outcome
    }
}

/// The environment's shared scene engine owner.
///
/// # Panics
///
/// When no owner was installed — [`prepare`] installs it beside the runtime
/// before any mount can render.
#[cfg(feature = "gpu_surface")]
#[must_use]
pub fn scene_engine(env: &Environment) -> Rc<SceneEngine> {
    env.get::<Rc<SceneEngine>>()
        .expect("scene engine is not installed in the WaterUI environment")
        .clone()
}

#[cfg(all(test, target_os = "macos", feature = "gpu_surface"))]
mod tests {
    use super::*;
    use waterui_graphics::cherenkov::{Instant, Surface};
    use waterui_graphics::cherenkov_gpu::interop::TextureTarget;

    /// A mounted scene's contract reduced to observation: whether `prepare`
    /// ran and for which timestamp, whether it is owed a later frame, and
    /// the failure a batch routed to it.
    struct Probe {
        /// A surface on the generation's engine — the frame scope
        /// `prepare` answers opens on it, as a scene's does.
        surface: Surface<Gpu>,
        prepares: Cell<u32>,
        prepared: Cell<Option<FrameTime>>,
        pending: Cell<bool>,
        /// A `prepare` failure the probe answers — a staged contract a real
        /// scene rejects.
        prepare_error: Option<Arc<HostedLayerError>>,
        failure: RefCell<Option<Arc<HostedLayerError>>>,
    }

    impl Probe {
        fn mount(generation: &Rc<EngineGeneration>) -> Rc<Self> {
            Self::mount_with(generation, None)
        }

        /// A probe whose `prepare` rejects with `error` — a staged
        /// contract the scene itself refused.
        fn mount_failing(
            generation: &Rc<EngineGeneration>,
            error: Arc<HostedLayerError>,
        ) -> Rc<Self> {
            Self::mount_with(generation, Some(error))
        }

        fn mount_with(
            generation: &Rc<EngineGeneration>,
            prepare_error: Option<Arc<HostedLayerError>>,
        ) -> Rc<Self> {
            let (target, _textures) = TextureTarget::new((1, 1));
            let surface = generation
                .engine()
                .surface(target, || {})
                .expect("the probe's surface settles");
            let probe = Rc::new(Self {
                surface,
                prepares: Cell::new(0),
                prepared: Cell::new(None),
                pending: Cell::new(false),
                prepare_error,
                failure: RefCell::new(None),
            });
            let participant: Rc<dyn SceneParticipant> = probe.clone();
            generation.mount(&participant);
            probe
        }
    }

    impl SceneParticipant for Probe {
        fn prepare(&self, time: FrameTime) -> Result<FrameScope, Arc<HostedLayerError>> {
            self.prepares.set(self.prepares.get() + 1);
            if let Some(error) = &self.prepare_error {
                return Err(error.clone());
            }
            self.prepared.set(Some(time));
            self.pending.set(false);
            Ok(self.surface.begin_frame())
        }
        fn produced_at(&self, time: FrameTime) -> bool {
            self.prepared.get() == Some(time) && !self.pending.get()
        }
        fn note_failure(&self, failure: Arc<HostedLayerError>) {
            *self.failure.borrow_mut() = Some(failure);
        }
    }

    fn gpu() -> (GpuRuntime, Arc<SharedGpuContext>) {
        let runtime = pollster::block_on(GpuRuntime::new())
            .expect("a GPU adapter is required on test hardware");
        let context = runtime.context();
        (runtime, context)
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

        let requester: Rc<dyn SceneParticipant> = first_part.clone();
        generation
            .produce(time, &requester)
            .expect("first requester runs the batch");
        let requester_b: Rc<dyn SceneParticipant> = second_part.clone();
        generation
            .produce(time, &requester_b)
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
            .produce(later, &requester_b)
            .expect("the owed frame produces at its own timestamp");
        assert!(second_part.produced_at(later));
        assert_eq!(second_part.prepares.get(), 2);
    }

    /// A later requester at an already-produced timestamp answers its
    /// own prepare outcome, not the batch's `Ok`: a participant whose
    /// contract failed in the produced batch still reads `Err` when it
    /// asks for that same frame afterward.
    #[test]
    fn a_late_requester_at_a_produced_timestamp_reads_its_own_failure() {
        let (runtime, context) = gpu();
        let engines = SceneEngine::new();
        let generation = engines
            .generation(&runtime, &context)
            .expect("the generation settles");
        let prepare_error: Arc<HostedLayerError> = Arc::new(HostedLayerError::Surface(
            waterui_graphics::cherenkov::SurfaceError::Lost,
        ));
        let failing = Probe::mount_failing(&generation, prepare_error.clone());
        let healthy = Probe::mount(&generation);
        let time = FrameTime(Instant::now());

        // The healthy requester produces the batch: the failing
        // participant's contract was rejected, routed and left behind —
        // the frame still lands for the scenes that wrote it.
        let requester: Rc<dyn SceneParticipant> = healthy.clone();
        generation
            .produce(time, &requester)
            .expect("the batch produces for the scenes that wrote it");
        assert!(healthy.produced_at(time));

        // The failed participant asking for that same produced frame
        // afterward reads its own rejected outcome, never the batch's
        // `Ok` for a frame its scene never wrote.
        let requester_b: Rc<dyn SceneParticipant> = failing;
        let Err(answered) = generation.produce(time, &requester_b) else {
            panic!("a requester whose prepare failed never reads the produced frame as Ok");
        };
        assert!(
            Arc::ptr_eq(&answered, &prepare_error),
            "the answered outcome is the requester's own prepare failure"
        );
    }
}
