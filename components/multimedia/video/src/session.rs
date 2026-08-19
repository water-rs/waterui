//! Owned playback session and cloneable controller semantics.

use core::fmt;
use std::{cell::RefCell, rc::Rc, time::Duration};

use nami::{Computed, SignalExt as _, collection::List};
use waterui_core::{Binding, binding};

use crate::{
    AudioTrackSelection, LiveWindow, MediaItem, MediaItemId, PlaybackPolicy, SubtitleSelection,
    TrackCatalog, VideoTrackSelection, Volume,
};

/// High-level phase shared by views, controls, and platform media sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackPhase {
    /// No source has begun preparation.
    #[default]
    Idle,
    /// The current item is opening its transport, container, and decoders.
    Preparing,
    /// The current item can start playback.
    Ready,
    /// Media time is actively advancing.
    Playing,
    /// Playback is paused while retaining the current position.
    Paused,
    /// Playback is waiting for media data.
    Buffering,
    /// The current item reached its end.
    Ended,
    /// Playback stopped because of a fatal error.
    Failed,
}

/// Playlist repetition policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    /// Stop at the end of the playlist.
    #[default]
    Off,
    /// Repeat the current media item after it ends.
    One,
    /// Continue from the last item to the first item.
    All,
}

/// A non-empty reactive media playlist.
#[derive(Clone)]
pub struct Playlist {
    items: List<MediaItem>,
}

impl fmt::Debug for Playlist {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Playlist")
            .field("items", &self.items.snapshot())
            .finish()
    }
}

impl Playlist {
    /// Creates a non-empty playlist from its first item and remaining items.
    #[must_use]
    pub fn new(
        first: impl Into<MediaItem>,
        remaining: impl IntoIterator<Item = MediaItem>,
    ) -> Self {
        let mut items = vec![first.into()];
        items.extend(remaining);
        Self {
            items: List::from(items),
        }
    }

    /// Creates a one-item playlist.
    #[must_use]
    pub fn single(item: impl Into<MediaItem>) -> Self {
        Self::new(item, core::iter::empty())
    }

    /// Takes a point-in-time snapshot in presentation order.
    #[must_use]
    pub fn snapshot(&self) -> Vec<MediaItem> {
        self.items.snapshot()
    }

    /// Returns the current number of items. The result is always non-zero.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.snapshot().len()
    }

    /// Returns `false`; playlists preserve a non-empty invariant by construction.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }
}

/// Typed failures returned by player-controller commands.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PlaybackError {
    /// The requested media item is not present in the active playlist.
    #[error("media item {0:?} is not present in the active playlist")]
    ItemNotFound(MediaItemId),
    /// The requested playlist position is outside its current bounds.
    #[error("playlist index {index} is out of range for {len} items")]
    PlaylistIndexOutOfRange {
        /// Requested index.
        index: usize,
        /// Current playlist length.
        len: usize,
    },
    /// Removing the item would violate the non-empty playlist invariant.
    #[error("the final media item cannot be removed from a playback session")]
    CannotRemoveFinalItem,
    /// Navigation has no item in the requested direction.
    #[error("the playlist has no {direction} item")]
    NavigationUnavailable {
        /// `next` or `previous`.
        direction: &'static str,
    },
    /// A duration-dependent seek was requested before duration became known.
    #[error("the current media duration is not available for seeking")]
    DurationUnavailable,
    /// A live-only command was requested for finite media or before live metadata arrived.
    #[error("the current media does not expose a live seek window")]
    LiveWindowUnavailable,
    /// The requested seek target is outside the current item duration.
    #[error("seek target {requested:?} exceeds media duration {duration:?}")]
    SeekOutOfRange {
        /// Requested presentation position.
        requested: Duration,
        /// Current media duration.
        duration: Duration,
    },
    /// The requested seek target is outside the current live DVR window.
    #[error("live seek target {requested:?} is outside {start:?}..={end:?}")]
    LiveSeekOutOfRange {
        /// Requested presentation position.
        requested: Duration,
        /// Earliest seekable live position.
        start: Duration,
        /// Latest seekable live position.
        end: Duration,
    },
}

#[derive(Clone)]
pub(crate) struct PlaybackBindings {
    pub source: Binding<MediaItem>,
    pub subtitle_selection: Binding<SubtitleSelection>,
    pub audio_track_selection: Binding<AudioTrackSelection>,
    pub video_track_selection: Binding<VideoTrackSelection>,
    pub track_catalog: Binding<TrackCatalog>,
    pub live_window: Binding<Option<LiveWindow>>,
    pub has_next: Binding<bool>,
    pub has_previous: Binding<bool>,
    pub volume: Binding<Volume>,
    pub muted: Binding<bool>,
    pub playback_rate: Binding<f32>,
    pub preserve_pitch: Binding<bool>,
    pub desired_playing: Binding<bool>,
    pub seek_target_seconds: Binding<f64>,
    pub seek_generation: Binding<u64>,
    pub step_forward_generation: Binding<u64>,
    pub step_backward_generation: Binding<u64>,
    pub position_seconds: Binding<f64>,
    pub duration_seconds: Binding<f64>,
    pub phase: Binding<PlaybackPhase>,
    pub current_item_id: Binding<MediaItemId>,
    pub current_item_index: Binding<usize>,
    pub repeat: Binding<RepeatMode>,
    pub shuffle: Binding<bool>,
    pub playback_policy: PlaybackPolicy,
}

struct ControllerState {
    playlist: Playlist,
    bindings: PlaybackBindings,
}

impl ControllerState {
    fn snapshot(&self) -> Vec<MediaItem> {
        self.playlist.snapshot()
    }

    fn current_index(&self) -> usize {
        self.bindings.current_item_index.get()
    }

    fn traversal_indices(&self) -> Vec<usize> {
        let items = self.snapshot();
        let mut indices = (0..items.len()).collect::<Vec<_>>();
        if self.bindings.shuffle.get() {
            indices.sort_by_key(|index| items[*index].id.into_bytes());
        }
        indices
    }

    fn traversal_position(&self) -> usize {
        self.traversal_indices()
            .iter()
            .position(|index| *index == self.current_index())
            .expect("the active playlist item must be present in the traversal order")
    }

    fn update_navigation(&self) {
        let len = self.playlist.len();
        let index = self.traversal_position();
        let wraps = self.bindings.repeat.get() == RepeatMode::All && len > 1;
        self.bindings.has_previous.set(index > 0 || wraps);
        self.bindings.has_next.set(index + 1 < len || wraps);
    }

    fn select_index(&self, index: usize) -> Result<(), PlaybackError> {
        let items = self.snapshot();
        let item = items
            .get(index)
            .cloned()
            .ok_or(PlaybackError::PlaylistIndexOutOfRange {
                index,
                len: items.len(),
            })?;
        let duration_seconds = item
            .metadata
            .duration()
            .map_or(0.0, |duration| duration.as_secs_f64());
        self.bindings.current_item_index.set(index);
        self.bindings.current_item_id.set(item.id);
        self.bindings.source.set(item);
        self.bindings.track_catalog.set(TrackCatalog::default());
        self.bindings.live_window.set(None);
        self.bindings.position_seconds.set(0.0);
        self.bindings.duration_seconds.set(duration_seconds);
        self.bindings.seek_target_seconds.set(0.0);
        self.bindings
            .seek_generation
            .set(self.bindings.seek_generation.get().wrapping_add(1));
        self.bindings.phase.set(PlaybackPhase::Preparing);
        self.update_navigation();
        Ok(())
    }
}

/// A cloneable command and reactive-state handle for one playback session.
#[derive(Clone)]
pub struct PlayerController {
    state: Rc<RefCell<ControllerState>>,
}

impl fmt::Debug for PlayerController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.borrow();
        formatter
            .debug_struct("PlayerController")
            .field("current_item_id", &state.bindings.current_item_id.get())
            .field("current_item_index", &state.current_index())
            .field("phase", &state.bindings.phase.get())
            .finish_non_exhaustive()
    }
}

impl PlayerController {
    /// Requests playback by committing the shared playing intent.
    pub fn play(&self) {
        let state = self.state.borrow();
        state.bindings.desired_playing.set(true);
        if matches!(
            state.bindings.phase.get(),
            PlaybackPhase::Ready | PlaybackPhase::Paused | PlaybackPhase::Ended
        ) {
            state.bindings.phase.set(PlaybackPhase::Playing);
        }
    }

    /// Requests a paused presentation while retaining the current position.
    pub fn pause(&self) {
        let state = self.state.borrow();
        state.bindings.desired_playing.set(false);
        state.bindings.phase.set(PlaybackPhase::Paused);
    }

    /// Pauses playback and requests presentation of the next video frame.
    pub fn step_forward(&self) {
        let state = self.state.borrow();
        state.bindings.desired_playing.set(false);
        state.bindings.phase.set(PlaybackPhase::Paused);
        state
            .bindings
            .step_forward_generation
            .set(state.bindings.step_forward_generation.get().wrapping_add(1));
    }

    /// Pauses playback and requests presentation of the previous retained frame.
    pub fn step_backward(&self) {
        let state = self.state.borrow();
        state.bindings.desired_playing.set(false);
        state.bindings.phase.set(PlaybackPhase::Paused);
        state.bindings.step_backward_generation.set(
            state
                .bindings
                .step_backward_generation
                .get()
                .wrapping_add(1),
        );
    }

    /// Pauses and seeks to the beginning of the current item.
    pub fn stop(&self) {
        let state = self.state.borrow();
        state.bindings.desired_playing.set(false);
        let start = state
            .bindings
            .live_window
            .get()
            .map_or(Duration::ZERO, LiveWindow::seekable_start);
        state.bindings.position_seconds.set(start.as_secs_f64());
        state.bindings.seek_target_seconds.set(start.as_secs_f64());
        state
            .bindings
            .seek_generation
            .set(state.bindings.seek_generation.get().wrapping_add(1));
        state.bindings.phase.set(PlaybackPhase::Idle);
    }

    /// Seeks to an absolute presentation position.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::DurationUnavailable`] before a finite duration is known,
    /// [`PlaybackError::SeekOutOfRange`] outside finite media, or
    /// [`PlaybackError::LiveSeekOutOfRange`] outside the current DVR window.
    pub fn seek(&self, position: Duration) -> Result<(), PlaybackError> {
        let state = self.state.borrow();
        if let Some(window) = state.bindings.live_window.get() {
            if !window.contains(position) {
                return Err(PlaybackError::LiveSeekOutOfRange {
                    requested: position,
                    start: window.seekable_start(),
                    end: window.seekable_end(),
                });
            }
        } else {
            let duration = duration_from_seconds(state.bindings.duration_seconds.get())
                .ok_or(PlaybackError::DurationUnavailable)?;
            if position > duration {
                return Err(PlaybackError::SeekOutOfRange {
                    requested: position,
                    duration,
                });
            }
        }
        state
            .bindings
            .seek_target_seconds
            .set(position.as_secs_f64());
        state
            .bindings
            .seek_generation
            .set(state.bindings.seek_generation.get().wrapping_add(1));
        Ok(())
    }

    /// Seeks to the backend-recommended position near the current live edge.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::LiveWindowUnavailable`] for finite media or
    /// before a live backend has published its timeline.
    pub fn seek_to_live_edge(&self) -> Result<(), PlaybackError> {
        let target = self
            .state
            .borrow()
            .bindings
            .live_window
            .get()
            .ok_or(PlaybackError::LiveWindowUnavailable)?
            .target_position();
        self.seek(target)
    }

    /// Seeks by a signed number of seconds and clamps at the item boundaries.
    ///
    /// # Panics
    ///
    /// Panics when `seconds` is not finite.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::DurationUnavailable`] before a finite duration is known.
    pub fn seek_relative(&self, seconds: f64) -> Result<(), PlaybackError> {
        assert!(seconds.is_finite(), "relative seek seconds must be finite");
        let requested = {
            let state = self.state.borrow();
            let position = state.bindings.position_seconds.get();
            if let Some(window) = state.bindings.live_window.get() {
                (position + seconds).clamp(
                    window.seekable_start().as_secs_f64(),
                    window.seekable_end().as_secs_f64(),
                )
            } else {
                let duration = duration_from_seconds(state.bindings.duration_seconds.get())
                    .ok_or(PlaybackError::DurationUnavailable)?;
                (position + seconds).clamp(0.0, duration.as_secs_f64())
            }
        };
        self.seek(Duration::from_secs_f64(requested))
    }

    /// Selects one playlist item by stable identity.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::ItemNotFound`] when the item is not in the playlist.
    pub fn seek_to_item(&self, id: MediaItemId) -> Result<(), PlaybackError> {
        let state = self.state.borrow();
        let index = state
            .snapshot()
            .iter()
            .position(|item| item.id == id)
            .ok_or(PlaybackError::ItemNotFound(id))?;
        state.select_index(index)
    }

    /// Selects the next playlist item.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::NavigationUnavailable`] at the end of a non-repeating queue.
    pub fn next(&self) -> Result<(), PlaybackError> {
        let state = self.state.borrow();
        let traversal = state.traversal_indices();
        let len = traversal.len();
        let position = state.traversal_position();
        let next = if position + 1 < len {
            traversal[position + 1]
        } else if state.bindings.repeat.get() == RepeatMode::All && len > 1 {
            traversal[0]
        } else {
            return Err(PlaybackError::NavigationUnavailable { direction: "next" });
        };
        state.select_index(next)
    }

    /// Selects the previous playlist item.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::NavigationUnavailable`] at the start of a non-repeating queue.
    pub fn previous(&self) -> Result<(), PlaybackError> {
        let state = self.state.borrow();
        let traversal = state.traversal_indices();
        let len = traversal.len();
        let position = state.traversal_position();
        let previous = if position > 0 {
            traversal[position - 1]
        } else if state.bindings.repeat.get() == RepeatMode::All && len > 1 {
            traversal[len - 1]
        } else {
            return Err(PlaybackError::NavigationUnavailable {
                direction: "previous",
            });
        };
        state.select_index(previous)
    }

    /// Replaces the repeat policy.
    pub fn set_repeat(&self, repeat: RepeatMode) {
        let state = self.state.borrow();
        state.bindings.repeat.set(repeat);
        state.update_navigation();
    }

    /// Enables or disables a stable shuffled traversal order.
    pub fn set_shuffle(&self, enabled: bool) {
        let state = self.state.borrow();
        state.bindings.shuffle.set(enabled);
        state.update_navigation();
    }

    /// Replaces the complete non-empty playlist and selects its first item.
    ///
    /// # Panics
    ///
    /// Panics only if the [`Playlist`] non-empty invariant is violated internally.
    pub fn replace_playlist(&self, playlist: Playlist) {
        let mut state = self.state.borrow_mut();
        state.playlist = playlist;
        state
            .select_index(0)
            .expect("a Playlist always has a first media item");
    }

    /// Appends one item to the active playlist.
    pub fn add_item(&self, item: impl Into<MediaItem>) {
        let state = self.state.borrow();
        state.playlist.items.push(item.into());
        state.update_navigation();
    }

    /// Removes an item by stable identity while preserving the active item when possible.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::CannotRemoveFinalItem`] for the last item, or
    /// [`PlaybackError::ItemNotFound`] when `id` is absent.
    pub fn remove_item(&self, id: MediaItemId) -> Result<MediaItem, PlaybackError> {
        let state = self.state.borrow();
        let items = state.snapshot();
        if items.len() == 1 {
            return Err(PlaybackError::CannotRemoveFinalItem);
        }
        let removed_index = items
            .iter()
            .position(|item| item.id == id)
            .ok_or(PlaybackError::ItemNotFound(id))?;
        let current_id = state.bindings.current_item_id.get();
        let removed = state.playlist.items.remove(removed_index);
        let remaining = state.snapshot();
        let selected = remaining
            .iter()
            .position(|item| item.id == current_id)
            .unwrap_or_else(|| removed_index.min(remaining.len() - 1));
        state.select_index(selected)?;
        Ok(removed)
    }

    /// Moves an item to a new presentation-order index.
    ///
    /// # Errors
    ///
    /// Returns [`PlaybackError::ItemNotFound`] when `id` is absent, or
    /// [`PlaybackError::PlaylistIndexOutOfRange`] for an invalid destination.
    ///
    /// # Panics
    ///
    /// Panics only if the active-item identity invariant is violated internally.
    pub fn move_item(&self, id: MediaItemId, destination: usize) -> Result<(), PlaybackError> {
        let state = self.state.borrow();
        let mut items = state.snapshot();
        if destination >= items.len() {
            return Err(PlaybackError::PlaylistIndexOutOfRange {
                index: destination,
                len: items.len(),
            });
        }
        let source = items
            .iter()
            .position(|item| item.id == id)
            .ok_or(PlaybackError::ItemNotFound(id))?;
        let item = items.remove(source);
        items.insert(destination, item);
        let current_id = state.bindings.current_item_id.get();
        let _ = state.playlist.items.replace(items.clone());
        let current = items
            .iter()
            .position(|item| item.id == current_id)
            .expect("moving an item must preserve the active media identity");
        state.bindings.current_item_index.set(current);
        state.update_navigation();
        Ok(())
    }

    /// Reactive playback phase.
    #[must_use]
    pub fn phase(&self) -> Computed<PlaybackPhase> {
        self.state.borrow().bindings.phase.clone().computed()
    }

    /// Reactive current item identity.
    #[must_use]
    pub fn current_item_id(&self) -> Computed<MediaItemId> {
        self.state
            .borrow()
            .bindings
            .current_item_id
            .clone()
            .computed()
    }

    /// Reactive zero-based playlist index.
    #[must_use]
    pub fn current_item_index(&self) -> Computed<usize> {
        self.state
            .borrow()
            .bindings
            .current_item_index
            .clone()
            .computed()
    }

    /// Reactive presentation position.
    #[must_use]
    pub fn position(&self) -> Computed<Duration> {
        self.state
            .borrow()
            .bindings
            .position_seconds
            .clone()
            .map(|seconds| Duration::from_secs_f64(seconds.max(0.0)))
            .computed()
    }

    /// Reactive presentation duration. Zero means that duration is not known yet.
    #[must_use]
    pub fn duration(&self) -> Computed<Duration> {
        self.state
            .borrow()
            .bindings
            .duration_seconds
            .clone()
            .map(|seconds| Duration::from_secs_f64(seconds.max(0.0)))
            .computed()
    }

    /// Shared volume binding.
    #[must_use]
    pub fn volume(&self) -> Binding<Volume> {
        self.state.borrow().bindings.volume.clone()
    }

    /// Shared mute binding.
    #[must_use]
    pub fn muted(&self) -> Binding<bool> {
        self.state.borrow().bindings.muted.clone()
    }

    /// Shared playback-rate binding.
    #[must_use]
    pub fn playback_rate(&self) -> Binding<f32> {
        self.state.borrow().bindings.playback_rate.clone()
    }

    /// Shared pitch-preservation binding.
    #[must_use]
    pub fn preserve_pitch(&self) -> Binding<bool> {
        self.state.borrow().bindings.preserve_pitch.clone()
    }

    /// Shared audio-track selection binding.
    #[must_use]
    pub fn audio_track_selection(&self) -> Binding<AudioTrackSelection> {
        self.state.borrow().bindings.audio_track_selection.clone()
    }

    /// Shared video-quality selection binding.
    #[must_use]
    pub fn video_track_selection(&self) -> Binding<VideoTrackSelection> {
        self.state.borrow().bindings.video_track_selection.clone()
    }

    /// Reactive selectable-track catalog for the active media item.
    #[must_use]
    pub fn track_catalog(&self) -> Computed<TrackCatalog> {
        self.state
            .borrow()
            .bindings
            .track_catalog
            .clone()
            .computed()
    }

    /// Reactive live seek window, or `None` for finite media.
    #[must_use]
    pub fn live_window(&self) -> Computed<Option<LiveWindow>> {
        self.state.borrow().bindings.live_window.clone().computed()
    }

    /// Shared subtitle selection binding.
    #[must_use]
    pub fn subtitle_selection(&self) -> Binding<SubtitleSelection> {
        self.state.borrow().bindings.subtitle_selection.clone()
    }

    /// Shared repeat-mode binding.
    #[must_use]
    pub fn repeat_mode(&self) -> Binding<RepeatMode> {
        self.state.borrow().bindings.repeat.clone()
    }

    /// Shared shuffled-traversal binding.
    #[must_use]
    pub fn shuffle_enabled(&self) -> Binding<bool> {
        self.state.borrow().bindings.shuffle.clone()
    }
}

/// A single owned playback presentation. It can be mounted exactly once.
pub struct PlaybackSession {
    state: Rc<RefCell<ControllerState>>,
}

impl fmt::Debug for PlaybackSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PlaybackSession")
            .field("controller", &self.controller())
            .finish()
    }
}

impl PlaybackSession {
    /// Creates a paused playback session for a non-empty playlist.
    #[must_use]
    pub fn new(playlist: Playlist) -> Self {
        Self::with_policy(playlist, PlaybackPolicy::default())
    }

    /// Creates a paused playback session with an explicit buffering policy.
    ///
    /// # Panics
    ///
    /// Panics only if the [`Playlist`] non-empty invariant is violated internally.
    #[must_use]
    pub fn with_policy(playlist: Playlist, playback_policy: PlaybackPolicy) -> Self {
        let first = playlist
            .snapshot()
            .into_iter()
            .next()
            .expect("Playlist construction guarantees at least one media item");
        let source = binding(first.clone());
        let bindings = PlaybackBindings {
            source,
            subtitle_selection: binding(SubtitleSelection::Auto),
            audio_track_selection: binding(AudioTrackSelection::Auto),
            video_track_selection: binding(VideoTrackSelection::Auto),
            track_catalog: binding(TrackCatalog::default()),
            live_window: binding(None),
            has_next: Binding::bool(playlist.len() > 1),
            has_previous: Binding::bool(false),
            volume: binding(Volume::default()),
            muted: Binding::bool(false),
            playback_rate: binding(1.0_f32),
            preserve_pitch: binding(true),
            desired_playing: Binding::bool(false),
            seek_target_seconds: Binding::f64(0.0),
            seek_generation: binding(0_u64),
            step_forward_generation: binding(0_u64),
            step_backward_generation: binding(0_u64),
            position_seconds: Binding::f64(0.0),
            duration_seconds: Binding::f64(
                first
                    .metadata
                    .duration()
                    .map_or(0.0, |duration| duration.as_secs_f64()),
            ),
            phase: binding(PlaybackPhase::Idle),
            current_item_id: binding(first.id),
            current_item_index: binding(0_usize),
            repeat: binding(RepeatMode::Off),
            shuffle: Binding::bool(false),
            playback_policy,
        };
        Self {
            state: Rc::new(RefCell::new(ControllerState { playlist, bindings })),
        }
    }

    /// Returns a cloneable controller for this presentation.
    #[must_use]
    pub fn controller(&self) -> PlayerController {
        PlayerController {
            state: Rc::clone(&self.state),
        }
    }

    /// Requests playback as soon as the mounted realization becomes ready.
    #[must_use]
    pub fn autoplay(self) -> Self {
        self.state.borrow().bindings.desired_playing.set(true);
        self
    }

    pub(crate) fn into_parts(self) -> (PlaybackBindings, PlayerController) {
        let controller = self.controller();
        let bindings = self.state.borrow().bindings.clone();
        (bindings, controller)
    }
}

fn duration_from_seconds(seconds: f64) -> Option<Duration> {
    (seconds.is_finite() && seconds > 0.0).then(|| Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioTrackInfo;
    use crate::url::Url;
    use nami::Signal as _;

    fn item(path: &str, id: u8) -> MediaItem {
        MediaItem::from(Url::from_file_path_str(path.to_owned()))
            .id(MediaItemId::from_bytes([id; 16]))
    }

    #[test]
    fn playlist_navigation_preserves_stable_item_identity() {
        let session =
            PlaybackSession::new(Playlist::new(item("first.mp4", 1), [item("second.mp4", 2)]));
        let controller = session.controller();

        controller.next().expect("next item must be available");
        assert_eq!(
            controller.state.borrow().bindings.current_item_id.get(),
            MediaItemId::from_bytes([2; 16])
        );
        controller
            .previous()
            .expect("previous item must be available");
        assert_eq!(
            controller.state.borrow().bindings.current_item_id.get(),
            MediaItemId::from_bytes([1; 16])
        );
    }

    #[test]
    fn removing_current_item_selects_a_remaining_item() {
        let first = item("first.mp4", 1);
        let second = item("second.mp4", 2);
        let session = PlaybackSession::new(Playlist::new(first.clone(), [second.clone()]));
        let controller = session.controller();

        let removed = controller
            .remove_item(first.id)
            .expect("item must be removable");

        assert_eq!(removed.id, first.id);
        assert_eq!(
            controller.state.borrow().bindings.current_item_id.get(),
            second.id
        );
    }

    #[test]
    fn final_playlist_item_cannot_be_removed() {
        let only = item("only.mp4", 1);
        let session = PlaybackSession::new(Playlist::single(only.clone()));

        assert_eq!(
            session.controller().remove_item(only.id),
            Err(PlaybackError::CannotRemoveFinalItem)
        );
    }

    #[test]
    fn stop_is_available_before_media_duration_is_known() {
        let session = PlaybackSession::new(Playlist::single(item("only.mp4", 1))).autoplay();
        let controller = session.controller();

        controller.stop();

        let bindings = &controller.state.borrow().bindings;
        assert!(!bindings.desired_playing.get());
        assert!(bindings.seek_target_seconds.get().abs() <= f64::EPSILON);
        assert_eq!(bindings.phase.get(), PlaybackPhase::Idle);
        assert_eq!(bindings.seek_generation.get(), 1);
    }

    #[test]
    fn repeated_seek_targets_are_distinguished_by_generation() {
        let session = PlaybackSession::new(Playlist::single(item("only.mp4", 1)));
        let controller = session.controller();
        controller
            .state
            .borrow()
            .bindings
            .duration_seconds
            .set(60.0);

        controller
            .seek(Duration::from_secs(15))
            .expect("first seek must succeed");
        controller
            .seek(Duration::from_secs(15))
            .expect("repeated seek must succeed");

        let bindings = &controller.state.borrow().bindings;
        assert!((bindings.seek_target_seconds.get() - 15.0).abs() <= f64::EPSILON);
        assert_eq!(bindings.seek_generation.get(), 2);
    }

    #[test]
    fn frame_step_commands_pause_and_preserve_repeated_requests() {
        let session = PlaybackSession::new(Playlist::single(item("only.mp4", 1))).autoplay();
        let controller = session.controller();

        controller.step_forward();
        controller.step_forward();
        controller.step_backward();

        let bindings = &controller.state.borrow().bindings;
        assert!(!bindings.desired_playing.get());
        assert_eq!(bindings.phase.get(), PlaybackPhase::Paused);
        assert_eq!(bindings.step_forward_generation.get(), 2);
        assert_eq!(bindings.step_backward_generation.get(), 1);
    }

    #[test]
    fn live_seek_uses_one_atomic_window_and_recommended_target() {
        let session = PlaybackSession::new(Playlist::single(item("live.m3u8", 1)));
        let controller = session.controller();
        let window = LiveWindow::new(
            Duration::from_secs(30),
            Duration::from_secs(90),
            Duration::from_secs(92),
            Duration::from_secs(86),
        );
        controller
            .state
            .borrow()
            .bindings
            .live_window
            .set(Some(window));

        controller
            .seek_to_live_edge()
            .expect("published live target must be seekable");

        let bindings = &controller.state.borrow().bindings;
        assert!((bindings.seek_target_seconds.get() - 86.0).abs() <= f64::EPSILON);
        assert_eq!(bindings.seek_generation.get(), 1);
        assert_eq!(
            controller.seek(Duration::from_secs(20)),
            Err(PlaybackError::LiveSeekOutOfRange {
                requested: Duration::from_secs(20),
                start: Duration::from_secs(30),
                end: Duration::from_secs(90),
            })
        );
    }

    #[test]
    fn controller_exposes_shared_track_selection_bindings() {
        let session = PlaybackSession::new(Playlist::single(item("only.mp4", 1)));
        let controller = session.controller();

        controller
            .audio_track_selection()
            .set(AudioTrackSelection::Track(1));
        controller
            .video_track_selection()
            .set(VideoTrackSelection::Track(2));
        controller
            .subtitle_selection()
            .set(SubtitleSelection::Track(3));

        let bindings = &controller.state.borrow().bindings;
        assert_eq!(
            bindings.audio_track_selection.get(),
            AudioTrackSelection::Track(1)
        );
        assert_eq!(
            bindings.video_track_selection.get(),
            VideoTrackSelection::Track(2)
        );
        assert_eq!(
            bindings.subtitle_selection.get(),
            SubtitleSelection::Track(3)
        );
    }

    #[test]
    fn controller_exposes_backend_track_catalog_updates() {
        let session = PlaybackSession::new(Playlist::single(item("only.mp4", 1)));
        let controller = session.controller();
        let catalog = TrackCatalog::default().replacing_audio(vec![AudioTrackInfo::new(
            "English",
            Some(String::from("en")),
            vec![String::from("main")],
        )]);

        controller
            .state
            .borrow()
            .bindings
            .track_catalog
            .set(catalog);

        let published = controller.track_catalog().get();
        assert_eq!(published.audio().len(), 1);
        assert_eq!(published.audio()[0].label(), "English");
        assert_eq!(published.audio()[0].language(), Some("en"));
    }

    #[test]
    fn shuffle_uses_stable_identity_order_without_mutating_presentation_order() {
        let first = item("third.mp4", 3);
        let second = item("first.mp4", 1);
        let third = item("second.mp4", 2);
        let session = PlaybackSession::new(Playlist::new(
            first.clone(),
            [second.clone(), third.clone()],
        ));
        let controller = session.controller();

        controller.set_shuffle(true);
        controller
            .previous()
            .expect("the shuffled predecessor must exist");

        let state = controller.state.borrow();
        assert_eq!(state.bindings.current_item_id.get(), third.id);
        assert_eq!(state.bindings.current_item_index.get(), 2);
        assert_eq!(state.snapshot(), vec![first, second, third]);
    }
}
