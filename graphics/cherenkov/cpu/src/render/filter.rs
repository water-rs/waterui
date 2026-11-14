use rustc_hash::{FxHashMap, FxHashSet};
use std::time::Duration;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
};

use cherenkov::{
    BackdropId, CompletionWaker, FilterId, FrameId, FrameTime, RenderError, SurfaceId, SurfaceWakes,
};
use filtrate_core::{
    AnimatedTarget, AnimationTrack, CpuFilter, CpuFilterError, CpuImage, Filter, Footprint,
    ParamArray, SignalVisitor, WatchGuard, WorkingSpace, visit_lengths,
};

pub(super) trait Erased: Send + Sync {
    fn footprint(&self, params: &[f32], size: (usize, usize)) -> f32;
    /// The filter's unresolved footprint bound: the same value
    /// [`cherenkov::BackdropChain::footprint_bound`] reports, because a
    /// spatial `CpuFilter` defines `cpu_footprint` as `footprint_of` and
    /// colour chains contribute [`Footprint::ZERO`] either way.
    fn footprint_bound(&self, params: &[f32]) -> Footprint;
    fn apply(
        &self,
        params: &[f32],
        space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError>;
}

pub(super) type Prepared = (Arc<dyn Erased + Send + Sync>, Arc<[f32]>, f32);

/// A backdrop group's prepared chain state for one frame.
pub(super) struct PreparedBackdrop {
    /// The chain and its sampled parameters; `None` for an unfiltered
    /// group.
    pub filter: Option<super::lower::FrameFilter>,
    /// The chain's footprint bound over the frame's animation magnitudes
    /// ([`Footprint::ZERO`] unfiltered), in capture texels.
    pub footprint: Footprint,
    /// The group's capture spec (scale and level count).
    pub spec: cherenkov::BackdropSpec,
}

/// A registered backdrop group: its capture spec and its chain, `None`
/// for an unfiltered group.
struct Group {
    spec: cherenkov::BackdropSpec,
    chain: Option<Entry>,
}

impl<F> Erased for F
where
    F: CpuFilter + Send + Sync,
{
    #[expect(
        clippy::cast_precision_loss,
        reason = "surface dimensions are bounded to 16384 pixels"
    )]
    fn footprint(&self, params: &[f32], size: (usize, usize)) -> f32 {
        let params = F::Params::read_from(params);
        F::cpu_footprint(&params).resolve((size.0 as f32, size.1 as f32))
    }

    fn footprint_bound(&self, params: &[f32]) -> Footprint {
        F::cpu_footprint(&F::Params::read_from(params))
    }

    fn apply(
        &self,
        params: &[f32],
        space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError> {
        self.apply_cpu_image(&F::Params::read_from(params), space, image)
    }
}

/// A filter run from its stage description: its composed stages on the
/// lane executor, bounded by its kind's footprint.
#[cfg(feature = "shaders")]
struct Described {
    program: filtrate::LaneProgram,
    reach: fn(&[f32]) -> Footprint,
}

#[cfg(feature = "shaders")]
impl Erased for Described {
    #[expect(
        clippy::cast_precision_loss,
        reason = "surface dimensions are bounded to 16384 pixels"
    )]
    fn footprint(&self, params: &[f32], size: (usize, usize)) -> f32 {
        (self.reach)(params).resolve((size.0 as f32, size.1 as f32))
    }

    fn footprint_bound(&self, params: &[f32]) -> Footprint {
        (self.reach)(params)
    }

    fn apply(
        &self,
        params: &[f32],
        space: &WorkingSpace,
        image: &mut CpuImage<'_>,
    ) -> Result<(), CpuFilterError> {
        self.program.apply(params, space, image);
        Ok(())
    }
}

/// `F`'s footprint for flattened parameters.
#[cfg(feature = "shaders")]
fn reach<F: filtrate::Reach>(params: &[f32]) -> Footprint {
    F::reach_of(&F::Params::read_from(params))
}

/// A filter the raster runs from its stage description — any filtrate
/// filter, whether or not it has a CPU kernel — registered with
/// [`Engine::effect`](cherenkov::Engine::effect).
///
/// Its stages run as the GPU executor composes them, on
/// `cherenkov-shader`'s lane executor; the raster bounds each band's apron
/// by the filter's [`Reach`](filtrate::Reach) over its parameters'
/// animation magnitudes, like a filter with a CPU kernel.
#[cfg(feature = "shaders")]
pub struct CpuEffect(Box<dyn EffectSource>);

#[cfg(feature = "shaders")]
impl std::fmt::Debug for CpuEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuEffect").finish_non_exhaustive()
    }
}

#[cfg(feature = "shaders")]
impl CpuEffect {
    /// Plans and compiles `filter`'s stages for the lane executor, on the
    /// caller's thread, so a filter the raster cannot run is refused before
    /// anything is registered.
    ///
    /// # Errors
    /// [`filtrate::LaneFilterError`] when the stages do not plan, use what
    /// the lane executor does not run, or bind an auxiliary image without
    /// CPU texels.
    pub fn new<F>(filter: F) -> Result<Self, filtrate::LaneFilterError>
    where
        F: filtrate::Reach + cherenkov::RenderTransfer,
    {
        let program = filtrate::LaneProgram::new(&filter)?;
        Ok(Self(Box::new(DescribedSource { filter, program })))
    }
}

/// What a [`CpuEffect`] carries to the render thread.
#[cfg(feature = "shaders")]
trait EffectSource: cherenkov::RenderTransfer {
    fn entry(self: Box<Self>) -> Entry;
}

#[cfg(feature = "shaders")]
struct DescribedSource<F> {
    filter: F,
    program: filtrate::LaneProgram,
}

#[cfg(feature = "shaders")]
impl<F: filtrate::Reach + cherenkov::RenderTransfer> EffectSource for DescribedSource<F> {
    fn entry(self: Box<Self>) -> Entry {
        let Self { filter, program } = *self;
        let mut entry = Registry::entry(
            &filter,
            Arc::new(Described {
                program,
                reach: reach::<F>,
            }),
        );
        // The parameters' subscriptions report through the filter's own
        // signals: it lives as long as the entry.
        entry.source = Some(Box::new(filter));
        entry
    }
}

struct Entry {
    filter: Arc<dyn Erased + Send + Sync>,
    /// The flattened parameters the filter's stages declare as lengths,
    /// in points: [`Entry::pixels`] scales them into device pixels.
    lengths: Vec<usize>,
    /// The described filter the entry's kernel was compiled from, owned
    /// for the entry's life as the GPU executor owns its filter: the
    /// parameters it holds feed the entry's subscriptions.
    #[cfg(feature = "shaders")]
    source: Option<Box<dyn cherenkov::RenderTransfer>>,
    tracks: Vec<AnimationTrack>,
    events: Receiver<(usize, AnimatedTarget)>,
    pending_events: VecDeque<(usize, AnimatedTarget)>,
    dirty: Arc<AtomicBool>,
    /// The wakes of the surfaces whose last frames ran the entry.
    wakes: Arc<SurfaceWakes>,
    _guards: Vec<WatchGuard>,
    sequence: Option<FrameId>,
    params: Arc<[f32]>,
}

impl Entry {
    /// `values`, with the length parameters scaled from points into device
    /// pixels at `scale` — what every footprint and kernel reads, as the
    /// GPU executor's `pixel_params`.
    fn pixels(&self, values: &[f32], scale: f32) -> Vec<f32> {
        let mut values = values.to_vec();
        for &index in &self.lengths {
            values[index] *= scale;
        }
        values
    }

    /// The parameters' largest magnitudes over their running animations,
    /// in device pixels at `scale`.
    fn bounds(&self, scale: f32) -> Vec<f32> {
        let magnitudes: Vec<f32> = self
            .tracks
            .iter()
            .map(AnimationTrack::magnitude_bound)
            .collect();
        self.pixels(&magnitudes, scale)
    }

    /// The frame's parameters in device pixels at `scale`.
    fn pixel_params(&self, scale: f32) -> Arc<[f32]> {
        if self.lengths.is_empty() {
            Arc::clone(&self.params)
        } else {
            self.pixels(&self.params, scale).into()
        }
    }

    fn wants_redraw(&self) -> bool {
        self.dirty.load(Ordering::Acquire) || self.tracks.iter().any(AnimationTrack::is_active)
    }

    fn prepare(&mut self, sequence: FrameId, delta: Duration) {
        if self.sequence == Some(sequence) {
            return;
        }
        let events: Vec<_> = std::mem::take(&mut self.pending_events)
            .into_iter()
            .chain(self.events.try_iter())
            .collect();
        self.dirty.store(false, Ordering::Release);
        let mut changed = !events.is_empty();
        for (index, target) in events {
            self.tracks[index].set_target(target.value, target.interpolator);
        }
        for track in &mut self.tracks {
            changed |= track.is_active();
            track.advance(delta);
        }
        if changed {
            self.dirty.store(true, Ordering::Release);
        }
        self.params = self.tracks.iter().map(AnimationTrack::value).collect();
        self.sequence = Some(sequence);
    }
}

struct WatcherInstaller<'a> {
    events: Sender<(usize, AnimatedTarget)>,
    dirty: Arc<AtomicBool>,
    wakes: Arc<SurfaceWakes>,
    guards: &'a mut Vec<WatchGuard>,
}

impl SignalVisitor for WatcherInstaller<'_> {
    fn visit<P: filtrate_core::FilterParam + ?Sized>(&mut self, index: usize, param: &P) {
        let events = self.events.clone();
        let dirty = Arc::clone(&self.dirty);
        let wakes = Arc::clone(&self.wakes);
        self.guards
            .push(param.watch_animated(Box::new(move |target| {
                if events.send((index, target)).is_err() {
                    return;
                }
                dirty.swap(true, Ordering::AcqRel);
                wakes.wake();
            })));
    }
}

pub struct Registry {
    entries: FxHashMap<u64, Entry>,
    /// The display scale of the surface being lowered: device pixels per
    /// point, applied to every filter's lengths.
    length_scale: f32,
    /// Backdrop groups by `(surface, group)`.
    backdrops: FxHashMap<(u64, u64), Group>,
    /// Registered backdrop effect shaders by `BackdropShaderId::raw()`.
    #[cfg(feature = "shaders")]
    backdrop_shaders: FxHashMap<u64, Arc<super::shader::BackdropProgram>>,
    last_frame: Option<(FrameId, cherenkov::Instant)>,
    delta: Duration,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            entries: FxHashMap::default(),
            length_scale: 1.0,
            backdrops: FxHashMap::default(),
            #[cfg(feature = "shaders")]
            backdrop_shaders: FxHashMap::default(),
            last_frame: None,
            delta: Duration::ZERO,
        }
    }
}

impl Registry {
    /// Builds the [`Entry`] running `filter` through `kernel`: its initial
    /// parameters, length indices, animation tracks and redraw watchers. A
    /// parameter change wakes the host of each surface whose last frame
    /// ran the entry.
    fn entry<F: Filter>(filter: &F, kernel: Arc<dyn Erased + Send + Sync>) -> Entry {
        let mut lengths = Vec::new();
        visit_lengths(filter, |index| lengths.push(index));
        // Stages sharing one length scale it once.
        lengths.sort_unstable();
        lengths.dedup();
        let mut initial = vec![0.0; F::Params::LEN];
        filter.params().write_to(&mut initial);
        let initial: Arc<[f32]> = initial.into();
        let (event_sender, events) = channel();
        let dirty = Arc::new(AtomicBool::new(false));
        let wakes = Arc::new(SurfaceWakes::default());
        let mut guards = Vec::with_capacity(F::Params::LEN);
        filter.visit_signals(&mut WatcherInstaller {
            events: event_sender,
            dirty: Arc::clone(&dirty),
            wakes: Arc::clone(&wakes),
            guards: &mut guards,
        });
        Entry {
            filter: kernel,
            lengths,
            #[cfg(feature = "shaders")]
            source: None,
            tracks: initial.iter().copied().map(AnimationTrack::new).collect(),
            events,
            pending_events: VecDeque::new(),
            dirty,
            wakes,
            _guards: guards,
            sequence: None,
            params: initial,
        }
    }

    /// The entry running `filter`'s own CPU kernel.
    fn kernel_entry<F>(filter: F) -> Entry
    where
        F: CpuFilter + cherenkov::RenderTransfer + Send + Sync,
    {
        let filter = Arc::new(filter);
        Self::entry(&*filter, filter.clone())
    }

    /// Sets the display scale — device pixels per point — the next
    /// lowering's filters run at.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a display scale is a small factor"
    )]
    pub(super) const fn set_display_scale(&mut self, scale: f64) {
        self.length_scale = scale as f32;
    }

    pub fn add<F>(&mut self, id: FilterId, filter: F)
    where
        F: CpuFilter + cherenkov::RenderTransfer + Send + Sync,
    {
        let entry = Self::kernel_entry(filter);
        self.entries.insert(id.raw(), entry);
    }

    /// Registers a filter run from its stage description.
    #[cfg(feature = "shaders")]
    pub fn add_effect(&mut self, id: FilterId, effect: CpuEffect) {
        let entry = effect.0.entry();
        if let Some(old) = self.entries.insert(id.raw(), entry) {
            old.wakes.clear();
        }
    }

    /// Registers an unfiltered backdrop group capturing per `spec`.
    pub fn add_backdrop_group(
        &mut self,
        surface: SurfaceId,
        id: BackdropId,
        spec: cherenkov::BackdropSpec,
    ) {
        self.insert_group(surface, id, Group { spec, chain: None });
    }

    /// Registers a backdrop group capturing per `spec` whose capture runs
    /// `filter`.
    pub fn add_filtered_backdrop_group<F>(
        &mut self,
        surface: SurfaceId,
        id: BackdropId,
        filter: F,
        spec: cherenkov::BackdropSpec,
    ) where
        F: CpuFilter + cherenkov::RenderTransfer + Send + Sync,
    {
        let chain = Some(Self::kernel_entry(filter));
        self.insert_group(surface, id, Group { spec, chain });
    }

    /// Registers `group`, clearing the wakes of a chain it replaces.
    fn insert_group(&mut self, surface: SurfaceId, id: BackdropId, group: Group) {
        if let Some(old) = self
            .backdrops
            .insert((surface.raw(), id.raw()), group)
            .and_then(|old| old.chain)
        {
            old.wakes.clear();
        }
    }

    /// Unregisters a backdrop group.
    pub fn remove_backdrop_group(&mut self, surface: SurfaceId, id: BackdropId) {
        if let Some(entry) = self
            .backdrops
            .remove(&(surface.raw(), id.raw()))
            .and_then(|group| group.chain)
        {
            entry.wakes.clear();
        }
    }

    /// Registers a compiled backdrop effect shader.
    #[cfg(feature = "shaders")]
    pub fn add_backdrop_shader(
        &mut self,
        id: cherenkov::BackdropShaderId,
        program: super::shader::BackdropProgram,
    ) {
        self.backdrop_shaders.insert(id.raw(), Arc::new(program));
    }

    /// Unregisters a backdrop effect shader.
    #[cfg(feature = "shaders")]
    pub fn remove_backdrop_shader(&mut self, id: cherenkov::BackdropShaderId) {
        self.backdrop_shaders.remove(&id.raw());
    }

    /// The registered backdrop effect shader `id`.
    #[cfg(feature = "shaders")]
    pub fn backdrop_shader(
        &self,
        id: cherenkov::BackdropShaderId,
    ) -> Option<&Arc<super::shader::BackdropProgram>> {
        self.backdrop_shaders.get(&id.raw())
    }

    pub fn remove(&mut self, id: FilterId) {
        if let Some(entry) = self.entries.remove(&id.raw()) {
            entry.wakes.clear();
        }
    }

    pub(super) fn wants_redraw(&self, id: u64) -> bool {
        self.entries.get(&id).is_some_and(Entry::wants_redraw)
    }

    pub(super) fn begin_frame(&mut self, id: FrameId, time: FrameTime) -> Duration {
        if let Some((last_id, last_time)) = self.last_frame {
            if last_id == id {
                self.delta = Duration::ZERO;
            } else {
                self.delta = time.instant().saturating_duration_since(last_time);
                self.last_frame = Some((id, time.instant()));
            }
        } else {
            self.last_frame = Some((id, time.instant()));
            self.delta = Duration::ZERO;
        }
        self.delta
    }

    pub(super) fn prepare(
        &mut self,
        id: FilterId,
        sequence: FrameId,
        size: (usize, usize),
    ) -> Result<Prepared, RenderError> {
        let entry = self
            .entries
            .get_mut(&id.raw())
            .ok_or_else(|| RenderError::Render(format!("unregistered filter {}", id.raw())))?;
        entry.prepare(sequence, self.delta);
        let footprint = entry
            .filter
            .footprint(&entry.bounds(self.length_scale), size);
        Ok((
            Arc::clone(&entry.filter),
            entry.pixel_params(self.length_scale),
            footprint,
        ))
    }

    /// The layer ids that backdrop groups registered on `surface` anchor
    /// at — the layers a group's capture is taken beneath.
    pub(super) fn backdrop_anchors(
        &self,
        surface: SurfaceId,
    ) -> impl Iterator<Item = cherenkov::LayerId> + '_ {
        self.backdrops.iter().filter_map(move |(&(s, _), group)| {
            (s == surface.raw())
                .then(|| group.spec.anchor_layer())
                .flatten()
        })
    }

    /// The frame's chain state for backdrop group `id` on `surface`, plus
    /// its unresolved [`Footprint`] bound over the frame's animation
    /// magnitudes.
    pub(super) fn prepare_backdrop(
        &mut self,
        surface: SurfaceId,
        id: BackdropId,
        sequence: FrameId,
    ) -> Result<PreparedBackdrop, RenderError> {
        let Some(group) = self.backdrops.get_mut(&(surface.raw(), id.raw())) else {
            return Err(RenderError::Render(format!(
                "unregistered backdrop group {}",
                id.raw()
            )));
        };
        let spec = group.spec;
        let Some(entry) = &mut group.chain else {
            return Ok(PreparedBackdrop {
                filter: None,
                footprint: Footprint::ZERO,
                spec,
            });
        };
        entry.prepare(sequence, self.delta);
        let scale = self.length_scale;
        Ok(PreparedBackdrop {
            filter: Some((Arc::clone(&entry.filter), entry.pixel_params(scale))),
            footprint: entry.filter.footprint_bound(&entry.bounds(scale)),
            spec,
        })
    }

    /// Whether backdrop group `id` on `surface` needs another frame.
    pub(super) fn wants_redraw_group(&self, surface: SurfaceId, id: BackdropId) -> bool {
        self.backdrops
            .get(&(surface.raw(), id.raw()))
            .is_some_and(|group| group.chain.as_ref().is_some_and(Entry::wants_redraw))
    }

    /// Sets each filter's and backdrop chain's wakes to the surfaces whose
    /// last frames ran it; an entry no frame ran wakes nothing.
    pub(super) fn set_surfaces(
        &self,
        uses: &FxHashMap<u64, Vec<CompletionWaker>>,
        groups: &FxHashMap<(u64, u64), CompletionWaker>,
    ) {
        for (id, entry) in &self.entries {
            entry
                .wakes
                .set(uses.get(id).map_or_default(Vec::as_slice), &entry.dirty);
        }
        for (key, group) in &self.backdrops {
            if let Some(entry) = &group.chain {
                entry.wakes.set(
                    groups.get(key).map_or_default(std::slice::from_ref),
                    &entry.dirty,
                );
            }
        }
    }

    /// The per-frame housekeeping for every entry sampled this frame.
    fn finish_entry(entry: &mut Entry) {
        if !entry.pending_events.is_empty() {
            entry.dirty.store(true, Ordering::Release);
            return;
        }
        entry.dirty.store(false, Ordering::Release);
        if let Ok(event) = entry.events.try_recv() {
            entry.pending_events.push_back(event);
            entry.dirty.store(true, Ordering::Release);
        }
    }

    pub(super) fn finish_frame(
        &mut self,
        used: &FxHashSet<u64>,
        used_groups: &FxHashSet<(u64, u64)>,
    ) {
        for id in used {
            if let Some(entry) = self.entries.get_mut(id) {
                Self::finish_entry(entry);
            }
        }
        for key in used_groups {
            if let Some(entry) = self
                .backdrops
                .get_mut(key)
                .and_then(|group| group.chain.as_mut())
            {
                Self::finish_entry(entry);
            }
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        for entry in self.entries.values().chain(
            self.backdrops
                .values()
                .filter_map(|group| group.chain.as_ref()),
        ) {
            entry.wakes.clear();
        }
    }
}
