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
    BackdropId, FilterId, FrameId, FrameTime, RenderError, SurfaceId, SurfaceVisibility, WakeGate,
};
use filtrate_core::{
    AnimatedTarget, AnimationTrack, CpuFilter, CpuFilterError, CpuImage, Footprint, ParamArray,
    SignalVisitor, WatchGuard, WorkingSpace,
};

use crate::RedrawCallback;

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
    /// ([`Footprint::ZERO`] unfiltered).
    pub footprint: Footprint,
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

struct Entry {
    filter: Arc<dyn Erased + Send + Sync>,
    tracks: Vec<AnimationTrack>,
    events: Receiver<(usize, AnimatedTarget)>,
    pending_events: VecDeque<(usize, AnimatedTarget)>,
    dirty: Arc<AtomicBool>,
    /// Open while a visible surface's last frame ran the entry.
    gate: Arc<WakeGate>,
    _guards: Vec<WatchGuard>,
    sequence: Option<FrameId>,
    params: Arc<[f32]>,
}

impl Entry {
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
    gate: Arc<WakeGate>,
    redraw: Option<RedrawCallback>,
    guards: &'a mut Vec<WatchGuard>,
}

impl SignalVisitor for WatcherInstaller<'_> {
    fn visit<P: filtrate_core::FilterParam + ?Sized>(&mut self, index: usize, param: &P) {
        let events = self.events.clone();
        let dirty = Arc::clone(&self.dirty);
        let gate = Arc::clone(&self.gate);
        let redraw = self.redraw.clone();
        self.guards
            .push(param.watch_animated(Box::new(move |target| {
                if events.send((index, target)).is_err() {
                    return;
                }
                dirty.store(true, Ordering::Release);
                if gate.is_open()
                    && let Some(redraw) = &redraw
                {
                    redraw.wake();
                }
            })));
    }
}

#[derive(Default)]
pub struct Registry {
    entries: FxHashMap<u64, Entry>,
    /// Backdrop groups by `(surface, group)`; `None` is an unfiltered
    /// group's registration marker.
    backdrops: FxHashMap<(u64, u64), Option<Entry>>,
    redraw: Option<RedrawCallback>,
    last_frame: Option<(FrameId, cherenkov::Instant)>,
    delta: Duration,
}

impl Registry {
    pub(super) fn new(redraw: Option<RedrawCallback>) -> Self {
        let mut registry = Self::default();
        registry.redraw = redraw;
        registry
    }

    /// Builds the [`Entry`] for `filter`: its initial parameters,
    /// animation tracks and redraw watchers.
    fn entry<F>(&self, filter: F) -> Entry
    where
        F: CpuFilter + cherenkov::RenderTransfer + Send + Sync,
    {
        let mut initial = vec![0.0; F::Params::LEN];
        filter.params().write_to(&mut initial);
        let initial: Arc<[f32]> = initial.into();
        let (event_sender, events) = channel();
        let dirty = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(WakeGate::default());
        let mut guards = Vec::with_capacity(F::Params::LEN);
        filter.visit_signals(&mut WatcherInstaller {
            events: event_sender,
            dirty: Arc::clone(&dirty),
            gate: Arc::clone(&gate),
            redraw: self.redraw.clone(),
            guards: &mut guards,
        });
        Entry {
            filter: Arc::new(filter),
            tracks: initial.iter().copied().map(AnimationTrack::new).collect(),
            events,
            pending_events: VecDeque::new(),
            dirty,
            gate,
            _guards: guards,
            sequence: None,
            params: initial,
        }
    }

    /// One entry's footprint bound over its animation magnitudes.
    fn footprint_bound(entry: &Entry) -> Footprint {
        let bounds: Vec<f32> = entry
            .tracks
            .iter()
            .map(AnimationTrack::magnitude_bound)
            .collect();
        entry.filter.footprint_bound(&bounds)
    }

    pub fn add<F>(&mut self, id: FilterId, filter: F)
    where
        F: CpuFilter + cherenkov::RenderTransfer + Send + Sync,
    {
        let entry = self.entry(filter);
        self.entries.insert(id.raw(), entry);
    }

    /// Registers an unfiltered backdrop group.
    pub fn add_backdrop_group(&mut self, surface: SurfaceId, id: BackdropId) {
        if let Some(entry) = self
            .backdrops
            .insert((surface.raw(), id.raw()), None)
            .flatten()
        {
            entry.gate.close();
        }
    }

    /// Registers a backdrop group whose capture runs `filter`.
    pub fn add_filtered_backdrop_group<F>(&mut self, surface: SurfaceId, id: BackdropId, filter: F)
    where
        F: CpuFilter + cherenkov::RenderTransfer + Send + Sync,
    {
        let entry = self.entry(filter);
        if let Some(old) = self
            .backdrops
            .insert((surface.raw(), id.raw()), Some(entry))
            .flatten()
        {
            old.gate.close();
        }
    }

    /// Unregisters a backdrop group.
    pub fn remove_backdrop_group(&mut self, surface: SurfaceId, id: BackdropId) {
        if let Some(entry) = self.backdrops.remove(&(surface.raw(), id.raw())).flatten() {
            entry.gate.close();
        }
    }

    pub fn remove(&mut self, id: FilterId) {
        if let Some(entry) = self.entries.remove(&id.raw()) {
            entry.gate.close();
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
                self.delta = time.0.saturating_duration_since(last_time);
                self.last_frame = Some((id, time.0));
            }
        } else {
            self.last_frame = Some((id, time.0));
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
        let bounds: Vec<f32> = entry
            .tracks
            .iter()
            .map(AnimationTrack::magnitude_bound)
            .collect();
        let footprint = entry.filter.footprint(&bounds, size);
        Ok((
            Arc::clone(&entry.filter),
            Arc::clone(&entry.params),
            footprint,
        ))
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
        let Some(entry) = group else {
            return Ok(PreparedBackdrop {
                filter: None,
                footprint: Footprint::ZERO,
            });
        };
        entry.prepare(sequence, self.delta);
        Ok(PreparedBackdrop {
            filter: Some((Arc::clone(&entry.filter), Arc::clone(&entry.params))),
            footprint: Self::footprint_bound(entry),
        })
    }

    /// Whether backdrop group `id` on `surface` needs another frame.
    pub(super) fn wants_redraw_group(&self, surface: SurfaceId, id: BackdropId) -> bool {
        self.backdrops
            .get(&(surface.raw(), id.raw()))
            .is_some_and(|group| group.as_ref().is_some_and(Entry::wants_redraw))
    }

    /// Sets each filter's and backdrop chain's wake gate to the surfaces
    /// whose last frames ran it; an entry no frame ran wakes nothing.
    pub(super) fn set_surfaces(
        &self,
        uses: &FxHashMap<u64, Vec<SurfaceVisibility>>,
        groups: &FxHashMap<(u64, u64), SurfaceVisibility>,
    ) {
        for (id, entry) in &self.entries {
            entry
                .gate
                .set(uses.get(id).map(Vec::as_slice).unwrap_or_default());
        }
        for (key, group) in &self.backdrops {
            if let Some(entry) = group {
                entry.gate.set(
                    groups
                        .get(key)
                        .map(std::slice::from_ref)
                        .unwrap_or_default(),
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
            if let Some(Some(entry)) = self.backdrops.get_mut(key) {
                Self::finish_entry(entry);
            }
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        for entry in self
            .entries
            .values()
            .chain(self.backdrops.values().flatten())
        {
            entry.gate.close();
        }
    }
}
