//! `video` / `video_player`: `NativeVideoConfig` and
//! `NativeVideoPlayerConfig` rendered through `AVPlayer` — the Apple
//! realization of the two video leaves.
//!
//! `Video` mounts a `PlayerLayerView` (a bare `AVPlayerLayer` surface) and
//! `VideoPlayer` mounts the kit's `PlayerView` (`AVPlayerView` on macOS, an
//! `AVPlayerViewController` on iOS). Both share one coordinator: it owns the
//! `AVPlayer`, mirrors the reactive playback contract into it, and writes the
//! player's observations back into the bindings — the same round-trip the
//! Swift `WuiVideoPlaybackCoordinator` ran.
//!
//! The media session is `waterkit_audio`'s `MediaSession` (now-playing info +
//! remote commands + audio focus); its command receiver is pumped on a
//! dedicated thread that hops onto the main queue before touching state.
//!
//! `Event`s go to the view's optional `BoundVideoEventHandler`; errors are
//! non-fatal unless the contract demands `fatalError` (spherical projection
//! and required hardware power paths — neither has an `AVPlayer` answer).

use alloc::rc::{Rc, Weak};
use alloc::sync::Arc;
use core::cell::RefCell;
use core::num::NonZeroU64;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use cocoa_ui::avkit::{
    ItemStatus, LoadGuard, MediaCharacteristic, PipEvent, Player, PlayerItem, PlayerLayerView,
    PlayerView, TimeControlStatus, VideoGravity, media_option_is_forced, media_option_label,
    media_option_language, media_option_roles, variant_codec_fourccs, variant_declared_bit_rate,
    variant_is_hdr, variant_presentation_size,
};
use cocoa_ui::objc2_av_foundation::{AVAssetVariant, AVMediaSelectionGroup};
use cocoa_ui::{MainThreadMarker, Retained, Size as CocoaSize, main_queue};
use dispatch2::MainThreadBound;
use waterkit_audio::{
    MediaCommand, MediaMetadata as SessionMetadata, MediaSession, PlaybackState as SessionState,
    PlaybackStatus as SessionStatus, QueueNavigationControls,
};
use waterui::reactive::{Binding, Computed, Signal};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};
use waterui_video::video::{
    AudioTrackInfo, AudioTrackSelection, BoundVideoEventHandler, ContentMode, Event,
    NativeVideoConfig, NativeVideoPlayerConfig, PlaybackMetrics, PlaybackOutputPath,
    PlaybackPowerPolicy, SubtitleSelection, SubtitleTrackInfo, SubtitleTrackOrigin, TrackCatalog,
    VideoTrackInfo, VideoTrackSelection,
};
use waterui_video::{
    Delivery, LiveWindow, MediaItem, PlaybackPhase, PlaybackPolicy, PlayerController, RepeatMode,
};

use crate::contract::{NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

#[cfg(all(target_os = "ios", feature = "video_player"))]
use cocoa_ui::uikit::HostView;

#[cfg(feature = "video_player")]
/// Registers both video leaves; each is still gated by its own feature.
pub fn install(dispatcher: &mut Dispatcher) {
    #[cfg(feature = "video")]
    dispatcher.register_native::<NativeVideoConfig>(render_video);
    #[cfg(feature = "video_player")]
    dispatcher.register_native::<NativeVideoPlayerConfig>(render_video_player);
}

/// Fallback bounds while the item's own size is unknown — the Swift
/// `sizeThatFits` answered `320×180`.
const FALLBACK_WIDTH: f32 = 320.0;
const FALLBACK_HEIGHT: f32 = 180.0;

/// The coordinator's periodic-observer cadence, matching the Swift 1/4s.
const PERIODIC_SECONDS: f64 = 0.25;

/// Reported-buffering threshold: stalls shorter than a second stay silent,
/// matching the coordinator's `systemUptime` gate.
const BUFFERING_REPORT_SECONDS: f64 = 1.0;

/// Duck volume multiplier while audio focus is lossy, per the Swift apply.
const DUCK_SCALE: f32 = 0.2;

/// Maps the content mode onto the layer gravity.
const fn gravity(mode: ContentMode) -> VideoGravity {
    match mode {
        ContentMode::Fit => VideoGravity::ResizeAspect,
        ContentMode::Fill => VideoGravity::ResizeAspectFill,
        ContentMode::Stretch => VideoGravity::Resize,
    }
}

/// The stretch axis the leaf reports, identical to `NativeView::stretch_axis`
/// on the payloads (spherical never reaches here — it panics at render).
const fn leaf_stretch_axis(mode: ContentMode) -> StretchAxis {
    match mode {
        ContentMode::Fit => StretchAxis::Horizontal,
        ContentMode::Fill | ContentMode::Stretch => StretchAxis::Both,
    }
}

/// The leaf's measure: the proposal, falling back to `320×180` per side.
fn measure(proposal: ProposalSize) -> ViewDimensions {
    ViewDimensions::new(Size::new(
        proposal.width.unwrap_or(FALLBACK_WIDTH),
        proposal.height.unwrap_or(FALLBACK_HEIGHT),
    ))
}

/// Layout face shared by both leaves.
struct VideoSubView {
    stretch: StretchAxis,
}

impl core::fmt::Debug for VideoSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VideoSubView").finish_non_exhaustive()
    }
}

impl SubView for VideoSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch
    }

    fn priority(&self) -> i32 {
        0
    }

    fn is_empty(&self) -> bool {
        false
    }
}

/// The bindings the coordinator watches and writes — lifted verbatim off
/// `PlaybackConfiguration`.
#[derive(Clone)]
struct Bindings {
    source: Computed<MediaItem>,
    subtitle_selection: Binding<SubtitleSelection>,
    audio_track_selection: Binding<AudioTrackSelection>,
    video_track_selection: Binding<VideoTrackSelection>,
    track_catalog: Binding<TrackCatalog>,
    live_window: Binding<Option<LiveWindow>>,
    has_next: Binding<bool>,
    has_previous: Binding<bool>,
    volume: Binding<waterui_video::Volume>,
    muted: Binding<bool>,
    playback_rate: Binding<f32>,
    preserve_pitch: Binding<bool>,
    desired_playing: Binding<bool>,
    seek_target_seconds: Binding<f64>,
    seek_generation: Binding<u64>,
    step_forward_generation: Binding<u64>,
    step_backward_generation: Binding<u64>,
    position_seconds: Binding<f64>,
    duration_seconds: Binding<f64>,
    phase: Binding<PlaybackPhase>,
    repeat: Binding<RepeatMode>,
    /// Kept for contract parity with the Swift coordinator; traversal order
    /// lives in `PlayerController`, so nothing here reads it.
    _shuffle: Binding<bool>,
}

impl core::fmt::Debug for Bindings {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Bindings").finish_non_exhaustive()
    }
}

/// The media-session half of the coordinator: lazily-created session, its
/// command pump, and the dedup snapshots.
#[derive(Debug)]
struct MediaSessionBridge {
    session: Option<MediaSession>,
    /// `false` once the bridge drops — the pump stops dispatching.
    pump_alive: Arc<AtomicBool>,
    last_metadata: Option<SessionMetadata>,
    last_state: Option<SessionState>,
    focus_active: bool,
}

impl MediaSessionBridge {
    fn new() -> Self {
        Self {
            session: None,
            pump_alive: Arc::new(AtomicBool::new(true)),
            last_metadata: None,
            last_state: None,
            focus_active: false,
        }
    }

    /// Abandons focus and clears now-playing before releasing the session —
    /// the Swift bridge's `deinit` branch.
    fn shutdown(&mut self) {
        self.pump_alive.store(false, Ordering::Release);
        if let Some(session) = self.session.take() {
            if self.focus_active {
                let _ = session.abandon_audio_focus();
            }
            let _ = session.clear();
        }
        self.focus_active = false;
    }
}

/// Which catalog slice an async group load landed in.
#[derive(Clone, Copy)]
enum SelectionKind {
    /// Legible (subtitle) group.
    Subtitle,
    /// Audible group.
    Audio,
    /// Video variants.
    Video,
}

/// A side effect that must run with the state cell free: a binding write,
/// an event emission, or a media-session push. Binding watchers fire
/// synchronously on `set`, so writing a binding while `state` is borrowed
/// reenters the cell — a `borrow_mut` on that stack panics. Methods
/// compute and queue these under the borrow; [`update`] applies them
/// after it is released.
enum Deferred {
    /// `bindings.phase`.
    Phase(PlaybackPhase),
    /// `bindings.position_seconds`.
    PositionSeconds(f64),
    /// `bindings.duration_seconds`.
    DurationSeconds(f64),
    /// `bindings.live_window`.
    LiveWindow(Option<LiveWindow>),
    /// `bindings.track_catalog`.
    TrackCatalog(TrackCatalog),
    /// `bindings.desired_playing`.
    DesiredPlaying(bool),
    /// `bindings.seek_target_seconds`.
    SeekTargetSeconds(f64),
    /// `bindings.seek_generation`.
    SeekGeneration(u64),
    /// `State::emit` — the app handler can write bindings, so it too must
    /// run off the borrow.
    Emit(Event),
    /// `State::push_playback_state` — reads the bindings it reports, so it
    /// must run after the queued writes above it.
    PushPlaybackState,
}

/// Runs `f` with the state mutably borrowed, then applies the side
/// effects it queued — binding writes, events, session pushes — with the
/// cell free, so watchers and event handlers never reenter a borrow.
///
/// Applying can queue more work (a watcher may drive `update` itself), so
/// the drain loops until empty.
fn update(state: &Rc<RefCell<State>>, f: impl FnOnce(&mut State)) {
    f(&mut state.borrow_mut());
    loop {
        let (deferred, bindings, handler) = {
            let mut state = state.borrow_mut();
            (
                core::mem::take(&mut state.deferred),
                state.bindings.clone(),
                state.emit.clone(),
            )
        };
        if deferred.is_empty() {
            return;
        }
        for effect in deferred {
            match effect {
                Deferred::Phase(phase) => bindings.phase.set(phase),
                Deferred::PositionSeconds(value) => bindings.position_seconds.set(value),
                Deferred::DurationSeconds(value) => bindings.duration_seconds.set(value),
                Deferred::LiveWindow(value) => bindings.live_window.set(value),
                Deferred::TrackCatalog(value) => bindings.track_catalog.set(value),
                Deferred::DesiredPlaying(value) => bindings.desired_playing.set(value),
                Deferred::SeekTargetSeconds(value) => bindings.seek_target_seconds.set(value),
                Deferred::SeekGeneration(value) => bindings.seek_generation.set(value),
                Deferred::Emit(event) => {
                    if let Some(handler) = &handler {
                        handler.call(event);
                    }
                }
                Deferred::PushPlaybackState => state.borrow_mut().push_playback_state(),
            }
        }
    }
}

/// The coordinator's whole state; touched only on the main thread.
#[allow(clippy::struct_excessive_bools)]
struct State {
    /// `Weak` self-reference for async completions — set once at build.
    weak: Weak<RefCell<Self>>,
    mtm: MainThreadMarker,
    player: Rc<Player>,
    item: Option<Rc<PlayerItem>>,
    guards: Vec<LoadGuard>,
    media: Option<MediaItem>,
    /// `(url, delivery)` of the loaded item — same-item reloads are dropped,
    /// matching the Swift `currentURL`/`currentDelivery` guard.
    source_key: Option<(String, Delivery)>,
    controller: PlayerController,
    bindings: Bindings,
    playback_policy: PlaybackPolicy,
    loops: bool,
    emit: Option<Rc<BoundVideoEventHandler>>,
    buffering: bool,
    buffering_since: Option<f64>,
    ducked: bool,
    resume_after_transient_loss: bool,
    seen_seek_generation: u64,
    seen_step_forward: u64,
    seen_step_backward: u64,
    last_buffer_level_ms: Option<u32>,
    started_at: f64,
    session: MediaSessionBridge,
    /// Whether the current item's end was already handled.
    ended: bool,
    /// Binding writes, events, and session pushes queued while this cell
    /// is borrowed; [`update`] applies them after it is released.
    deferred: Vec<Deferred>,
}

impl core::fmt::Debug for State {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("State")
            .field("media", &self.media)
            .field("source_key", &self.source_key)
            .field("loops", &self.loops)
            .field("buffering", &self.buffering)
            .field("ducked", &self.ducked)
            .finish_non_exhaustive()
    }
}

impl State {
    /// Queues an event for the view's handler — handlers can write
    /// bindings, so delivery waits for [`update`] to release the borrow.
    fn emit(&mut self, event: Event) {
        self.deferred.push(Deferred::Emit(event));
    }

    /// Emits a non-fatal error event.
    fn emit_error(&mut self, message: impl Into<String>) {
        self.emit(Event::Error {
            message: message.into(),
        });
    }

    /// The phase the queued writes leave behind — the binding value when
    /// nothing is queued.
    fn effective_phase(&self) -> PlaybackPhase {
        self.deferred
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Deferred::Phase(phase) => Some(*phase),
                _ => None,
            })
            .unwrap_or_else(|| self.bindings.phase.snapshot())
    }

    /// The desired-playing flag the queued writes leave behind.
    fn effective_desired_playing(&self) -> bool {
        self.deferred
            .iter()
            .rev()
            .find_map(|effect| match effect {
                Deferred::DesiredPlaying(desired) => Some(*desired),
                _ => None,
            })
            .unwrap_or_else(|| self.bindings.desired_playing.snapshot())
    }

    /// Queues the phase write when it differs, keeping writes minimal.
    /// The write itself lands once [`update`] releases the borrow.
    fn set_phase(&mut self, phase: PlaybackPhase) {
        if self.effective_phase() != phase {
            self.deferred.push(Deferred::Phase(phase));
        }
    }

    /// Queues a media-session playback-state push; it reads the bindings
    /// it reports, so it lands after the queued writes above it.
    fn defer_push_playback_state(&mut self) {
        self.deferred.push(Deferred::PushPlaybackState);
    }

    /// Effective output volume: the duck gate scales by `0.2`, per the Swift
    /// `applyEffectiveVolume`.
    fn effective_volume(&self) -> f32 {
        let volume = self.bindings.volume.snapshot().level();
        if self.ducked {
            volume * DUCK_SCALE
        } else {
            volume
        }
    }

    /// Pushes volume/mute/pitch to the player.
    fn apply_audio(&self) {
        self.player.set_volume(self.effective_volume());
        self.player.set_muted(self.bindings.muted.snapshot());
        if let Some(item) = &self.item {
            item.set_preserve_pitch(self.bindings.preserve_pitch.snapshot());
        }
    }

    /// Loads `media` into the shared player — the Swift `update(from:)`.
    fn load(&mut self, media: MediaItem) {
        let url = media.source.to_string();
        let key = (url.clone(), media.delivery);
        if self.source_key.as_ref() == Some(&key) {
            return;
        }
        self.source_key = Some(key);
        self.guards.clear();
        self.item = None;
        self.ended = false;
        self.buffering = false;
        self.buffering_since = None;
        self.last_buffer_level_ms = None;
        self.started_at = uptime_seconds();
        self.deferred.push(Deferred::DurationSeconds(0.0));
        self.deferred.push(Deferred::PositionSeconds(0.0));
        self.deferred.push(Deferred::LiveWindow(None));
        self.deferred
            .push(Deferred::TrackCatalog(TrackCatalog::default()));
        self.push_metadata(&media);

        if media.delivery == Delivery::Dash {
            self.media = Some(media);
            self.emit_error(
                "MPEG-DASH is not supported by the Apple AVPlayer realization; \
                 select the WaterKit self-drawn video realization",
            );
            self.set_phase(PlaybackPhase::Failed);
            self.player.set_item(None);
            return;
        }

        let Some(item) = PlayerItem::from_url_string(&url, self.mtm) else {
            self.media = Some(media);
            self.emit_error(format!("invalid media URL: {url}"));
            self.set_phase(PlaybackPhase::Failed);
            self.player.set_item(None);
            return;
        };
        let item = Rc::new(item);

        self.configure_policy(&item);
        self.attach_item_observations(&item);
        self.media = Some(media);
        self.item = Some(item);
        self.set_phase(PlaybackPhase::Preparing);
        self.player
            .set_item(self.item.as_deref().map(PlayerItem::raw));
        self.apply_audio();
        self.defer_push_playback_state();
    }

    /// The playback policy mirrored onto the item, per the Swift
    /// `configurePlaybackPolicy`.
    fn configure_policy(&self, item: &PlayerItem) {
        match self.playback_policy.power {
            PlaybackPowerPolicy::PlatformManaged => {}
            PlaybackPowerPolicy::RequireAudioOffload => {
                panic!(
                    "RequireAudioOffload cannot be satisfied by the AVPlayer realization; \
                     select the WaterKit self-drawn video realization"
                );
            }
            PlaybackPowerPolicy::RequireAudioVideoTunneling => {
                panic!(
                    "RequireAudioVideoTunneling cannot be satisfied by the AVPlayer realization; \
                     select the WaterKit self-drawn video realization"
                );
            }
        }
        if self.playback_policy.realtime {
            self.player
                .set_automatically_waits_to_minimize_stalling(false);
            item.set_can_use_network_resources_for_live_streaming_while_paused(true);
            item.set_preferred_forward_buffer_duration(0.0);
        } else {
            self.player
                .set_automatically_waits_to_minimize_stalling(true);
            item.set_can_use_network_resources_for_live_streaming_while_paused(false);
            item.set_preferred_forward_buffer_duration(
                f64::from(self.playback_policy.vod_start_buffer_ms) / 1000.0,
            );
        }
    }

    /// Hooks the item's KVO/notification surface into the coordinator — the
    /// same set the Swift coordinator observed.
    fn attach_item_observations(&self, item: &PlayerItem) {
        item.on_status_change({
            let weak = self.weak.clone();
            move || {
                if let Some(shared) = weak.upgrade() {
                    update(&shared, Self::status_did_change);
                }
            }
        });
        item.on_buffer_empty_change({
            let weak = self.weak.clone();
            move || {
                if let Some(shared) = weak.upgrade() {
                    update(&shared, Self::buffer_did_change);
                }
            }
        });
        item.on_likely_to_keep_up_change({
            let weak = self.weak.clone();
            move || {
                if let Some(shared) = weak.upgrade() {
                    update(&shared, Self::buffer_did_change);
                }
            }
        });
        item.on_seekable_ranges_change({
            let weak = self.weak.clone();
            move || {
                if let Some(shared) = weak.upgrade() {
                    update(&shared, Self::update_live_window);
                }
            }
        });
        item.on_live_offset_change({
            let weak = self.weak.clone();
            move || {
                if let Some(shared) = weak.upgrade() {
                    update(&shared, Self::update_live_window);
                }
            }
        });
        item.on_did_play_to_end({
            let weak = self.weak.clone();
            move || {
                if let Some(shared) = weak.upgrade() {
                    update(&shared, Self::item_did_end);
                }
            }
        });
    }

    /// `AVPlayerItem.status` changed: `ReadyToPlay` loads the catalog,
    /// applies selections, reports the duration, and starts if desired.
    fn status_did_change(&mut self) {
        let Some(item) = self.item.clone() else {
            return;
        };
        match item.status() {
            ItemStatus::Unknown => {}
            ItemStatus::ReadyToPlay => {
                self.emit(Event::ReadyToPlay);
                self.set_phase(PlaybackPhase::Ready);
                self.report_duration(&item);
                self.update_live_window();
                self.refresh_track_catalog(&item);
                self.apply_subtitle_selection(&item);
                self.apply_audio_track_selection(&item);
                self.apply_video_track_selection(&item);
                if self.bindings.desired_playing.snapshot() {
                    self.start_playing();
                }
                self.defer_push_playback_state();
            }
            ItemStatus::Failed => {
                let message = item
                    .error_message()
                    .unwrap_or_else(|| "AVPlayerItem failed without an error description".into());
                self.emit_error(message);
                self.set_phase(PlaybackPhase::Failed);
                self.defer_push_playback_state();
            }
        }
    }

    /// Buffering bookkeeping + events, gated on ≥1s of continuous stall.
    fn buffer_did_change(&mut self) {
        let Some(item) = &self.item else { return };
        let empty = item.is_playback_buffer_empty();
        let waiting = self.player.time_control_status() == TimeControlStatus::WaitingToPlay;
        let now_buffering = empty && waiting;
        if now_buffering && !self.buffering {
            self.buffering = true;
            self.buffering_since = Some(uptime_seconds());
            self.emit(Event::Buffering);
            self.set_phase(PlaybackPhase::Buffering);
        } else if !now_buffering && self.buffering {
            self.buffering = false;
            if let Some(since) = self.buffering_since.take()
                && uptime_seconds() - since >= BUFFERING_REPORT_SECONDS
            {
                self.emit(Event::BufferingEnded);
            }
            self.set_phase(
                if self.player.time_control_status() == TimeControlStatus::Playing {
                    PlaybackPhase::Playing
                } else {
                    PlaybackPhase::Paused
                },
            );
        }
        self.defer_push_playback_state();
    }

    /// Periodic tick: position, duration, buffered level, metrics, live window.
    fn tick(&mut self, position: f64) {
        self.deferred.push(Deferred::PositionSeconds(position));
        if let Some(item) = self.item.clone() {
            self.report_duration(&item);
            self.update_live_window();
            if let Some(level_ms) = buffer_level_ms(&item, position)
                && self.last_buffer_level_ms != Some(level_ms)
            {
                self.last_buffer_level_ms = Some(level_ms);
                self.emit(Event::BufferLevel {
                    buffered_ms: level_ms,
                });
            }
            self.emit(Event::PlaybackMetrics {
                metrics: self.metrics(&item, position),
            });
        }
        self.defer_push_playback_state();
    }

    /// The backend-independent metrics snapshot for the current source.
    fn metrics(&self, item: &PlayerItem, position: f64) -> PlaybackMetrics {
        let buffered = buffer_level_ms(item, position).unwrap_or(0);
        let mut metrics = PlaybackMetrics::new(
            Duration::from_secs_f64(position.max(0.0)),
            Duration::from_millis(u64::from(buffered)),
            Duration::from_secs_f64((uptime_seconds() - self.started_at).max(0.0)),
        );
        if let Some(log) = item.latest_access_log() {
            metrics = metrics.dropped_video_frames(log.dropped_video_frames());
            if let Some(bps) = NonZeroU64::new(log.observed_bitrate()) {
                metrics = metrics.observed_network_throughput(bps);
            }
        }
        metrics
    }

    /// Duration write-back, guarding the indefinite case (`0` while unknown).
    fn report_duration(&mut self, item: &PlayerItem) {
        if let Some(seconds) = item.duration_seconds()
            && seconds.is_finite()
            && seconds >= 0.0
        {
            self.deferred.push(Deferred::DurationSeconds(seconds));
        }
    }

    /// Live-window write-back from the seekable ranges + recommended offset.
    fn update_live_window(&mut self) {
        let window = self.item.as_deref().and_then(live_window);
        self.deferred.push(Deferred::LiveWindow(window));
    }

    /// Loads both media-selection groups and the video variants, then merges
    /// each into the catalog binding — the Swift `refreshTrackCatalog`.
    fn refresh_track_catalog(&mut self, item: &PlayerItem) {
        for (characteristic, kind) in [
            (MediaCharacteristic::Legible, SelectionKind::Subtitle),
            (MediaCharacteristic::Audible, SelectionKind::Audio),
        ] {
            let weak = self.weak.clone();
            let guard = item.load_media_selection_group(characteristic, move |group, error| {
                let Some(shared) = weak.upgrade() else { return };
                update(&shared, |state| {
                    if let Some(message) = error {
                        state.emit_error(message);
                        return;
                    }
                    let Some(group) = group else { return };
                    state.merge_catalog(kind, &group);
                });
            });
            self.guards.push(guard);
        }
        let weak = self.weak.clone();
        let guard = item.load_video_variants(move |variants, error| {
            let Some(shared) = weak.upgrade() else { return };
            update(&shared, |state| {
                if let Some(message) = error {
                    state.emit_error(message);
                    return;
                }
                state.merge_video_catalog(variants);
            });
        });
        self.guards.push(guard);
    }

    /// Audio/subtitle group → the matching `TrackCatalog` slice.
    fn merge_catalog(&mut self, kind: SelectionKind, group: &AVMediaSelectionGroup) {
        let options = media_options(group);
        let catalog = self.bindings.track_catalog.snapshot();
        let catalog = match kind {
            SelectionKind::Subtitle => catalog.replacing_subtitles(
                options
                    .iter()
                    .map(|option| {
                        SubtitleTrackInfo::new(
                            media_option_label(option),
                            media_option_language(option),
                            media_option_roles(option),
                            media_option_is_forced(option),
                            SubtitleTrackOrigin::Native,
                        )
                    })
                    .collect(),
            ),
            SelectionKind::Audio => catalog.replacing_audio(
                options
                    .iter()
                    .map(|option| {
                        AudioTrackInfo::new(
                            media_option_label(option),
                            media_option_language(option),
                            media_option_roles(option),
                        )
                    })
                    .collect(),
            ),
            SelectionKind::Video => catalog,
        };
        self.deferred.push(Deferred::TrackCatalog(catalog));
        if matches!(kind, SelectionKind::Subtitle | SelectionKind::Audio) {
            self.reapply_selection(kind);
        }
    }

    /// Video variants → the catalog's `video` slice in stable quality order.
    fn merge_video_catalog(&mut self, mut variants: Vec<Retained<AVAssetVariant>>) {
        variants.sort_by(|a, b| {
            variant_quality(a)
                .partial_cmp(&variant_quality(b))
                .unwrap_or(core::cmp::Ordering::Equal)
        });
        let catalog = self.bindings.track_catalog.snapshot().replacing_video(
            variants
                .iter()
                .enumerate()
                .map(|(index, variant)| {
                    let bitrate = variant_declared_bit_rate(variant);
                    let size = variant_presentation_size(variant);
                    let codecs = variant_codec_fourccs(variant)
                        .iter()
                        .map(|fourcc| codec_string(*fourcc))
                        .collect();
                    VideoTrackInfo::new(
                        format!("avasset-variant-{index}"),
                        video_track_label(index, size, bitrate),
                        NonZeroU64::new(f64_as_u64(bitrate)),
                        size.map(|size| (f64_as_u32(size.width), f64_as_u32(size.height))),
                        codecs,
                        variant_is_hdr(variant),
                    )
                })
                .collect(),
        );
        self.deferred.push(Deferred::TrackCatalog(catalog));
        self.reapply_selection(SelectionKind::Video);
    }

    /// Re-applies the current selection for `kind` after the catalog updates.
    fn reapply_selection(&mut self, kind: SelectionKind) {
        let Some(item) = self.item.clone() else {
            return;
        };
        match kind {
            SelectionKind::Subtitle => self.apply_subtitle_selection(&item),
            SelectionKind::Audio => self.apply_audio_track_selection(&item),
            SelectionKind::Video => self.apply_video_track_selection(&item),
        }
    }

    /// `subtitle_selection` → the legible group selection — Auto/Off/Track(i).
    fn apply_subtitle_selection(&mut self, item: &PlayerItem) {
        let selection = self.bindings.subtitle_selection.snapshot();
        let weak = self.weak.clone();
        let guard =
            item.load_media_selection_group(MediaCharacteristic::Legible, move |group, error| {
                let Some(shared) = weak.upgrade() else { return };
                update(&shared, |state| {
                    if let Some(message) = error {
                        state.emit_error(message);
                        return;
                    }
                    let (Some(group), Some(item)) = (group, state.item.clone()) else {
                        return;
                    };
                    match selection {
                        SubtitleSelection::Auto => item.select_media_option_automatically(&group),
                        SubtitleSelection::Off => item.select_media_option(None, &group),
                        SubtitleSelection::Track(index) => {
                            let options = media_options(&group);
                            let Some(option) = options.get(index) else {
                                state.emit_error(format!(
                                    "subtitle track {index} is out of range ({} options)",
                                    options.len()
                                ));
                                return;
                            };
                            item.select_media_option(Some(option), &group);
                        }
                    }
                });
            });
        self.guards.push(guard);
    }

    /// `audio_track_selection` → the audible group selection — Auto/Track(i).
    fn apply_audio_track_selection(&mut self, item: &PlayerItem) {
        let selection = self.bindings.audio_track_selection.snapshot();
        let weak = self.weak.clone();
        let guard =
            item.load_media_selection_group(MediaCharacteristic::Audible, move |group, error| {
                let Some(shared) = weak.upgrade() else { return };
                update(&shared, |state| {
                    if let Some(message) = error {
                        state.emit_error(message);
                        return;
                    }
                    let (Some(group), Some(item)) = (group, state.item.clone()) else {
                        return;
                    };
                    match selection {
                        AudioTrackSelection::Auto => item.select_media_option_automatically(&group),
                        AudioTrackSelection::Track(index) => {
                            let options = media_options(&group);
                            let Some(option) = options.get(index) else {
                                state.emit_error(format!(
                                    "audio track {index} is out of range ({} options)",
                                    options.len()
                                ));
                                return;
                            };
                            item.select_media_option(Some(option), &group);
                        }
                    }
                });
            });
        self.guards.push(guard);
    }

    /// `video_track_selection` → peak bitrate + max resolution over the
    /// quality-sorted variant list — the Swift `applyVideoTrackSelection`.
    fn apply_video_track_selection(&mut self, item: &PlayerItem) {
        match self.bindings.video_track_selection.snapshot() {
            VideoTrackSelection::Auto => {
                item.set_preferred_peak_bit_rate(0.0);
                item.set_preferred_maximum_resolution(CocoaSize::new(0.0, 0.0));
            }
            VideoTrackSelection::Track(index) => {
                let weak = self.weak.clone();
                let guard = item.load_video_variants(move |mut variants, error| {
                    let Some(shared) = weak.upgrade() else { return };
                    update(&shared, |state| {
                        if let Some(message) = error {
                            state.emit_error(message);
                            return;
                        }
                        variants.sort_by(|a, b| {
                            variant_quality(a)
                                .partial_cmp(&variant_quality(b))
                                .unwrap_or(core::cmp::Ordering::Equal)
                        });
                        let Some(variant) = variants.get(index) else {
                            state.emit_error(format!(
                                "video track {index} is out of range ({} variants)",
                                variants.len()
                            ));
                            return;
                        };
                        let Some(item) = &state.item else { return };
                        item.set_preferred_peak_bit_rate(variant_declared_bit_rate(variant));
                        item.set_preferred_maximum_resolution(
                            variant_presentation_size(variant).unwrap_or(CocoaSize::new(0.0, 0.0)),
                        );
                    });
                });
                self.guards.push(guard);
            }
        }
    }

    /// A seek request: target seconds + generation — deduped by generation,
    /// clamped into the last seekable range (or resolved duration), per Swift.
    fn apply_seek(&mut self, generation: u64) {
        if generation == self.seen_seek_generation {
            return;
        }
        self.seen_seek_generation = generation;
        let target = self.bindings.seek_target_seconds.snapshot();
        let clamped = self.clamp_seek(target);
        self.player.seek_to_seconds(clamped);
        self.deferred.push(Deferred::PositionSeconds(clamped));
        self.defer_push_playback_state();
    }

    /// The Swift clamp: last seekable range, else `[0, duration]`.
    fn clamp_seek(&self, target: f64) -> f64 {
        if let Some(item) = &self.item {
            let ranges = item.seekable_ranges();
            if let Some(last) = ranges.last() {
                return target.clamp(last.start, last.end);
            }
            if let Some(duration) = item.duration_seconds()
                && duration.is_finite()
            {
                return target.clamp(0.0, duration);
            }
        }
        target.max(0.0)
    }

    /// A frame-step request: pause + `stepByCount` when stepping is possible.
    fn apply_step(&mut self, forward: bool, generation: u64) {
        let last_seen = if forward {
            &mut self.seen_step_forward
        } else {
            &mut self.seen_step_backward
        };
        if generation == *last_seen {
            return;
        }
        *last_seen = generation;
        let Some(item) = &self.item else { return };
        let can = if forward {
            item.can_step_forward()
        } else {
            item.can_step_backward()
        };
        if !can {
            self.emit_error(if forward {
                "cannot step forward at the current position"
            } else {
                "cannot step backward at the current position"
            });
            return;
        }
        self.player.pause();
        item.step_by_count(if forward { 1 } else { -1 });
    }

    /// `desired_playing` mirrored onto the player, honoring the rate binding.
    fn apply_desired_playing(&mut self, desired: bool) {
        if desired {
            self.start_playing();
        } else {
            self.player.pause();
            if self.effective_phase() == PlaybackPhase::Playing {
                self.set_phase(PlaybackPhase::Paused);
            }
        }
        self.defer_push_playback_state();
    }

    /// Plays at the requested rate — `rate` zero falls back to `play()`.
    fn start_playing(&self) {
        let rate = self.bindings.playback_rate.snapshot();
        if rate == 0.0 {
            self.player.play();
        } else {
            self.player.set_rate(rate);
        }
    }

    /// `AVPlayerItemDidPlayToEndTime`: loop, repeat, or advance — the Swift
    /// `itemDidEnd` branch.
    fn item_did_end(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.emit(Event::Ended);
        let repeat = self.bindings.repeat.snapshot();
        if self.loops || repeat == RepeatMode::One {
            self.player.seek_to_seconds(0.0);
            self.start_playing();
            self.ended = false;
            return;
        }
        if repeat == RepeatMode::All || self.bindings.has_next.snapshot() {
            if self.controller.next().is_err() {
                self.set_phase(PlaybackPhase::Ended);
            }
            self.defer_push_playback_state();
            return;
        }
        self.set_phase(PlaybackPhase::Ended);
        self.defer_push_playback_state();
    }

    /// The media-session metadata push, dedup'd — the Swift
    /// `updateNowPlayingInfo`.
    fn push_metadata(&mut self, media: &MediaItem) {
        let metadata = media.metadata.clone();
        let title = metadata
            .title()
            .map(str::to_owned)
            .or_else(|| media_title_fallback(media.source.as_ref()));
        let mut snapshot = SessionMetadata::new();
        if let Some(title) = title {
            snapshot = snapshot.with_title(title);
        }
        if let Some(artist) = metadata.artist() {
            snapshot = snapshot.with_artist(artist);
        }
        if let Some(album) = metadata.album() {
            snapshot = snapshot.with_album(album);
        }
        if let Some(duration) = metadata.duration() {
            snapshot = snapshot.with_duration(duration);
        }
        // The kit takes encoded artwork bytes, not a URL — `artwork_url` is a
        // no-op until the session contract carries bytes.
        self.push_session_metadata(&snapshot);
    }

    /// The media-session playback snapshot, dedup'd — the Swift
    /// `updatePlaybackState`.
    fn push_playback_state(&mut self) {
        let position = Duration::from_secs_f64(self.bindings.position_seconds.snapshot().max(0.0));
        let phase = self.bindings.phase.snapshot();
        let state = match phase {
            PlaybackPhase::Idle | PlaybackPhase::Failed | PlaybackPhase::Ended => {
                SessionState::stopped()
            }
            PlaybackPhase::Paused => SessionState::paused(position),
            _ => SessionState::playing(position)
                .with_rate(f64::from(self.bindings.playback_rate.snapshot().max(0.0))),
        }
        .with_queue_navigation_controls(
            QueueNavigationControls::enabled()
                .with_next_enabled(self.bindings.has_next.snapshot())
                .with_previous_enabled(self.bindings.has_previous.snapshot()),
        );
        self.push_session_state(state);
    }

    /// Lazily opens the media session and its command pump on first use.
    fn ensure_session(&mut self) -> Option<&MediaSession> {
        if self.session.session.is_none() {
            match MediaSession::new() {
                Ok(session) => {
                    let receiver = session.command_receiver();
                    let alive = self.session.pump_alive.clone();
                    let bound = Arc::new(MainThreadBound::new(self.weak.clone(), self.mtm));
                    if std::thread::Builder::new()
                        .name("video-media-commands".into())
                        .spawn(move || pump_media_commands(receiver, bound, alive))
                        .is_err()
                    {
                        self.emit_error("media-command pump failed to start");
                        return None;
                    }
                    self.session.session = Some(session);
                }
                Err(error) => {
                    self.emit_error(format!("media session unavailable: {error}"));
                }
            }
        }
        self.session.session.as_ref()
    }

    fn push_session_metadata(&mut self, metadata: &SessionMetadata) {
        if self.session.last_metadata.as_ref() == Some(metadata) {
            return;
        }
        self.session.last_metadata = Some(metadata.clone());
        if let Some(session) = self.ensure_session() {
            let _ = session.set_metadata(metadata);
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    fn push_session_state(&mut self, state: SessionState) {
        if self.session.last_state.as_ref() == Some(&state) {
            return;
        }
        self.session.last_state = Some(state.clone());
        let stopped = state.status() == SessionStatus::Stopped;
        self.ensure_session();
        let bridge = &mut self.session;
        if let Some(session) = bridge.session.as_ref() {
            if !stopped && !bridge.focus_active {
                if session.request_audio_focus().is_ok() {
                    bridge.focus_active = true;
                }
            } else if stopped && bridge.focus_active {
                let _ = session.abandon_audio_focus();
                bridge.focus_active = false;
            }
            let _ = session.set_playback_state(&state);
        }
    }

    /// One remote-command dispatch — the Swift `handleCommand`.
    #[allow(clippy::needless_pass_by_value)]
    fn handle_media_command(&mut self, command: MediaCommand) {
        match command {
            MediaCommand::Play => self.deferred.push(Deferred::DesiredPlaying(true)),
            MediaCommand::Pause => self.deferred.push(Deferred::DesiredPlaying(false)),
            MediaCommand::PlayPause => {
                let playing = self.effective_phase() == PlaybackPhase::Playing;
                self.deferred.push(Deferred::DesiredPlaying(!playing));
            }
            MediaCommand::Stop => {
                self.ducked = false;
                self.apply_audio();
                self.controller.stop();
            }
            MediaCommand::Next => {
                if self.bindings.has_next.snapshot() {
                    let _ = self.controller.next();
                    self.emit(Event::NextRequested);
                }
            }
            MediaCommand::Previous => {
                if self.bindings.has_previous.snapshot() {
                    let _ = self.controller.previous();
                    self.emit(Event::PreviousRequested);
                }
            }
            MediaCommand::Seek(position) => {
                self.deferred
                    .push(Deferred::SeekTargetSeconds(position.as_secs_f64()));
                let next = self.bindings.seek_generation.snapshot().wrapping_add(1);
                self.deferred.push(Deferred::SeekGeneration(next));
            }
            MediaCommand::SeekForward(delta) => self.seek_relative(delta.as_secs_f64()),
            MediaCommand::SeekBackward(delta) => self.seek_relative(-delta.as_secs_f64()),
            MediaCommand::AudioFocusGained => {
                self.ducked = false;
                self.apply_audio();
                if self.resume_after_transient_loss {
                    self.resume_after_transient_loss = false;
                    self.deferred.push(Deferred::DesiredPlaying(true));
                }
            }
            MediaCommand::AudioFocusLost => {
                self.ducked = false;
                self.apply_audio();
                self.deferred.push(Deferred::DesiredPlaying(false));
                self.resume_after_transient_loss = false;
            }
            MediaCommand::AudioFocusLostTransient => {
                if self.effective_phase() == PlaybackPhase::Playing {
                    self.resume_after_transient_loss = true;
                }
                self.deferred.push(Deferred::DesiredPlaying(false));
            }
            MediaCommand::AudioFocusLostDuck => {
                self.ducked = true;
                self.apply_audio();
            }
            MediaCommand::AudioBecomingNoisy => {
                self.deferred.push(Deferred::DesiredPlaying(false));
            }
            _ => {}
        }
        self.apply_desired_playing(self.effective_desired_playing());
    }

    /// Position ± delta, committed through the seek bindings like the Swift
    /// `SeekForward`/`SeekBackward` branches did.
    fn seek_relative(&mut self, delta: f64) {
        let position = self.bindings.position_seconds.snapshot();
        self.deferred
            .push(Deferred::SeekTargetSeconds((position + delta).max(0.0)));
        let next = self.bindings.seek_generation.snapshot().wrapping_add(1);
        self.deferred.push(Deferred::SeekGeneration(next));
    }

    /// Full teardown — the Swift coordinator's `deinit`: player released,
    /// session cleared, pump stopped.
    fn teardown(&mut self) {
        self.player.pause();
        self.player.set_item(None);
        self.item = None;
        self.guards.clear();
        self.session.shutdown();
    }
}

/// Command-pump loop: blocks on the kit receiver, hops onto the main queue,
/// exits when the channel closes or the bridge is gone.
#[allow(clippy::needless_pass_by_value)]
fn pump_media_commands(
    receiver: async_channel::Receiver<MediaCommand>,
    bound: Arc<MainThreadBound<Weak<RefCell<State>>>>,
    alive: Arc<AtomicBool>,
) {
    while alive.load(Ordering::Acquire) {
        let Ok(command) = receiver.recv_blocking() else {
            break;
        };
        let bound = bound.clone();
        main_queue::enqueue(move |mtm| {
            if let Some(shared) = bound.get(mtm).upgrade() {
                update(&shared, |state| state.handle_media_command(command));
            }
        });
    }
}

/// The group's options as a `Vec` — the load completion is the only caller.
fn media_options(
    group: &AVMediaSelectionGroup,
) -> Vec<Retained<cocoa_ui::objc2_av_foundation::AVMediaSelectionOption>> {
    // SAFETY: `options` reads a live group; `to_vec` retains each element.
    unsafe { group.options() }.to_vec()
}

/// The Swift metadata fallback: URL `lastPathComponent`, else the string.
fn media_title_fallback(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let tail = url.rsplit('/').find(|part| !part.is_empty());
    Some(tail.unwrap_or(url).to_owned())
}

/// Milliseconds of buffered media ahead of `position` — the Swift
/// `loadedTimeRanges` read.
fn buffer_level_ms(item: &PlayerItem, position: f64) -> Option<u32> {
    item.loaded_ranges().iter().find_map(|range| {
        (position >= range.start && position < range.end)
            .then(|| f64_as_u32((range.end - position) * 1000.0))
    })
}

/// `LiveWindow` from the item's seekable ranges and recommended offset; `None`
/// for finite media — the Swift `updateLiveWindow`.
fn live_window(item: &PlayerItem) -> Option<LiveWindow> {
    let finite = item.duration_seconds().is_some_and(f64::is_finite);
    let ranges = item.seekable_ranges();
    let (Some(first), Some(last)) = (ranges.first(), ranges.last()) else {
        return None;
    };
    let (start, end) = (first.start, last.end);
    if finite || !start.is_finite() || !end.is_finite() || end < start {
        return None;
    }
    let offset = item.recommended_time_offset_from_live().unwrap_or(0.0);
    let target = (end - offset).clamp(start, end);
    Some(LiveWindow::new(
        Duration::from_secs_f64(start.max(0.0)),
        Duration::from_secs_f64(end.max(0.0)),
        Duration::from_secs_f64(end.max(0.0)),
        Duration::from_secs_f64(target.max(0.0)),
    ))
}

/// f64 → u64 for catalog bitrates/resolutions; clamps negatives and NaN.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn f64_as_u64(value: f64) -> u64 {
    if value.is_finite() && value > 0.0 {
        value as u64
    } else {
        0
    }
}

/// f64 → u32 for catalog resolutions; clamps negatives and NaN.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn f64_as_u32(value: f64) -> u32 {
    if value.is_finite() && value > 0.0 {
        value as u32
    } else {
        0
    }
}

/// Sort key for a variant: declared bitrate first, then pixel count — the
/// Swift catalog ordering.
fn variant_quality(variant: &AVAssetVariant) -> f64 {
    let bitrate = variant_declared_bit_rate(variant);
    if bitrate > 0.0 {
        bitrate
    } else if let Some(size) = variant_presentation_size(variant) {
        size.width * size.height
    } else {
        0.0
    }
}

/// `VideoTrackInfo` label: `"WxH · X.Y Mbps"` when both are known, else
/// `Variant N` — the Swift label builder.
fn video_track_label(index: usize, size: Option<CocoaSize>, bitrate: f64) -> String {
    match (size, bitrate) {
        (Some(size), bits) if bits > 0.0 => {
            format!(
                "{}x{} · {:.1} Mbps",
                f64_as_u32(size.width),
                f64_as_u32(size.height),
                bits / 1_000_000.0
            )
        }
        _ => format!("Variant {}", index + 1),
    }
}

/// 4-char code name for a `CMVideoCodecType` fourcc.
fn codec_string(fourcc: u32) -> String {
    let bytes = fourcc.to_be_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Uptime in seconds — `systemUptime`'s equivalent for the buffering gate.
fn uptime_seconds() -> f64 {
    cocoa_ui::process::time_since_start().map_or(0.0, |duration| duration.as_secs_f64())
}

/// The coordinator the leaf keeps alive; dropping it tears everything down.
struct Coordinator {
    state: Rc<RefCell<State>>,
}

impl core::fmt::Debug for Coordinator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Coordinator").finish_non_exhaustive()
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        update(&self.state, State::teardown);
    }
}

/// Builds the coordinator: shared player, player-level observations, media
/// session. `loops` is the leaf's own loop flag — `VideoPlayer` hardcodes it
/// `false`, matching the Swift leaf.
fn coordinator(
    mtm: MainThreadMarker,
    playback: waterui_video::video::PlaybackConfiguration<Option<BoundVideoEventHandler>>,
    loops: bool,
) -> (Coordinator, Rc<Player>) {
    let player = Rc::new(Player::new(mtm));
    let state = Rc::new(RefCell::new(State {
        weak: Weak::new(),
        mtm,
        player: player.clone(),
        item: None,
        guards: Vec::new(),
        media: None,
        source_key: None,
        controller: playback.controller.clone(),
        bindings: Bindings {
            source: playback.source.clone(),
            subtitle_selection: playback.subtitle_selection.clone(),
            audio_track_selection: playback.audio_track_selection.clone(),
            video_track_selection: playback.video_track_selection.clone(),
            track_catalog: playback.track_catalog.clone(),
            live_window: playback.live_window.clone(),
            has_next: playback.has_next.clone(),
            has_previous: playback.has_previous.clone(),
            volume: playback.volume.clone(),
            muted: playback.muted.clone(),
            playback_rate: playback.playback_rate.clone(),
            preserve_pitch: playback.preserve_pitch.clone(),
            desired_playing: playback.desired_playing.clone(),
            seek_target_seconds: playback.seek_target_seconds.clone(),
            seek_generation: playback.seek_generation.clone(),
            step_forward_generation: playback.step_forward_generation.clone(),
            step_backward_generation: playback.step_backward_generation.clone(),
            position_seconds: playback.position_seconds.clone(),
            duration_seconds: playback.duration_seconds.clone(),
            phase: playback.phase.clone(),
            repeat: playback.repeat.clone(),
            _shuffle: playback.shuffle.clone(),
        },
        playback_policy: playback.playback_policy,
        loops,
        emit: playback.on_event.map(Rc::new),
        deferred: Vec::new(),
        buffering: false,
        buffering_since: None,
        ducked: false,
        resume_after_transient_loss: false,
        seen_seek_generation: 0,
        seen_step_forward: 0,
        seen_step_backward: 0,
        last_buffer_level_ms: None,
        started_at: uptime_seconds(),
        session: MediaSessionBridge::new(),
        ended: false,
    }));
    state.borrow_mut().weak = Rc::downgrade(&state);

    // Player-level observations: time-control status drives the phase, the
    // external-playback flag and the periodic tick write bindings + events.
    player.on_time_control_status({
        let state = Rc::downgrade(&state);
        move || {
            if let Some(state) = state.upgrade() {
                update(&state, |state| {
                    match state.player.time_control_status() {
                        TimeControlStatus::Playing => state.set_phase(PlaybackPhase::Playing),
                        TimeControlStatus::Paused => {
                            if !state.buffering {
                                state.set_phase(PlaybackPhase::Paused);
                            }
                        }
                        TimeControlStatus::WaitingToPlay => state.buffer_did_change(),
                    }
                    state.defer_push_playback_state();
                });
            }
        }
    });
    player.on_external_playback_change({
        let state = Rc::downgrade(&state);
        move || {
            if let Some(state) = state.upgrade() {
                update(&state, |state| {
                    state.emit(Event::ExternalPlaybackChanged {
                        active: state.player.is_external_playback_active(),
                    });
                });
            }
        }
    });
    player.observe_periodic(PERIODIC_SECONDS, {
        let state = Rc::downgrade(&state);
        move |position| {
            if let Some(state) = state.upgrade() {
                update(&state, |state| state.tick(position));
            }
        }
    });

    (Coordinator { state }, player)
}

/// Wires the bindings onto the leaf. The configuration bindings (source,
/// desired state, audio, seek/step generations) are `bind`ed so their
/// current values apply at mount — the same init-time application the
/// Swift coordinator performed — plus every later change. The remaining
/// watchers only report backend-originated state, so they subscribe for
/// changes alone.
#[allow(clippy::too_many_lines)]
fn bind(leaf: &mut NativeLeaf, coordinator: &Coordinator) {
    let state = &coordinator.state;
    // Clone the bindings out before wiring: `leaf.bind` applies the current
    // value synchronously, and a `Ref` borrowed only for the field access
    // would live until the end of the whole `bind` call — running `update`
    // inside it while the cell is still borrowed.
    let bindings = state.borrow().bindings.clone();

    leaf.bind(&bindings.source, {
        let state = Rc::downgrade(state);
        move |media| {
            if let Some(state) = state.upgrade() {
                update(&state, |state| state.load(media));
            }
        }
    });
    leaf.bind(&bindings.desired_playing, {
        let state = Rc::downgrade(state);
        move |desired| {
            if let Some(state) = state.upgrade() {
                update(&state, |state| {
                    state.apply_desired_playing(desired);
                });
            }
        }
    });
    leaf.bind(&bindings.volume, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                state.borrow().apply_audio();
            }
        }
    });
    leaf.bind(&bindings.muted, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                state.borrow().apply_audio();
            }
        }
    });
    leaf.bind(&bindings.playback_rate, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                let state = state.borrow();
                if state.bindings.desired_playing.snapshot() {
                    state.start_playing();
                }
            }
        }
    });
    leaf.bind(&bindings.preserve_pitch, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                state.borrow().apply_audio();
            }
        }
    });
    leaf.bind(&bindings.seek_generation, {
        let state = Rc::downgrade(state);
        move |generation| {
            if let Some(state) = state.upgrade() {
                update(&state, |state| state.apply_seek(generation));
            }
        }
    });
    leaf.bind(&bindings.step_forward_generation, {
        let state = Rc::downgrade(state);
        move |generation| {
            if let Some(state) = state.upgrade() {
                update(&state, |state| state.apply_step(true, generation));
            }
        }
    });
    leaf.bind(&bindings.step_backward_generation, {
        let state = Rc::downgrade(state);
        move |generation| {
            if let Some(state) = state.upgrade() {
                update(&state, |state| state.apply_step(false, generation));
            }
        }
    });
    leaf.watch(&bindings.subtitle_selection, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade()
                && let Some(item) = state.borrow().item.clone()
            {
                update(&state, |state| state.apply_subtitle_selection(&item));
            }
        }
    });
    leaf.watch(&bindings.audio_track_selection, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade()
                && let Some(item) = state.borrow().item.clone()
            {
                update(&state, |state| state.apply_audio_track_selection(&item));
            }
        }
    });
    leaf.watch(&bindings.video_track_selection, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade()
                && let Some(item) = state.borrow().item.clone()
            {
                update(&state, |state| state.apply_video_track_selection(&item));
            }
        }
    });
    leaf.watch(&bindings.has_next, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                state.borrow_mut().push_playback_state();
            }
        }
    });
    leaf.watch(&bindings.has_previous, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                state.borrow_mut().push_playback_state();
            }
        }
    });
    leaf.watch(&bindings.phase, {
        let state = Rc::downgrade(state);
        move |_| {
            if let Some(state) = state.upgrade() {
                state.borrow_mut().push_playback_state();
            }
        }
    });
}

/// `Native<NativeVideoConfig>` — the raw surface.
#[cfg(feature = "video")]
fn render_video(config: NativeVideoConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    assert!(
        !config.projection.is_spherical(),
        "spherical video projection is not supported by the AVPlayer realization; \
         select the WaterKit self-drawn video realization"
    );
    let mtm = ctx.mtm();
    let surface = PlayerLayerView::new(mtm, gravity(config.content_mode));
    let (coordinator, player) = coordinator(mtm, config.playback, config.loops);
    surface.set_player(Some(&player));
    update(&coordinator.state, |state| {
        state.emit(Event::PlaybackOutputPathChanged {
            path: PlaybackOutputPath::PlatformManaged,
        });
    });

    let mut leaf = NativeLeaf::new(
        surface.view(),
        VideoSubView {
            stretch: leaf_stretch_axis(config.content_mode),
        },
    );
    bind(&mut leaf, &coordinator);
    leaf.keep(surface);
    leaf.keep(coordinator);
    leaf
}

/// `Native<NativeVideoPlayerConfig>` — the controls surface.
#[cfg(feature = "video_player")]
fn render_video_player(config: NativeVideoPlayerConfig, ctx: &RenderContext<'_>) -> NativeLeaf {
    assert!(
        !config.projection.is_spherical(),
        "spherical video projection is not supported by the AVPlayer realization; \
         select the WaterKit self-drawn video realization"
    );
    let mtm = ctx.mtm();
    let view = Rc::new(PlayerView::new(mtm, gravity(config.content_mode)));
    view.set_shows_controls(config.show_controls);
    view.set_allows_picture_in_picture(true);
    let (coordinator, player) = coordinator(mtm, config.playback, false);
    view.set_player(Some(&player));
    update(&coordinator.state, |state| {
        state.emit(Event::PlaybackOutputPathChanged {
            path: PlaybackOutputPath::PlatformManaged,
        });
    });
    view.set_pip_handler({
        let state = Rc::downgrade(&coordinator.state);
        move |event| {
            if let Some(state) = state.upgrade() {
                let active = matches!(event, PipEvent::Started);
                update(&state, |state| {
                    state.emit(Event::PictureInPictureChanged { active });
                });
            }
        }
    });

    let mut leaf = build_player_leaf(&view, config.content_mode, mtm);
    bind(&mut leaf, &coordinator);
    leaf.keep(coordinator);
    leaf.keep(view);
    leaf
}

/// Host view + `PlayerView` inside it; on iOS the controller attaches to the
/// parent view controller when the host joins a window and detaches when it
/// leaves — the same lifecycle the Swift `WuiVideoPlayer` ran.
#[cfg(feature = "video_player")]
fn build_player_leaf(
    view: &Rc<PlayerView>,
    mode: ContentMode,
    mtm: MainThreadMarker,
) -> NativeLeaf {
    #[cfg(target_os = "ios")]
    {
        let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let child = view.view();
        let parent: &cocoa_ui::PlatformView = AsRef::as_ref(&*host);
        // A `UIViewController`'s view is created at screen size and follows
        // no parent; pin it to the host's bounds so it stays inside the
        // leaf's layout frame instead of covering the window.
        child.setFrame(parent.bounds());
        cocoa_ui::view::set_autoresizing_flexible_size(&child);
        cocoa_ui::view::add_subview(parent, &child);
        host.set_window_handler({
            let view = Rc::clone(view);
            move |host| {
                let base: &cocoa_ui::PlatformView = AsRef::as_ref(host);
                if cocoa_ui::view::window(base).is_some() {
                    view.attach_to_parent_controller();
                } else {
                    view.detach_from_parent_controller();
                }
            }
        });
        let mut leaf = NativeLeaf::new(
            &*host,
            VideoSubView {
                stretch: leaf_stretch_axis(mode),
            },
        );
        leaf.keep(host);
        leaf
    }
    #[cfg(target_os = "macos")]
    {
        let _ = mtm;
        NativeLeaf::new(
            view.view(),
            VideoSubView {
                stretch: leaf_stretch_axis(mode),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gravity_maps_content_mode() {
        assert!(matches!(
            gravity(ContentMode::Fit),
            VideoGravity::ResizeAspect
        ));
        assert!(matches!(
            gravity(ContentMode::Fill),
            VideoGravity::ResizeAspectFill
        ));
        assert!(matches!(
            gravity(ContentMode::Stretch),
            VideoGravity::Resize
        ));
    }

    #[test]
    fn stretch_axis_matches_contract() {
        assert_eq!(leaf_stretch_axis(ContentMode::Fit), StretchAxis::Horizontal);
        assert_eq!(leaf_stretch_axis(ContentMode::Fill), StretchAxis::Both);
        assert_eq!(leaf_stretch_axis(ContentMode::Stretch), StretchAxis::Both);
    }

    #[test]
    fn measure_falls_back_to_320x180() {
        let empty = measure(ProposalSize::new(None, None));
        assert!(f32::abs(empty.size.width - FALLBACK_WIDTH) < f32::EPSILON);
        assert!(f32::abs(empty.size.height - FALLBACK_HEIGHT) < f32::EPSILON);
    }

    #[test]
    fn measure_uses_proposal() {
        let proposal = ProposalSize::new(Some(640.0), Some(360.0));
        let measured = measure(proposal);
        assert!(f32::abs(measured.size.width - 640.0) < f32::EPSILON);
        assert!(f32::abs(measured.size.height - 360.0) < f32::EPSILON);
    }

    #[test]
    fn title_fallback_uses_last_path_component() {
        assert_eq!(
            media_title_fallback("https://example.com/movie.mp4").as_deref(),
            Some("movie.mp4")
        );
        assert_eq!(
            media_title_fallback("https://example.com/a/b/clip.mov").as_deref(),
            Some("clip.mov")
        );
    }

    #[test]
    fn title_fallback_uses_url_when_no_slash() {
        assert_eq!(
            media_title_fallback("movie.mp4").as_deref(),
            Some("movie.mp4")
        );
    }

    #[test]
    fn title_fallback_none_for_empty() {
        assert!(media_title_fallback("").is_none());
    }

    #[test]
    fn codec_string_decodes_fourcc() {
        assert_eq!(codec_string(u32::from_be_bytes(*b"avc1")), "avc1");
    }
}
