//! `AVPlayer`/`AVKit` playback bridging.
//!
//! The kit keeps this layer imperative: observations and asynchronous loads
//! are plain Rust closures, and the types here carry no `WaterUI` or reactive
//! state. Callers wire those closures to whatever signal layer they use.
//!
//! [`Player`] wraps `AVPlayer` with typed playback control and observation.
//! [`PlayerItem`] wraps `AVPlayerItem` with status, buffering, timeline and
//! media-selection support. [`PlayerLayerView`] is the raw video surface —
//! an `AVPlayerLayer` managed inside a [`HostView`] — and [`PlayerView`] is
//! the controls surface: `AVPlayerView` on macOS, `AVPlayerViewController`
//! (declared here because `objc2-av-kit` is a macOS-only framework crate) on
//! iOS.
//!
//! # Safety
//!
//! Every `unsafe` call in this module sends a message to a live Objective-C
//! object the crate retains, from the main thread, with the signature the
//! generated `objc2-av-foundation`/`objc2-av-kit` bindings declare. KVO and
//! notification callbacks run inside [`guarded`]. `AVFoundation` may invoke
//! completion blocks and — for notifications observed through this module —
//! deliver them on threads other than the main one; every such handler hops
//! to the main dispatch queue (or wraps its payload in
//! [`MainThreadBound`]) before touching caller state, which callers keep in
//! main-thread (`!Send`) closures.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use dispatch2::{DispatchQueue, MainThreadBound};
use objc2::runtime::{AnyObject, Bool, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained};
use objc2_av_foundation::{
    AVAssetVariant, AVAsynchronousKeyValueLoading, AVAudioTimePitchAlgorithmSpectral,
    AVAudioTimePitchAlgorithmVarispeed, AVKeyValueStatus, AVLayerVideoGravity,
    AVLayerVideoGravityResize, AVLayerVideoGravityResizeAspect,
    AVLayerVideoGravityResizeAspectFill, AVMediaCharacteristic, AVMediaCharacteristicAudible,
    AVMediaCharacteristicContainsOnlyForcedSubtitles,
    AVMediaCharacteristicDescribesMusicAndSoundForAccessibility,
    AVMediaCharacteristicDescribesVideoForAccessibility, AVMediaCharacteristicIsAuxiliaryContent,
    AVMediaCharacteristicLegible, AVMediaCharacteristicTranscribesSpokenDialogForAccessibility,
    AVMediaSelectionGroup, AVMediaSelectionOption, AVPlayer, AVPlayerActionAtItemEnd, AVPlayerItem,
    AVPlayerItemAccessLog, AVPlayerItemAccessLogEvent, AVPlayerItemDidPlayToEndTimeNotification,
    AVPlayerItemStatus, AVPlayerLayer, AVPlayerTimeControlStatus, AVURLAsset, AVVideoRangeSDR,
    NSValueAVFoundationExtensions,
};
use objc2_core_foundation::CGSize;
use objc2_core_media::{CMTime, CMTimeRange};
use objc2_foundation::{
    NSArray, NSError, NSKeyValueObservingOptions, NSObject, NSObjectNSKeyValueObserverRegistration,
    NSString, NSValue,
};

use crate::callback::guarded;
use crate::geometry::{Rect, Size};
use crate::main_queue;
use crate::notification::{NotificationName, NotificationObserver};

#[cfg(target_os = "macos")]
use crate::appkit::HostView;
#[cfg(target_os = "ios")]
use crate::uikit::HostView;

/// How the decoded picture fills the bounds it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VideoGravity {
    /// Fit the video within the bounds, preserving aspect (letterbox).
    #[default]
    ResizeAspect,
    /// Fill the bounds, preserving aspect (may crop).
    ResizeAspectFill,
    /// Stretch to the bounds exactly, ignoring aspect.
    Resize,
}

fn video_gravity(gravity: VideoGravity) -> &'static AVLayerVideoGravity {
    // SAFETY: these are framework-provided string constants that exist on
    // every supported Apple platform.
    unsafe {
        match gravity {
            VideoGravity::ResizeAspect => AVLayerVideoGravityResizeAspect,
            VideoGravity::ResizeAspectFill => AVLayerVideoGravityResizeAspectFill,
            VideoGravity::Resize => AVLayerVideoGravityResize,
        }
        .expect("AVLayerVideoGravity constants exist on all supported Apple platforms")
    }
}

/// `AVPlayerItemStatus`, as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    /// The item's readiness is not yet known.
    Unknown,
    /// The item can play.
    ReadyToPlay,
    /// The item can never play. Read the error with [`PlayerItem::error_message`].
    Failed,
}

/// `AVPlayerTimeControlStatus`, as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeControlStatus {
    /// Playback is paused.
    Paused,
    /// The player is waiting for data or for a scheduled start.
    WaitingToPlay,
    /// Media time is advancing.
    Playing,
}

/// A media characteristic usable with
/// [`PlayerItem::load_media_selection_group`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaCharacteristic {
    /// Subtitles and other legible tracks.
    Legible,
    /// Audio tracks.
    Audible,
}

fn media_characteristic(characteristic: MediaCharacteristic) -> &'static AVMediaCharacteristic {
    // SAFETY: framework-provided constants present on all supported platforms.
    unsafe {
        match characteristic {
            MediaCharacteristic::Legible => AVMediaCharacteristicLegible,
            MediaCharacteristic::Audible => AVMediaCharacteristicAudible,
        }
        .expect("AVMediaCharacteristic constants exist on all supported Apple platforms")
    }
}

/// One seekable or buffered time interval, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeRange {
    /// Where the interval starts.
    pub start: f64,
    /// Where the interval ends.
    pub end: f64,
}

impl TimeRange {
    fn from_value(value: &NSValue) -> Option<Self> {
        // SAFETY: `seekableTimeRanges`/`loadedTimeRanges` only contain
        // `NSValue` boxes of `CMTimeRange`, so unwrapping as that type is what
        // the framework wrote in.
        let range: CMTimeRange = unsafe { value.CMTimeRangeValue() };
        let start = cm_time_seconds(range.start);
        // SAFETY: `range` is a live `CMTimeRange` from the box above.
        let end = cm_time_seconds(unsafe { range.end() });
        if start.is_finite() && end.is_finite() && end >= start {
            Some(Self { start, end })
        } else {
            None
        }
    }
}

fn cm_time_seconds(time: CMTime) -> f64 {
    // SAFETY: `time` is a `CMTime` value the caller already holds.
    unsafe { time.seconds() }
}

fn cm_time(seconds: f64) -> CMTime {
    // SAFETY: callers clamp `seconds` to finite values before calling.
    unsafe { CMTime::with_seconds(seconds, 600) }
}

fn error_message(error: &NSError) -> String {
    error.localizedDescription().to_string()
}

/// A resource load `AVFoundation` performs off the main thread; dropping it
/// cancels the completion (the closure is never called afterwards).
#[derive(Debug)]
#[must_use = "dropping the guard cancels the load"]
pub struct LoadGuard {
    cancelled: Arc<AtomicBool>,
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// Packages a main-thread `handler` and a cancellation flag so an
/// `AVFoundation` completion invoked on an arbitrary thread delivers its
/// result on the main queue, once.
struct Completion<F> {
    handler: MainThreadBound<RefCell<Option<F>>>,
    cancelled: Arc<AtomicBool>,
}

impl<F> fmt::Debug for Completion<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Completion").finish_non_exhaustive()
    }
}

impl<F> Completion<F> {
    /// Runs the handler with the payload `payload` builds — built here, on
    /// the main thread — unless the completion was cancelled.
    fn finish<P>(&self, mtm: MainThreadMarker, payload: impl FnOnce() -> P)
    where
        F: FnOnce(P),
    {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        if let Some(handler) = self.handler.get(mtm).borrow_mut().take() {
            handler(payload());
        }
    }
}

/// Builds the [`Completion`] slot a callback block captures.
fn completion_slot<F>(mtm: MainThreadMarker, handler: F) -> Arc<Completion<F>> {
    Arc::new(Completion {
        handler: MainThreadBound::new(RefCell::new(Some(handler)), mtm),
        cancelled: Arc::new(AtomicBool::new(false)),
    })
}

/// Like [`completion_slot`], but also returns the [`LoadGuard`] a caller
/// drops to cancel the delivery.
fn guarded_completion<F>(mtm: MainThreadMarker, handler: F) -> (Arc<Completion<F>>, LoadGuard) {
    let completion = completion_slot(mtm, handler);
    let guard = LoadGuard {
        cancelled: completion.cancelled.clone(),
    };
    (completion, guard)
}

// ── KVO ──────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct ObserverIvars {
    handler: RefCell<Option<Rc<dyn Fn()>>>,
}

impl fmt::Debug for ObserverIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObserverIvars").finish_non_exhaustive()
    }
}

define_class!(
    // SAFETY: `NSObject` subclasses are created with `init`; the class
    // overrides `observeValueForKeyPath:ofObject:change:context:` with the
    // signature the runtime contract declares, and everything it touches is
    // main-thread state guarded by `guarded`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiPathObserver"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ObserverIvars]
    #[derive(Debug)]
    /// An `NSObject` KVO observer that forwards every change to a Rust
    /// closure.
    struct PathObserver;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for PathObserver {}

    impl PathObserver {
        // SAFETY: the override signature matches the one `NSObject` declares.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&objc2_foundation::NSDictionary<NSString, AnyObject>>,
            _context: *mut core::ffi::c_void,
        ) {
            guarded("PathObserver KVO", || {
                if let Some(handler) = self.ivars().handler.borrow().as_ref() {
                    handler();
                }
            });
        }
    }
);

impl PathObserver {
    fn new(mtm: MainThreadMarker, handler: impl Fn() + 'static) -> Retained<Self> {
        // SAFETY: `init` is the constructor `NSObject` subclasses use.
        let observer: Retained<Self> = unsafe { msg_send![Self::alloc(mtm), init] };
        observer.ivars().handler.replace(Some(Rc::new(handler)));
        observer
    }
}

/// Keeps a key-path observation registered; dropping it unregisters.
///
/// The handler is called on the thread `AVFoundation` performs the KVO
/// callback on — which is wherever the property setter ran. Callers that
/// need the main thread re-dispatch from inside their handler; the
/// [`Player`]/[`PlayerItem`] observation setters already do that hop.
#[derive(Debug)]
#[must_use = "the observation ends as soon as this guard is dropped"]
pub struct PathObservation {
    object: Retained<NSObject>,
    observer: Retained<PathObserver>,
    key_path: Retained<NSString>,
}

impl Drop for PathObservation {
    fn drop(&mut self) {
        // SAFETY: `observer` was registered on `object` for `key_path` in
        // `observe_path`; this is the matching removal.
        unsafe {
            self.object.removeObserver_forKeyPath_context(
                &self.observer,
                &self.key_path,
                core::ptr::null_mut(),
            );
        }
    }
}

/// Registers `handler` for `key_path` changes on `object`
/// (`NSKeyValueObservingOptionNew`).
///
/// # Panics
///
/// If `object` does not report a value for `key_path` — the registration
/// itself throws in that case, same as KVO always has.
fn observe_path(
    object: &NSObject,
    key_path: &'static str,
    mtm: MainThreadMarker,
    handler: impl Fn() + 'static,
) -> PathObservation {
    let observer = PathObserver::new(mtm, handler);
    let key_path = NSString::from_str(key_path);
    // SAFETY: `observer` is a live NSObject subclass instance; the context is
    // null because the observer's override ignores it.
    unsafe {
        object.addObserver_forKeyPath_options_context(
            &observer,
            &key_path,
            NSKeyValueObservingOptions::New,
            core::ptr::null_mut(),
        );
    }
    PathObservation {
        object: Retained::from(object),
        observer,
        key_path,
    }
}

// ── Player ───────────────────────────────────────────────────────────────────

/// A retained `AVPlayer` with typed observation hooks.
///
/// All methods must be called on the main thread. Observation handlers are
/// re-dispatched to the main dispatch queue before running, mirroring the
/// `DispatchQueue.main.async` the previous implementation used.
#[derive(Debug)]
pub struct Player {
    inner: Retained<AVPlayer>,
    mtm: MainThreadMarker,
    observations: RefCell<Vec<PathObservation>>,
    time_observer: RefCell<Option<Retained<AnyObject>>>,
}

impl Player {
    /// A fresh player with nothing loaded.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        // SAFETY: `AVPlayer::new` is a message send creating a live object.
        let player = unsafe { AVPlayer::new(mtm) };
        Self {
            inner: player,
            mtm,
            observations: RefCell::new(Vec::new()),
            time_observer: RefCell::new(None),
        }
    }

    /// The wrapped `AVPlayer`, for calls this wrapper does not cover.
    #[must_use]
    pub fn raw(&self) -> &AVPlayer {
        &self.inner
    }

    /// Replaces the currently playing item.
    pub fn set_item(&self, item: Option<&AVPlayerItem>) {
        // SAFETY: `item` is a live player item; `self.inner` is retained.
        unsafe { self.inner.replaceCurrentItemWithPlayerItem(item) };
    }

    /// The item currently loaded, if any.
    #[must_use]
    pub fn current_item(&self) -> Option<Retained<AVPlayerItem>> {
        // SAFETY: property read on a live object on the main thread.
        unsafe { self.inner.currentItem() }
    }

    /// Begins playback.
    pub fn play(&self) {
        // SAFETY: playback control on a live player.
        unsafe { self.inner.play() };
    }

    /// Pauses playback.
    pub fn pause(&self) {
        // SAFETY: playback control on a live player.
        unsafe { self.inner.pause() };
    }

    /// The current playback rate.
    #[must_use]
    pub fn rate(&self) -> f32 {
        // SAFETY: property read on a live player.
        unsafe { self.inner.rate() }
    }

    /// Sets the playback rate (only takes effect while playing).
    pub fn set_rate(&self, rate: f32) {
        // SAFETY: playback control on a live player.
        unsafe { self.inner.setRate(rate) };
    }

    /// Sets the player volume (0...1).
    pub fn set_volume(&self, volume: f32) {
        // SAFETY: playback control on a live player.
        unsafe { self.inner.setVolume(volume) };
    }

    /// Mutes or unmutes the player.
    pub fn set_muted(&self, muted: bool) {
        // SAFETY: playback control on a live player.
        unsafe { self.inner.setMuted(muted) };
    }

    /// Whether the player waits for data before resuming playback.
    pub fn set_automatically_waits_to_minimize_stalling(&self, waits: bool) {
        // SAFETY: property write on a live player.
        unsafe { self.inner.setAutomaticallyWaitsToMinimizeStalling(waits) };
    }

    /// Whether playback may route to an external screen.
    pub fn set_allows_external_playback(&self, allows: bool) {
        // SAFETY: property write on a live player.
        unsafe { self.inner.setAllowsExternalPlayback(allows) };
    }

    /// What happens when the current item finishes.
    pub fn set_action_at_item_end(&self, action: ActionAtItemEnd) {
        let action = match action {
            ActionAtItemEnd::Advance => AVPlayerActionAtItemEnd::Advance,
            ActionAtItemEnd::Pause => AVPlayerActionAtItemEnd::Pause,
            ActionAtItemEnd::None => AVPlayerActionAtItemEnd::None,
        };
        // SAFETY: property write on a live player.
        unsafe { self.inner.setActionAtItemEnd(action) };
    }

    /// The current `timeControlStatus`.
    #[must_use]
    pub fn time_control_status(&self) -> TimeControlStatus {
        // SAFETY: property read on a live player.
        let status = unsafe { self.inner.timeControlStatus() };
        if status == AVPlayerTimeControlStatus::Playing {
            TimeControlStatus::Playing
        } else if status == AVPlayerTimeControlStatus::Paused {
            TimeControlStatus::Paused
        } else {
            TimeControlStatus::WaitingToPlay
        }
    }

    /// Whether playback is currently on an external screen.
    #[must_use]
    pub fn is_external_playback_active(&self) -> bool {
        // SAFETY: property read on a live player.
        unsafe { self.inner.isExternalPlaybackActive() }
    }

    /// The playhead position in seconds.
    #[must_use]
    pub fn current_time_seconds(&self) -> f64 {
        // SAFETY: property read on a live player.
        cm_time_seconds(unsafe { self.inner.currentTime() })
    }

    /// Seeks to `seconds` from the item's start.
    pub fn seek_to_seconds(&self, seconds: f64) {
        // SAFETY: `cm_time` produces a valid CMTime for a finite value.
        unsafe { self.inner.seekToTime(cm_time(seconds)) };
    }

    /// Like [`Player::seek_to_seconds`], calling `completion` on the main
    /// thread with `finished` once the seek completes or is interrupted.
    ///
    /// `AVFoundation` may invoke the completion on a background queue; the
    /// hop to the main queue is done by this wrapper so `completion` does not
    /// need to be `Send`.
    pub fn seek_to_seconds_with_completion(
        &self,
        seconds: f64,
        completion: impl FnOnce(bool) + 'static,
    ) {
        let slot = completion_slot(self.mtm, completion);
        let block = RcBlock::new(move |finished: Bool| {
            let slot = slot.clone();
            main_queue::enqueue(move |mtm| {
                slot.finish(mtm, move || finished.as_bool());
            });
        });
        // SAFETY: `block` is retained by the player until the seek completes;
        // it only touches the Send-safe `slot` and the main queue.
        unsafe {
            self.inner
                .seekToTime_completionHandler(cm_time(seconds), &block);
        };
    }

    /// Calls `handler` (on the main queue) when `timeControlStatus` changes.
    ///
    /// Registering fires the handler once immediately, matching the
    /// `.initial` option the previous implementation used.
    pub fn on_time_control_status(&self, handler: impl Fn() + 'static) {
        self.observations.borrow_mut().push(observe_on_main(
            self.mtm,
            &self.inner,
            "timeControlStatus",
            true,
            handler,
        ));
    }

    /// Calls `handler` (on the main queue) when `isExternalPlaybackActive`
    /// changes, once immediately then on each change.
    pub fn on_external_playback_change(&self, handler: impl Fn() + 'static) {
        self.observations.borrow_mut().push(observe_on_main(
            self.mtm,
            &self.inner,
            "isExternalPlaybackActive",
            true,
            handler,
        ));
    }

    /// Registers `handler` on the main queue every `interval_seconds` of
    /// playback time; the argument is the playhead seconds the callback
    /// reports.
    ///
    /// # Panics
    ///
    /// If the periodic observer callback fires off the main queue (the queue
    /// it is registered on).
    pub fn observe_periodic(&self, interval_seconds: f64, handler: impl Fn(f64) + 'static) {
        let mtm = self.mtm;
        let handler = MainThreadBound::new(handler, mtm);
        let block = RcBlock::new(move |time: CMTime| {
            let seconds = cm_time_seconds(time);
            let handler = &handler;
            guarded("periodic time observer", || {
                let mtm =
                    MainThreadMarker::new().expect("the periodic observer runs on the main queue");
                handler.get(mtm)(seconds);
            });
        });
        // SAFETY: the interval is a valid CMTime; the block is invoked on
        // `DispatchQueue::main()`, which is the main thread the bound
        // handler requires.
        let observer = unsafe {
            self.inner
                .addPeriodicTimeObserverForInterval_queue_usingBlock(
                    cm_time(interval_seconds),
                    Some(DispatchQueue::main()),
                    &block,
                )
        };
        self.time_observer.replace(Some(observer));
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        if let Some(observer) = self.time_observer.borrow_mut().take() {
            // SAFETY: `observer` is the token `addPeriodicTimeObserver` gave
            // this player; removing a live token is a no-op-safe message.
            unsafe { self.inner.removeTimeObserver(&observer) };
        }
    }
}

/// `AVPlayerActionAtItemEnd`, as a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionAtItemEnd {
    /// Move to the next item in a queue player.
    Advance,
    /// Pause at the end.
    Pause,
    /// Do nothing.
    None,
}

/// KVO on `object` whose callback re-dispatches onto the main queue before
/// running `handler`; with `initial` the handler is queued once at
/// registration too, matching KVO's `.initial` option.
fn observe_on_main(
    mtm: MainThreadMarker,
    object: &NSObject,
    key_path: &'static str,
    initial: bool,
    handler: impl Fn() + 'static,
) -> PathObservation {
    let handler: Rc<dyn Fn()> = Rc::new(handler);
    let bound = Arc::new(MainThreadBound::new(handler, mtm));
    let bound_for_kvo = bound.clone();
    let observation = observe_path(object, key_path, mtm, move || {
        let bound = bound_for_kvo.clone();
        main_queue::enqueue(move |mtm| (bound.get(mtm))());
    });
    if initial {
        main_queue::enqueue(move |mtm| (bound.get(mtm))());
    }
    observation
}

// ── Player item ──────────────────────────────────────────────────────────────

/// A retained `AVPlayerItem` plus the observations tied to its lifetime.
#[derive(Debug)]
pub struct PlayerItem {
    item: Retained<AVPlayerItem>,
    mtm: MainThreadMarker,
    observations: RefCell<Vec<PathObservation>>,
    did_end: RefCell<Option<NotificationObserver>>,
}

impl PlayerItem {
    /// Builds an item for `url`; `None` when the URL is not parseable.
    #[must_use]
    pub fn from_url_string(url: &str, mtm: MainThreadMarker) -> Option<Self> {
        let string = NSString::from_str(url);
        // `NSURL::URLWithString` returns `None` for malformed input.
        let url = objc2_foundation::NSURL::URLWithString(&string)?;
        // SAFETY: the URL is live; construction is a single message send.
        let item = unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) };
        Some(Self {
            item,
            mtm,
            observations: RefCell::new(Vec::new()),
            did_end: RefCell::new(None),
        })
    }

    /// The wrapped `AVPlayerItem`.
    #[must_use]
    pub fn raw(&self) -> &AVPlayerItem {
        &self.item
    }

    /// Whether this item is still `player`'s current item.
    #[must_use]
    pub fn is_current(&self, player: &Player) -> bool {
        player.current_item().is_some_and(|item| item == self.item)
    }

    /// The item's current status.
    #[must_use]
    pub fn status(&self) -> ItemStatus {
        // SAFETY: property read on a live item.
        match unsafe { self.item.status() } {
            status if status == AVPlayerItemStatus::ReadyToPlay => ItemStatus::ReadyToPlay,
            status if status == AVPlayerItemStatus::Failed => ItemStatus::Failed,
            _ => ItemStatus::Unknown,
        }
    }

    /// The error that put the item in [`ItemStatus::Failed`], if any.
    #[must_use]
    pub fn error_message(&self) -> Option<String> {
        // SAFETY: property read on a live item.
        unsafe { self.item.error() }.as_deref().map(error_message)
    }

    /// The duration in seconds; `None` while still unknown or indefinite
    /// (live streams report `NaN`/indefinite).
    #[must_use]
    pub fn duration_seconds(&self) -> Option<f64> {
        // SAFETY: property read on a live item.
        let seconds = cm_time_seconds(unsafe { self.item.duration() });
        (seconds.is_finite() && seconds > 0.0).then_some(seconds)
    }

    /// The `presentationSize`, for callers that want the native aspect.
    #[must_use]
    pub fn presentation_size(&self) -> Size {
        // SAFETY: property read on a live item.
        let size = unsafe { self.item.presentationSize() };
        Size::new(size.width, size.height)
    }

    /// The ranges the playhead may be moved into.
    #[must_use]
    pub fn seekable_ranges(&self) -> Vec<TimeRange> {
        // SAFETY: property read on a live item.
        unsafe { self.item.seekableTimeRanges() }
            .iter()
            .filter_map(|value| TimeRange::from_value(&value))
            .collect()
    }

    /// The ranges already buffered.
    #[must_use]
    pub fn loaded_ranges(&self) -> Vec<TimeRange> {
        // SAFETY: property read on a live item.
        unsafe { self.item.loadedTimeRanges() }
            .iter()
            .filter_map(|value| TimeRange::from_value(&value))
            .collect()
    }

    /// Whether the buffer at the playhead is empty.
    #[must_use]
    pub fn is_playback_buffer_empty(&self) -> bool {
        // SAFETY: property read on a live item.
        unsafe { self.item.isPlaybackBufferEmpty() }
    }

    /// Whether the buffered data supports uninterrupted playback.
    #[must_use]
    pub fn is_playback_likely_to_keep_up(&self) -> bool {
        // SAFETY: property read on a live item.
        unsafe { self.item.isPlaybackLikelyToKeepUp() }
    }

    /// The recommended offset behind the live edge; `None` if not finite.
    #[must_use]
    pub fn recommended_time_offset_from_live(&self) -> Option<f64> {
        // SAFETY: property read on a live item.
        let offset = cm_time_seconds(unsafe { self.item.recommendedTimeOffsetFromLive() });
        (offset.is_finite() && offset >= 0.0).then_some(offset)
    }

    /// Whether stepping a frame later is supported.
    #[must_use]
    pub fn can_step_forward(&self) -> bool {
        // SAFETY: property read on a live item.
        unsafe { self.item.canStepForward() }
    }

    /// Whether stepping a frame earlier is supported.
    #[must_use]
    pub fn can_step_backward(&self) -> bool {
        // SAFETY: property read on a live item.
        unsafe { self.item.canStepBackward() }
    }

    /// Steps the item's current frame by `count` (negative goes backwards).
    pub fn step_by_count(&self, count: isize) {
        // SAFETY: playback control on a live item; `count` fits NSInteger on
        // every supported target.
        unsafe { self.item.stepByCount(count as _) };
    }

    /// Whether network resources may be used while paused (live).
    pub fn set_can_use_network_resources_for_live_streaming_while_paused(&self, allowed: bool) {
        // SAFETY: property write on a live item.
        unsafe {
            self.item
                .setCanUseNetworkResourcesForLiveStreamingWhilePaused(allowed);
        };
    }

    /// Seconds of media the player buffers ahead.
    pub fn set_preferred_forward_buffer_duration(&self, seconds: f64) {
        // SAFETY: property write on a live item.
        unsafe { self.item.setPreferredForwardBufferDuration(seconds) };
    }

    /// A hard cap on the peak bit rate the variant selection may pick
    /// (`0` removes the cap).
    pub fn set_preferred_peak_bit_rate(&self, bits_per_second: f64) {
        // SAFETY: property write on a live item.
        unsafe { self.item.setPreferredPeakBitRate(bits_per_second) };
    }

    /// A cap on the resolution the variant selection may pick
    /// (`Size::ZERO` removes the cap).
    pub fn set_preferred_maximum_resolution(&self, size: Size) {
        // SAFETY: property write on a live item.
        unsafe {
            self.item
                .setPreferredMaximumResolution(CGSize::new(size.width, size.height));
        };
    }

    /// Selects a pitch algorithm: `.spectral` preserves pitch at any rate,
    /// `.varispeed` lets pitch vary with rate.
    ///
    /// # Panics
    ///
    /// If the pitch-algorithm framework constants are missing (they are not).
    pub fn set_preserve_pitch(&self, preserve: bool) {
        // SAFETY: the pitch-algorithm constants exist on every supported
        // platform.
        let algorithm = unsafe {
            if preserve {
                AVAudioTimePitchAlgorithmSpectral
            } else {
                AVAudioTimePitchAlgorithmVarispeed
            }
            .expect("AVAudioTimePitchAlgorithm constants exist on all supported platforms")
        };
        // SAFETY: property write on a live item.
        unsafe { self.item.setAudioTimePitchAlgorithm(algorithm) };
    }

    /// Selects `option` inside `group`; `None` turns the group off.
    pub fn select_media_option(
        &self,
        option: Option<&AVMediaSelectionOption>,
        group: &AVMediaSelectionGroup,
    ) {
        // SAFETY: both objects are live and the group belongs to this item's
        // asset's media-selection universe.
        unsafe {
            self.item
                .selectMediaOption_inMediaSelectionGroup(option, group);
        };
    }

    /// Lets the player pick the appropriate option for `group`.
    pub fn select_media_option_automatically(&self, group: &AVMediaSelectionGroup) {
        // SAFETY: property write on a live item.
        unsafe {
            self.item
                .selectMediaOptionAutomaticallyInMediaSelectionGroup(group);
        };
    }

    /// The most recent access-log event, if any.
    #[must_use]
    pub fn latest_access_log(&self) -> Option<AccessLogEvent> {
        // SAFETY: `accessLog()` and its events are live objects.
        let log: Retained<AVPlayerItemAccessLog> = unsafe { self.item.accessLog() }?;
        // SAFETY: property read on a live access log.
        let events = unsafe { log.events() };
        events.lastObject().map(AccessLogEvent)
    }

    /// Calls `handler` (on the main queue) when `status` changes.
    pub fn on_status_change(&self, handler: impl Fn() + 'static) {
        self.register("status", false, handler);
    }

    /// Calls `handler` (on the main queue) when `isPlaybackBufferEmpty`
    /// changes.
    pub fn on_buffer_empty_change(&self, handler: impl Fn() + 'static) {
        self.register("isPlaybackBufferEmpty", false, handler);
    }

    /// Calls `handler` (on the main queue) when `isPlaybackLikelyToKeepUp`
    /// changes.
    pub fn on_likely_to_keep_up_change(&self, handler: impl Fn() + 'static) {
        self.register("isPlaybackLikelyToKeepUp", false, handler);
    }

    /// Calls `handler` (on the main queue) when `seekableTimeRanges` changes,
    /// once immediately then on each change.
    pub fn on_seekable_ranges_change(&self, handler: impl Fn() + 'static) {
        self.register("seekableTimeRanges", true, handler);
    }

    /// Calls `handler` (on the main queue) when `recommendedTimeOffsetFromLive`
    /// changes, once immediately then on each change.
    pub fn on_live_offset_change(&self, handler: impl Fn() + 'static) {
        self.register("recommendedTimeOffsetFromLive", true, handler);
    }

    /// Calls `handler` on the main queue when the item reaches its end.
    pub fn on_did_play_to_end(&self, handler: impl Fn() + 'static) {
        // SAFETY: `AVPlayerItemDidPlayToEndTimeNotification` is a framework
        // string constant; `observe_object` filters delivery to this item.
        let name = unsafe { AVPlayerItemDidPlayToEndTimeNotification };
        let observer = crate::notification::observe_object(
            self.mtm,
            &NotificationName::framework(name),
            AsRef::<AnyObject>::as_ref(&self.item),
            handler,
        );
        self.did_end.replace(Some(observer));
    }

    fn register(&self, key_path: &'static str, initial: bool, handler: impl Fn() + 'static) {
        self.observations.borrow_mut().push(observe_on_main(
            self.mtm, &self.item, key_path, initial, handler,
        ));
    }

    /// Loads the asset's media-selection group for `characteristic` on
    /// `AVFoundation`'s background machinery, then calls `handler` on the
    /// main thread. Dropping the returned [`LoadGuard`] cancels delivery.
    ///
    /// `handler` receives `Err(message)` as the second argument's `Some`.
    pub fn load_media_selection_group(
        &self,
        characteristic: MediaCharacteristic,
        handler: impl FnOnce(Option<Retained<AVMediaSelectionGroup>>, Option<String>) + 'static,
    ) -> LoadGuard {
        let (slot, guard) =
            guarded_completion(self.mtm, move |(group, error)| handler(group, error));
        let block = RcBlock::new(
            move |group: *mut AVMediaSelectionGroup, error: *mut NSError| {
                // SAFETY: `group`/`error` are objects the completion hands
                // us; retaining them keeps them alive until the main queue
                // runs, and they are moved across threads only as raw
                // pointers rebuilt into `Retained` there.
                let group = unsafe { Retained::retain(group) }
                    .map(|retained| Retained::into_raw(retained) as usize);
                // SAFETY: `error` is the object the completion handed us.
                let error =
                    unsafe { Retained::retain(error) }.map(|retained| error_message(&retained));
                let slot = slot.clone();
                main_queue::enqueue(move |mtm| {
                    slot.finish(mtm, move || {
                        // SAFETY: the raw pointer is the `Retained` released
                        // above, taken back exactly once.
                        let group = group.and_then(|ptr| unsafe {
                            Retained::from_raw(ptr as *mut AVMediaSelectionGroup)
                        });
                        (group, error)
                    });
                });
            },
        );
        // SAFETY: the block is retained by AVAsset until the load finishes;
        // it only touches the Send-safe slot plus Objective-C objects it was
        // handed.
        unsafe {
            self.item
                .asset()
                .loadMediaSelectionGroupForMediaCharacteristic_completionHandler(
                    media_characteristic(characteristic),
                    &block,
                );
        }
        guard
    }

    /// Loads the asset's `variants` and calls `handler` on the main thread
    /// with the variants that carry video attributes. `Err(message)` comes
    /// through as the second argument's `Some`.
    pub fn load_video_variants(
        &self,
        handler: impl FnOnce(Vec<Retained<AVAssetVariant>>, Option<String>) + 'static,
    ) -> LoadGuard {
        let (slot, guard) =
            guarded_completion(self.mtm, move |(variants, error)| handler(variants, error));
        // SAFETY: `asset()` returns a live retained asset; every URL-based
        // asset is an `AVURLAsset`, which is where `variants` is declared.
        let Ok(asset) = (unsafe { self.item.asset() }).downcast::<AVURLAsset>() else {
            slot.finish(self.mtm, || {
                (
                    Vec::new(),
                    Some("the player's asset is not a URL asset".to_string()),
                )
            });
            return guard;
        };
        let asset_for_block = asset.clone();
        let block = RcBlock::new(move || {
            let asset = &*asset_for_block;
            // SAFETY: `statusOfValueForKey:`/`variants` on the asset from the
            // completion's thread is what the loading API exists for; the
            // objects stay alive via `asset` above, and only raw pointers
            // cross to the main queue.
            let mut error: *mut NSError = core::ptr::null_mut();
            let key = NSString::from_str("variants");
            // SAFETY: see the block-level note above.
            let status: AVKeyValueStatus =
                unsafe { msg_send![asset, statusOfValueForKey: &*key, error: &mut error] };
            let variants = if status == AVKeyValueStatus::Loaded {
                // SAFETY: see the block-level note above.
                unsafe { asset.variants() }
                    .iter()
                    // SAFETY: `videoAttributes` reads a live variant.
                    .filter(|variant| unsafe { variant.videoAttributes().is_some() })
                    .map(|variant| Retained::into_raw(variant) as usize)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            // SAFETY: `error` is the object the failed load handed us.
            let error = unsafe { Retained::retain(error) }.map(|retained| error_message(&retained));
            let slot = slot.clone();
            main_queue::enqueue(move |mtm| {
                slot.finish(mtm, move || {
                    let variants = variants
                        .into_iter()
                        // SAFETY: each raw pointer is the `Retained` this
                        // block released above, taken back exactly once.
                        .filter_map(|ptr| unsafe { Retained::from_raw(ptr as *mut AVAssetVariant) })
                        .collect();
                    (variants, error)
                });
            });
        });
        let keys = NSArray::from_retained_slice(&[NSString::from_str("variants")]);
        // SAFETY: the block is retained by the asset until the load finishes
        // and only touches Send-safe payloads plus the retained `asset`.
        unsafe {
            asset.loadValuesAsynchronouslyForKeys_completionHandler(&keys, Some(&block));
        }
        guard
    }
}

/// The interesting fields of one `AVPlayerItemAccessLogEvent`.
#[derive(Debug)]
pub struct AccessLogEvent(Retained<AVPlayerItemAccessLogEvent>);

impl AccessLogEvent {
    /// Dropped video frames reported in this event's window.
    #[must_use]
    pub fn dropped_video_frames(&self) -> u64 {
        // SAFETY: property read on a live event object.
        let count = unsafe { self.0.numberOfDroppedVideoFrames() };
        usize::try_from(count).unwrap_or(0) as u64
    }

    /// Stalls the event reports.
    #[must_use]
    pub fn stalls(&self) -> u64 {
        // SAFETY: property read on a live event object.
        let count = unsafe { self.0.numberOfStalls() };
        usize::try_from(count).unwrap_or(0) as u64
    }

    /// Observed network throughput in bits per second (`0` when unknown).
    #[must_use]
    pub fn observed_bitrate(&self) -> u64 {
        // SAFETY: property read on a live event object.
        let bitrate = unsafe { self.0.observedBitrate() };
        if bitrate.is_finite() && bitrate > 0.0 {
            // `u64::try_from` on `f64` does not exist; clamp before casting.
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            let bits = bitrate.min(u64::MAX as f64) as u64;
            bits
        } else {
            0
        }
    }
}

// ── Media selection helpers ──────────────────────────────────────────────────

/// The display label of a media-selection option.
#[must_use]
pub fn media_option_label(option: &AVMediaSelectionOption) -> String {
    // SAFETY: property read on a live option.
    unsafe { option.displayName() }.to_string()
}

/// The BCP-47 tag of a media-selection option, if it declares one.
#[must_use]
pub fn media_option_language(option: &AVMediaSelectionOption) -> Option<String> {
    // SAFETY: property read on a live option.
    unsafe { option.extendedLanguageTag() }.map(|tag| tag.to_string())
}

/// Whether `option` carries `characteristic` (one of the `AVMediaCharacteristic`
/// accessibility/forced-subtitle constants).
fn option_has(
    option: &AVMediaSelectionOption,
    characteristic: &'static AVMediaCharacteristic,
) -> bool {
    // SAFETY: `option` and the constant are live framework objects.
    unsafe { option.hasMediaCharacteristic(characteristic) }
}

/// The semantic roles of a media-selection option (`"description"`,
/// `"caption"`, `"sound-description"`, `"forced-subtitle"`, `"auxiliary"`).
#[must_use]
pub fn media_option_roles(option: &AVMediaSelectionOption) -> Vec<String> {
    // SAFETY: each constant exists on every supported platform.
    let characteristics = unsafe {
        [
            (
                AVMediaCharacteristicDescribesVideoForAccessibility,
                "description",
            ),
            (
                AVMediaCharacteristicTranscribesSpokenDialogForAccessibility,
                "caption",
            ),
            (
                AVMediaCharacteristicDescribesMusicAndSoundForAccessibility,
                "sound-description",
            ),
            (
                AVMediaCharacteristicContainsOnlyForcedSubtitles,
                "forced-subtitle",
            ),
            (AVMediaCharacteristicIsAuxiliaryContent, "auxiliary"),
        ]
    };
    characteristics
        .into_iter()
        .filter_map(|(characteristic, role)| {
            characteristic
                .filter(|characteristic| option_has(option, characteristic))
                .map(|_| role.to_string())
        })
        .collect()
}

/// Whether `option` is a forced-subtitle rendition.
#[must_use]
pub fn media_option_is_forced(option: &AVMediaSelectionOption) -> bool {
    // SAFETY: the constant exists on every supported platform.
    unsafe { AVMediaCharacteristicContainsOnlyForcedSubtitles }
        .is_some_and(|characteristic| option_has(option, characteristic))
}

// ── Variant helpers ──────────────────────────────────────────────────────────

/// The bit rate a variant declares: peak first, else average, else `-1`.
#[must_use]
pub fn variant_declared_bit_rate(variant: &AVAssetVariant) -> f64 {
    // SAFETY: property reads on a live variant.
    unsafe {
        let peak = variant.peakBitRate();
        if peak > 0.0 {
            peak
        } else {
            variant.averageBitRate()
        }
    }
}

/// The presentation size a variant declares; `None` when it has no video
/// attributes.
#[must_use]
pub fn variant_presentation_size(variant: &AVAssetVariant) -> Option<Size> {
    // SAFETY: property read on a live variant.
    unsafe { variant.videoAttributes() }.map(|attributes| {
        // SAFETY: property read on the live attributes object above.
        let size = unsafe { attributes.presentationSize() };
        Size::new(size.width, size.height)
    })
}

/// The codec four-character codes a variant declares.
#[must_use]
pub fn variant_codec_fourccs(variant: &AVAssetVariant) -> Vec<u32> {
    // SAFETY: `videoAttributes` reads a live variant.
    let Some(attributes) = (unsafe { variant.videoAttributes() }) else {
        return Vec::new();
    };
    // SAFETY: `codecTypes` contains `NSNumber` values wrapping
    // `CMVideoCodecType` fourccs.
    unsafe { attributes.codecTypes() }
        .iter()
        .map(|number| number.unsignedIntValue())
        .collect()
}

/// Whether the variant's video range is anything other than SDR.
///
/// # Panics
///
/// If the `AVVideoRange` framework constant is missing (it is not).
#[must_use]
pub fn variant_is_hdr(variant: &AVAssetVariant) -> bool {
    // SAFETY: `videoAttributes` reads a live variant.
    let Some(attributes) = (unsafe { variant.videoAttributes() }) else {
        return false;
    };
    // SAFETY: `videoRange`/`AVVideoRangeSDR` are live framework objects.
    !(unsafe { attributes.videoRange() }).isEqualToString(unsafe {
        AVVideoRangeSDR.expect("AVVideoRangeSDR exists on every supported platform")
    })
}

// ── Player layer view ────────────────────────────────────────────────────────

/// The raw video surface: a [`HostView`] whose backing layer hosts an
/// `AVPlayerLayer` resized in the layout pass.
#[derive(Debug)]
pub struct PlayerLayerView {
    view: Retained<HostView>,
    player_layer: Retained<AVPlayerLayer>,
}

impl PlayerLayerView {
    /// A surface with `gravity` and no player yet.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, gravity: VideoGravity) -> Self {
        let view = HostView::new(mtm, Rect::ZERO);
        // SAFETY: `playerLayerWithPlayer` returns a live layer.
        let player_layer = unsafe { AVPlayerLayer::playerLayerWithPlayer(None) };
        // SAFETY: property write on a live layer.
        unsafe { player_layer.setVideoGravity(video_gravity(gravity)) };
        if let Some(layer) = crate::view::layer(&view) {
            // Both layers are live and main-thread-bound.
            layer.addSublayer(&player_layer);
        }
        let player_layer_for_layout = player_layer.clone();
        let view_for_layout = view.clone();
        view.set_layout_handler(move |_| {
            // Mutating `frame` inside a disabled-action transaction keeps
            // the layer pinned to the view bounds without animating.
            objc2_quartz_core::CATransaction::begin();
            objc2_quartz_core::CATransaction::setDisableActions(true);
            let frame = view_for_layout.bounds();
            player_layer_for_layout.setFrame(frame);
            objc2_quartz_core::CATransaction::commit();
            crate::dynamic_range::apply_resolved_to_layer(
                &player_layer_for_layout,
                &view_for_layout,
            );
        });
        Self { view, player_layer }
    }

    /// The view to mount; on macOS it is an `NSView`, on iOS a `UIView`.
    #[must_use]
    pub fn view(&self) -> &HostView {
        &self.view
    }

    /// The wrapped `AVPlayerLayer`, for direct configuration.
    #[must_use]
    pub fn player_layer(&self) -> &AVPlayerLayer {
        &self.player_layer
    }

    /// Which player renders into the surface.
    pub fn set_player(&self, player: Option<&Player>) {
        // SAFETY: `player` is a live `AVPlayer` the caller keeps alive.
        unsafe { self.player_layer.setPlayer(player.map(Player::raw)) };
    }

    /// Sets how the picture fills the surface.
    pub fn set_video_gravity(&self, gravity: VideoGravity) {
        // SAFETY: property write on a live layer.
        unsafe { self.player_layer.setVideoGravity(video_gravity(gravity)) };
    }
}

// ── Player view (with controls) ──────────────────────────────────────────────

/// A picture-in-picture transition.
#[derive(Debug, Clone)]
pub enum PipEvent {
    /// `PiP` began.
    Started,
    /// `PiP` ended.
    Stopped,
    /// `PiP` failed to start; the message is localized.
    Failed(String),
}

#[cfg(target_os = "macos")]
mod platform {
    #[allow(clippy::wildcard_imports)]
    use super::*;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::NSView;
    use objc2_av_kit::{
        AVPlayerView, AVPlayerViewControlsStyle, AVPlayerViewPictureInPictureDelegate,
    };
    use objc2_foundation::NSObject;

    type PipHandler = Rc<dyn Fn(PipEvent)>;

    #[derive(Default)]
    struct PipDelegateIvars {
        handler: RefCell<Option<PipHandler>>,
    }

    impl fmt::Debug for PipDelegateIvars {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PipDelegateIvars").finish_non_exhaustive()
        }
    }

    define_class!(
        // SAFETY: the protocol methods are optional and declared with their
        // exact signatures; the delegate is retained by `AVPlayerView` while
        // assigned, and everything it touches is main-thread state.
        #[unsafe(super(NSObject))]
        #[name = "CocoaUiPipDelegate"]
        #[thread_kind = MainThreadOnly]
        #[ivars = PipDelegateIvars]
        #[derive(Debug)]
        /// `AVPlayerViewPictureInPictureDelegate` forwarding to a closure.
        struct PipDelegate;

        // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
        unsafe impl NSObjectProtocol for PipDelegate {}

        // SAFETY: the optional methods are declared with their documented
        // signatures; each runs inside `guarded`.
        unsafe impl AVPlayerViewPictureInPictureDelegate for PipDelegate {
            // SAFETY: see the module safety note.
            #[unsafe(method(playerViewDidStartPictureInPicture:))]
            fn did_start(&self, _player_view: &AVPlayerView) {
                guarded("pip did start", || {
                    if let Some(handler) = self.ivars().handler.borrow().as_ref() {
                        handler(PipEvent::Started);
                    }
                });
            }

            // SAFETY: see the module safety note.
            #[unsafe(method(playerViewDidStopPictureInPicture:))]
            fn did_stop(&self, _player_view: &AVPlayerView) {
                guarded("pip did stop", || {
                    if let Some(handler) = self.ivars().handler.borrow().as_ref() {
                        handler(PipEvent::Stopped);
                    }
                });
            }

            // SAFETY: see the module safety note.
            #[unsafe(method(playerView:failedToStartPictureInPictureWithError:))]
            fn failed_to_start(&self, _player_view: &AVPlayerView, error: &NSError) {
                let message = error_message(error);
                guarded("pip failed to start", || {
                    if let Some(handler) = self.ivars().handler.borrow().as_ref() {
                        handler(PipEvent::Failed(message));
                    }
                });
            }
        }
    );

    /// `AVPlayerView`-backed controls surface.
    #[derive(Debug)]
    pub struct PlayerView {
        view: Retained<AVPlayerView>,
        delegate: RefCell<Option<Retained<PipDelegate>>>,
        mtm: MainThreadMarker,
    }

    impl PlayerView {
        /// A controls surface with `gravity` and no player yet.
        #[must_use]
        pub fn new(mtm: MainThreadMarker, gravity: VideoGravity) -> Self {
            // SAFETY: `initWithFrame:` on a live AVPlayerView class.
            let view: Retained<AVPlayerView> = unsafe {
                msg_send![AVPlayerView::alloc(mtm), initWithFrame: objc2_foundation::NSRect::ZERO]
            };
            // SAFETY: property writes on the live view built above.
            unsafe {
                view.setVideoGravity(video_gravity(gravity));
                view.setWantsLayer(true);
            }
            Self {
                view,
                delegate: RefCell::new(None),
                mtm,
            }
        }

        /// The view to mount.
        #[must_use]
        pub fn view(&self) -> &AVPlayerView {
            &self.view
        }

        /// Which player the controls drive.
        pub fn set_player(&self, player: Option<&Player>) {
            // SAFETY: property write on a live view.
            unsafe { self.view.setPlayer(player.map(Player::raw)) };
        }

        /// `true` shows the inline transport controls; `false` hides them.
        pub fn set_shows_controls(&self, shows: bool) {
            // SAFETY: property writes on a live view.
            unsafe {
                self.view.setControlsStyle(if shows {
                    AVPlayerViewControlsStyle::Inline
                } else {
                    AVPlayerViewControlsStyle::None
                });
                self.view.setShowsFullScreenToggleButton(shows);
            }
        }

        /// Whether the view may present picture-in-picture.
        pub fn set_allows_picture_in_picture(&self, allows: bool) {
            // SAFETY: property write on a live view.
            unsafe { self.view.setAllowsPictureInPicturePlayback(allows) };
        }

        /// The layer the view's content renders into, for dynamic-range
        /// tagging.
        #[must_use]
        pub fn backing_layer(&self) -> Option<Retained<objc2_quartz_core::CALayer>> {
            // Property read on a live view after wantsLayer.
            self.view.layer()
        }

        /// Sets the picture-in-picture event handler.
        pub fn set_pip_handler(&self, handler: impl Fn(PipEvent) + 'static) {
            // SAFETY: `init` is the constructor `NSObject` subclasses use.
            let delegate: Retained<PipDelegate> =
                unsafe { msg_send![PipDelegate::alloc(self.mtm), init] };
            delegate.ivars().handler.replace(Some(Rc::new(handler)));
            // SAFETY: `delegate` is a live `PipDelegate` implementing the
            // protocol the property expects.
            unsafe {
                self.view
                    .setPictureInPictureDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            }
            self.delegate.replace(Some(delegate));
        }

        /// macOS keeps the controls surface a plain subview — no controller
        /// to attach.
        pub const fn attach_to_parent_controller(&self) {}

        /// See [`PlayerView::attach_to_parent_controller`].
        pub const fn detach_from_parent_controller(&self) {}
    }

    // Referenced so the `NSView` import is used on every platform this file
    // compiles for.
    const _: () = {
        const fn _uses(_: &NSView) {}
    };
}

#[cfg(target_os = "ios")]
mod platform {
    #[allow(clippy::wildcard_imports)]
    use super::*;
    use objc2::ClassType;
    use objc2::runtime::AnyClass;
    use objc2_foundation::NSObject;
    use objc2_ui_kit::{UIView, UIViewController};

    type PipHandler = Rc<dyn Fn(PipEvent)>;

    #[derive(Default)]
    struct PipDelegateIvars {
        handler: RefCell<Option<PipHandler>>,
    }

    impl fmt::Debug for PipDelegateIvars {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PipDelegateIvars").finish_non_exhaustive()
        }
    }

    define_class!(
        // SAFETY: the `AVPlayerViewControllerDelegate` methods are optional;
        // the selectors here are the ones the protocol declares, implemented
        // on a plain `NSObject` so `delegate` accepts it.
        #[unsafe(super(NSObject))]
        #[name = "CocoaUiPlayerViewControllerDelegate"]
        #[thread_kind = MainThreadOnly]
        #[ivars = PipDelegateIvars]
        #[derive(Debug)]
        /// `AVPlayerViewControllerDelegate` forwarding to a closure.
        struct PipDelegate;

        // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
        unsafe impl NSObjectProtocol for PipDelegate {}

        impl PipDelegate {
            // SAFETY: signature matches `AVPlayerViewControllerDelegate`'s
            // `playerViewControllerDidStartPictureInPicture:`.
            #[unsafe(method(playerViewControllerDidStartPictureInPicture:))]
            fn did_start(&self, _controller: &AnyObject) {
                guarded("pip did start", || {
                    if let Some(handler) = self.ivars().handler.borrow().as_ref() {
                        handler(PipEvent::Started);
                    }
                });
            }

            // SAFETY: signature matches `playerViewControllerDidStopPictureInPicture:`.
            #[unsafe(method(playerViewControllerDidStopPictureInPicture:))]
            fn did_stop(&self, _controller: &AnyObject) {
                guarded("pip did stop", || {
                    if let Some(handler) = self.ivars().handler.borrow().as_ref() {
                        handler(PipEvent::Stopped);
                    }
                });
            }

            // SAFETY: signature matches
            // `playerViewController:failedToStartPictureInPictureWithError:`.
            #[unsafe(method(playerViewController:failedToStartPictureInPictureWithError:))]
            fn failed_to_start(&self, _controller: &AnyObject, error: &NSError) {
                let message = error_message(error);
                guarded("pip failed to start", || {
                    if let Some(handler) = self.ivars().handler.borrow().as_ref() {
                        handler(PipEvent::Failed(message));
                    }
                });
            }
        }
    );

    fn player_view_controller_class() -> &'static AnyClass {
        AnyClass::get(c"AVPlayerViewController")
            .expect("AVKit is available on every supported platform")
    }

    /// `AVPlayerViewController`-backed controls surface. The controller is
    /// held by its class at runtime because `objc2-av-kit` is macOS-only.
    #[derive(Debug)]
    pub struct PlayerView {
        controller: Retained<AnyObject>,
        delegate: RefCell<Option<Retained<PipDelegate>>>,
        mtm: MainThreadMarker,
    }

    impl PlayerView {
        /// A controls surface with `gravity` and no player yet.
        #[must_use]
        pub fn new(mtm: MainThreadMarker, gravity: VideoGravity) -> Self {
            let class = player_view_controller_class();
            // SAFETY: `AVPlayerViewController` responds to `new`; it is a
            // `UIViewController` subclass so `new` returns a live instance.
            let controller: Retained<AnyObject> = unsafe { msg_send![class, new] };
            let view = Self::controller_view(&controller);
            let this = Self {
                controller,
                delegate: RefCell::new(None),
                mtm,
            };
            this.set_video_gravity(gravity);
            let _ = view;
            this
        }

        fn controller_view(controller: &AnyObject) -> Retained<UIView> {
            // SAFETY: `AVPlayerViewController.view` is a live `UIView`.
            unsafe { msg_send![controller, view] }
        }

        /// The view to mount — the controller's root `UIView`.
        #[must_use]
        pub fn view(&self) -> Retained<UIView> {
            Self::controller_view(&self.controller)
        }

        /// Which player the controls drive.
        pub fn set_player(&self, player: Option<&Player>) {
            // SAFETY: `setPlayer:` takes an `AVPlayer` or nil.
            unsafe {
                let _: () = msg_send![&*self.controller, setPlayer: player.map(Player::raw)];
            }
        }

        /// `true` shows the transport controls and enables interaction.
        pub fn set_shows_controls(&self, shows: bool) {
            let view = self.view();
            // SAFETY: property writes on the live controller and its view.
            unsafe {
                let _: () = msg_send![&*self.controller, setShowsPlaybackControls: shows];
                view.setUserInteractionEnabled(shows);
            }
        }

        /// Whether the view may present picture-in-picture.
        pub fn set_allows_picture_in_picture(&self, allows: bool) {
            // SAFETY: property write on the live controller.
            unsafe {
                let _: () = msg_send![&*self.controller, setAllowsPictureInPicturePlayback: allows];
                let _: () = msg_send![&*self.controller, setCanStartPictureInPictureAutomaticallyFromInline: allows];
            }
        }

        /// Sets how the picture fills the surface.
        pub fn set_video_gravity(&self, gravity: VideoGravity) {
            // SAFETY: `videoGravity` is an `NSString`-backed property.
            unsafe {
                let _: () = msg_send![&*self.controller, setVideoGravity: video_gravity(gravity)];
            }
        }

        /// Sets the picture-in-picture event handler.
        pub fn set_pip_handler(&self, handler: impl Fn(PipEvent) + 'static) {
            // SAFETY: `init` is the constructor `NSObject` subclasses use.
            let delegate: Retained<PipDelegate> =
                unsafe { msg_send![PipDelegate::alloc(self.mtm), init] };
            delegate.ivars().handler.replace(Some(Rc::new(handler)));
            // SAFETY: the delegate property accepts any NSObject; ours
            // implements the optional methods it needs.
            unsafe {
                let _: () = msg_send![&*self.controller, setDelegate: &*delegate];
            }
            self.delegate.replace(Some(delegate));
        }

        /// Installs the controller as a child of the nearest view controller
        /// up the responder chain.
        ///
        /// # Panics
        ///
        /// When the surface is mounted outside a `UIViewController`
        /// hierarchy — the same contract the previous implementation had.
        pub fn attach_to_parent_controller(&self) {
            // SAFETY: `parent`/`addChildViewController:` are the standard
            // containment messages on `UIViewController`.
            unsafe {
                let existing: *mut AnyObject = msg_send![&*self.controller, parentViewController];
                if !existing.is_null() {
                    return;
                }
            }
            let mut responder: Option<Retained<objc2_ui_kit::UIResponder>> = {
                let view = self.view();
                // SAFETY: `nextResponder` reads the responder chain.
                unsafe { msg_send![&*view, nextResponder] }
            };
            let parent = loop {
                let Some(current) = responder else {
                    panic!("video player was attached outside a UIViewController hierarchy");
                };
                // A view controller's own view answers its controller as the
                // next responder, so the walk reaches `controller` itself
                // first; skipping it keeps containment aimed at the
                // enclosing hierarchy — `UIKit` raises when a controller is
                // added as its own child.
                let is_self = Retained::as_ptr(&current).cast::<AnyObject>()
                    == Retained::as_ptr(&self.controller);
                // SAFETY: `isKindOfClass:` on a live responder.
                if current.isKindOfClass(UIViewController::class()) && !is_self {
                    break current
                        .downcast::<UIViewController>()
                        .expect("isKindOfClass checked above");
                }
                // SAFETY: `nextResponder` on a live responder.
                responder = unsafe { msg_send![&*current, nextResponder] };
            };
            // SAFETY: `parent` is a live controller; `controller` is the
            // child being added.
            unsafe {
                let _: () = msg_send![&*parent, addChildViewController: &*self.controller];
                let _: () = msg_send![&*self.controller, didMoveToParentViewController: &*parent];
            }
        }

        /// Removes the controller from its parent, if it has one.
        pub fn detach_from_parent_controller(&self) {
            // SAFETY: the containment-uninstall messages are only sent when
            // the controller still has a parent.
            unsafe {
                let existing: *mut AnyObject = msg_send![&*self.controller, parentViewController];
                if existing.is_null() {
                    return;
                }
                let _: () = msg_send![&*self.controller, willMoveToParentViewController: core::ptr::null::<AnyObject>()];
                let _: () = msg_send![&*self.controller, removeFromParentViewController];
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
pub use platform::PlayerView;
