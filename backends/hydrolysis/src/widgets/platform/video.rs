//! The video leaves on the web: the browser's own `<video>` element.
//!
//! The page's media element is the web's native player, the counterpart of
//! `AVPlayer` on Apple platforms. Each `Video` and `VideoPlayer` mounts one
//! `<video>` as hosted content: the engine places it on a DOM plane at the
//! leaf's laid-out frame, clip and stacking order, and the browser decodes,
//! composites and draws its controls. Its pixels never reach the engine, so
//! GPU effects do not apply to them.
//!
//! The bridge plays what the element plays. Progressive sources play
//! everywhere; an HLS source plays where the element reports it can; DASH and
//! DRM-protected sources fail with a playback error. The browser owns
//! buffering, so the playback policy's buffering hints have no counterpart
//! here, the element cannot step by frame, and it exposes no embedded audio
//! or video tracks to select. Sidecar subtitle tracks become `<track>`
//! children of the element.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed, Signal};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use waterui_core::{AnyView, Environment, Native};
use waterui_video::video::{
    AudioTrackSelection, ContentMode, Event, PlaybackConfiguration, PlaybackOutputPath,
    PlaybackPowerPolicy, SubtitleSelection, SubtitleTrackInfo, SubtitleTrackOrigin, TrackCatalog,
    VideoConfig, VideoPlayerConfig, VideoProjection, VideoTrackSelection,
};
use waterui_video::{
    Delivery, LiveWindow, MediaItem, PlaybackMetrics, PlaybackPhase, PlayerController, RepeatMode,
    Volume,
};
use web_sys::{HtmlElement, HtmlTrackElement, HtmlVideoElement, TextTrackMode};

use crate::time::Instant;
use crate::{HostedContent, HostedObject, HostedOcclusion, HostedView};

/// The MIME type an HLS playlist is offered to the element as.
const HLS_MIME: &str = "application/vnd.apple.mpegurl";

/// Installs the `Video` and `VideoPlayer` realizations.
pub fn install(env: &mut Environment) {
    env.insert_hook::<VideoConfig, AnyView>(|env, config| {
        let VideoConfig {
            playback,
            content_mode,
            projection,
            loops,
        } = config;
        leaf(env, playback, content_mode, &projection, false, loops)
    });
    env.insert_hook::<VideoPlayerConfig, AnyView>(|env, config| {
        let VideoPlayerConfig {
            playback,
            content_mode,
            projection,
            show_controls,
        } = config;
        leaf(
            env,
            playback,
            content_mode,
            &projection,
            show_controls,
            false,
        )
    });
}

/// One leaf: the element, the coordinator mirroring the playback contract
/// into it, and the hosted view that places it.
fn leaf(
    env: &Environment,
    playback: PlaybackConfiguration,
    content_mode: ContentMode,
    projection: &VideoProjection,
    controls: bool,
    loops: bool,
) -> AnyView {
    assert!(
        !projection.is_spherical(),
        "spherical video projection is not supported by the web video element realization"
    );
    match playback.playback_policy.power {
        PlaybackPowerPolicy::PlatformManaged => {}
        PlaybackPowerPolicy::RequireAudioOffload
        | PlaybackPowerPolicy::RequireAudioVideoTunneling => {
            panic!(
                "{:?} cannot be satisfied by the web video element realization",
                playback.playback_policy.power
            )
        }
    }
    let element = video_element(content_mode, controls);
    let player = Player::new(env, element, playback, loops);
    let aspect = player.aspect.clone().into();
    let natural_width = player.natural_width.clone().into();
    let hosted = Native::new(HostedView::new(VideoContent { player }));
    match content_mode {
        ContentMode::Fit => AnyView::new(waterui_video::fit_video(hosted, aspect, natural_width)),
        ContentMode::Fill | ContentMode::Stretch => AnyView::new(hosted),
    }
}

/// A `<video>` element configured for `content_mode`, with the browser's
/// controls when `controls` is set.
fn video_element(content_mode: ContentMode, controls: bool) -> HtmlVideoElement {
    let element: HtmlVideoElement = web_sys::window()
        .and_then(|window| window.document())
        .expect("hydrolysis web video: the page has no document")
        .create_element("video")
        .expect("hydrolysis web video: failed to create a <video> element")
        .unchecked_into();
    element.set_controls(controls);
    element.set_preload("auto");
    // Inline on iOS Safari, which otherwise takes playback fullscreen.
    element
        .set_attribute("playsinline", "")
        .expect("hydrolysis web video: failed to mark the element inline");
    let style = element.style();
    for (property, value) in [
        (
            "object-fit",
            match content_mode {
                ContentMode::Fit => "contain",
                ContentMode::Fill => "cover",
                ContentMode::Stretch => "fill",
            },
        ),
        // A player letterboxes on black, as the platform players do; a bare
        // video shows whatever lies behind its letterbox.
        (
            "background-color",
            if controls { "black" } else { "transparent" },
        ),
    ] {
        style
            .set_property(property, value)
            .expect("hydrolysis web video: failed to style the element");
    }
    element
}

/// A DOM event listener, registered while it is held.
type Listener = Closure<dyn FnMut(web_sys::Event)>;

/// What one media event does to the coordinator.
type Handler = fn(&Rc<Player>);

/// The bindings the coordinator watches and writes, lifted off
/// [`PlaybackConfiguration`].
struct Bindings {
    source: Computed<MediaItem>,
    subtitle_selection: Binding<SubtitleSelection>,
    audio_track_selection: Binding<AudioTrackSelection>,
    video_track_selection: Binding<VideoTrackSelection>,
    track_catalog: Binding<TrackCatalog>,
    live_window: Binding<Option<LiveWindow>>,
    has_next: Binding<bool>,
    volume: Binding<Volume>,
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
}

/// The coordinator: one per element. Every DOM listener and binding watcher
/// holds it weakly; the hosted content owns it.
///
/// No borrow is held across a binding write or an emitted event, so a
/// handler or watcher that writes back into the coordinator never finds it
/// borrowed.
struct Player {
    element: HtmlVideoElement,
    bindings: Bindings,
    controller: PlayerController,
    emit: Option<Rc<dyn Fn(Event)>>,
    loops: bool,
    /// The `(url, delivery)` of the loaded source; a repeated source is not
    /// reloaded.
    source_key: RefCell<Option<(String, Delivery)>>,
    /// Whether `ReadyToPlay` was reported for the current source.
    ready: Cell<bool>,
    /// Whether the current source's end was handled.
    ended: Cell<bool>,
    /// Whether the element is waiting for data after reporting
    /// `Buffering`; the stall ends with `BufferingEnded`.
    stalled: Cell<bool>,
    /// When the current source began loading, for the metrics' start-up
    /// time.
    loaded_at: Cell<Instant>,
    last_buffer_level_ms: Cell<Option<u32>>,
    seen_seek_generation: Cell<u64>,
    seen_step_forward: Cell<u64>,
    seen_step_backward: Cell<u64>,
    /// The `<track>` children the current source's sidecar subtitles added.
    tracks: RefCell<Vec<HtmlTrackElement>>,
    focused: Binding<bool>,
    /// The source's width-to-height ratio, 16:9 until it reports its size.
    aspect: Binding<f32>,
    /// The source's own width, once it reports its size.
    natural_width: Binding<Option<f32>>,
    listeners: RefCell<Vec<Listener>>,
    guards: RefCell<Vec<BoxWatcherGuard>>,
}

impl Player {
    fn new(
        env: &Environment,
        element: HtmlVideoElement,
        playback: PlaybackConfiguration,
        loops: bool,
    ) -> Rc<Self> {
        let bindings = Bindings {
            source: playback.source,
            subtitle_selection: playback.subtitle_selection,
            audio_track_selection: playback.audio_track_selection,
            video_track_selection: playback.video_track_selection,
            track_catalog: playback.track_catalog,
            live_window: playback.live_window,
            has_next: playback.has_next,
            volume: playback.volume,
            muted: playback.muted,
            playback_rate: playback.playback_rate,
            preserve_pitch: playback.preserve_pitch,
            desired_playing: playback.desired_playing,
            seek_target_seconds: playback.seek_target_seconds,
            seek_generation: playback.seek_generation,
            step_forward_generation: playback.step_forward_generation,
            step_backward_generation: playback.step_backward_generation,
            position_seconds: playback.position_seconds,
            duration_seconds: playback.duration_seconds,
            phase: playback.phase,
            repeat: playback.repeat,
        };
        let player = Rc::new(Self {
            seen_seek_generation: Cell::new(bindings.seek_generation.snapshot()),
            seen_step_forward: Cell::new(bindings.step_forward_generation.snapshot()),
            seen_step_backward: Cell::new(bindings.step_backward_generation.snapshot()),
            element,
            bindings,
            controller: playback.controller,
            emit: playback
                .on_event
                .map(|handler| handler.bind_callback(env.clone())),
            loops,
            source_key: RefCell::new(None),
            ready: Cell::new(false),
            ended: Cell::new(false),
            stalled: Cell::new(false),
            loaded_at: Cell::new(Instant::now()),
            last_buffer_level_ms: Cell::new(None),
            tracks: RefCell::new(Vec::new()),
            focused: nami::binding(false),
            aspect: nami::binding(waterui_video::DEFAULT_ASPECT),
            natural_width: nami::binding(None),
            listeners: RefCell::new(Vec::new()),
            guards: RefCell::new(Vec::new()),
        });
        player.listen();
        player.watch();
        player.emit(Event::PlaybackOutputPathChanged {
            path: PlaybackOutputPath::PlatformManaged,
        });
        player.apply_audio();
        player.load(&player.bindings.source.snapshot());
        player
    }

    fn emit(&self, event: Event) {
        if let Some(emit) = &self.emit {
            emit(event);
        }
    }

    fn emit_error(&self, message: impl Into<String>) {
        self.emit(Event::Error {
            message: message.into(),
        });
    }

    fn set_phase(&self, phase: PlaybackPhase) {
        if self.bindings.phase.snapshot() != phase {
            self.bindings.phase.set(phase);
        }
    }

    /// Fails the current source: an error event, the failed phase, and an
    /// element that loads nothing.
    fn fail(&self, message: impl Into<String>) {
        self.element
            .remove_attribute("src")
            .expect("hydrolysis web video: failed to clear the source");
        self.element.load();
        self.emit_error(message);
        self.set_phase(PlaybackPhase::Failed);
    }

    /// Loads `media` into the element, unless it is the source already
    /// loaded.
    fn load(&self, media: &MediaItem) {
        let url = media.source.to_string();
        let key = (url.clone(), media.delivery);
        if self.source_key.borrow().as_ref() == Some(&key) {
            return;
        }
        self.source_key.replace(Some(key));
        self.ready.set(false);
        self.ended.set(false);
        self.stalled.set(false);
        self.last_buffer_level_ms.set(None);
        self.loaded_at.set(Instant::now());
        self.bindings.duration_seconds.set(0.0);
        self.bindings.position_seconds.set(0.0);
        self.bindings.live_window.set(None);
        self.bindings.track_catalog.set(TrackCatalog::default());
        self.aspect.set(waterui_video::DEFAULT_ASPECT);
        self.natural_width.set(None);
        for track in self.tracks.take() {
            track.remove();
        }

        if media.delivery == Delivery::Dash {
            self.fail("MPEG-DASH is not supported by the web video element realization");
            return;
        }
        if media.drm.is_some() {
            self.fail(
                "DRM-protected sources are not supported by the web video element realization",
            );
            return;
        }
        if media.delivery == Delivery::Hls && self.element.can_play_type(HLS_MIME).is_empty() {
            self.fail("this browser's video element does not play HLS");
            return;
        }

        self.element.set_src(&url);
        self.add_subtitle_tracks(media);
        self.set_phase(PlaybackPhase::Preparing);
    }

    /// The source's sidecar subtitles, as `<track>` children and as the
    /// catalog's subtitle slice.
    fn add_subtitle_tracks(&self, media: &MediaItem) {
        let document = self
            .element
            .owner_document()
            .expect("hydrolysis web video: the element has no document");
        let mut infos = Vec::with_capacity(media.subtitle_tracks.len());
        let mut tracks = Vec::with_capacity(media.subtitle_tracks.len());
        for (index, track) in media.subtitle_tracks.iter().enumerate() {
            let element: HtmlTrackElement = document
                .create_element("track")
                .expect("hydrolysis web video: failed to create a <track> element")
                .unchecked_into();
            element.set_kind("subtitles");
            element.set_src(track.source.as_ref());
            if let Some(language) = &track.language {
                element.set_srclang(language);
            }
            let label = track
                .label
                .clone()
                .or_else(|| track.language.clone())
                .unwrap_or_else(|| format!("Subtitles {}", index + 1));
            element.set_label(&label);
            self.element
                .append_child(&element)
                .expect("hydrolysis web video: failed to add a subtitle track");
            infos.push(SubtitleTrackInfo::new(
                label,
                track.language.clone(),
                Vec::new(),
                track.forced,
                SubtitleTrackOrigin::Sidecar,
            ));
            tracks.push(element);
        }
        self.tracks.replace(tracks);
        let catalog = self
            .bindings
            .track_catalog
            .snapshot()
            .replacing_subtitles(infos);
        self.bindings.track_catalog.set(catalog);
        self.apply_subtitle_selection();
    }

    /// `subtitle_selection` onto the element's text tracks: `Auto` leaves
    /// the browser's own choice, `Off` disables every track, `Track(i)`
    /// shows the `i`-th alone.
    fn apply_subtitle_selection(&self) {
        let selection = self.bindings.subtitle_selection.snapshot();
        let Some(list) = self.element.text_tracks() else {
            return;
        };
        let count = list.length();
        match selection {
            SubtitleSelection::Auto => {}
            SubtitleSelection::Off => {
                for index in 0..count {
                    if let Some(track) = list.get(index) {
                        track.set_mode(TextTrackMode::Disabled);
                    }
                }
            }
            SubtitleSelection::Track(selected) => {
                if selected >= crate::num_cast::u64_as_usize(u64::from(count)) {
                    self.emit_error(format!(
                        "subtitle track {selected} is out of range ({count} tracks)"
                    ));
                    return;
                }
                for index in 0..count {
                    if let Some(track) = list.get(index) {
                        track.set_mode(
                            if crate::num_cast::u64_as_usize(u64::from(index)) == selected {
                                TextTrackMode::Showing
                            } else {
                                TextTrackMode::Disabled
                            },
                        );
                    }
                }
            }
        }
    }

    /// The element exposes no embedded audio or video tracks, so the
    /// catalog lists none and only `Auto` can be met.
    fn apply_track_selections(&self) {
        if let AudioTrackSelection::Track(index) = self.bindings.audio_track_selection.snapshot() {
            self.emit_error(format!("audio track {index} is out of range (0 tracks)"));
        }
        if let VideoTrackSelection::Track(index) = self.bindings.video_track_selection.snapshot() {
            self.emit_error(format!("video track {index} is out of range (0 tracks)"));
        }
    }

    /// Volume, mute, rate and pitch onto the element.
    fn apply_audio(&self) {
        self.element
            .set_volume(f64::from(self.bindings.volume.snapshot().level()));
        self.element.set_muted(self.bindings.muted.snapshot());
        let rate = self.bindings.playback_rate.snapshot();
        if rate > 0.0 {
            self.element.set_playback_rate(f64::from(rate));
        }
        js_sys::Reflect::set(
            &self.element,
            &JsValue::from_str("preservesPitch"),
            &JsValue::from_bool(self.bindings.preserve_pitch.snapshot()),
        )
        .expect("hydrolysis web video: failed to set preservesPitch");
    }

    /// Starts playback. A browser that refuses — an autoplay policy waiting
    /// for a user gesture — reports it as a playback error, and the request
    /// is withdrawn so the bindings say what the element does.
    fn start_playing(self: &Rc<Self>) {
        let promise = self
            .element
            .play()
            .expect("hydrolysis web video: HTMLMediaElement.play threw");
        let weak = Rc::downgrade(self);
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = wasm_bindgen_futures::JsFuture::from(promise).await
                && let Some(player) = weak.upgrade()
            {
                let message = js_sys::Error::from(error).message();
                player.emit_error(format!("the browser refused to play: {message}"));
                player.bindings.desired_playing.set(false);
            }
        });
    }

    fn apply_desired_playing(self: &Rc<Self>, desired: bool) {
        if desired {
            if self.ready.get() && self.element.paused() {
                self.start_playing();
            }
        } else if !self.element.paused() {
            self.element
                .pause()
                .expect("hydrolysis web video: HTMLMediaElement.pause threw");
        }
    }

    /// A seek request, deduplicated by generation and clamped into the last
    /// seekable range, or `[0, duration]`.
    fn apply_seek(&self, generation: u64) {
        if generation == self.seen_seek_generation.replace(generation) {
            return;
        }
        let target = self.bindings.seek_target_seconds.snapshot();
        let seekable = self.element.seekable();
        let clamped = if seekable.length() > 0 {
            let last = seekable.length() - 1;
            target.clamp(
                seekable.start(last).unwrap_or(0.0),
                seekable.end(last).unwrap_or(target),
            )
        } else {
            let duration = self.element.duration();
            if duration.is_finite() {
                target.clamp(0.0, duration)
            } else {
                target.max(0.0)
            }
        };
        self.element.set_current_time(clamped);
        self.bindings.position_seconds.set(clamped);
    }

    fn apply_step(&self, forward: bool, generation: u64) {
        let seen = if forward {
            &self.seen_step_forward
        } else {
            &self.seen_step_backward
        };
        if generation == seen.replace(generation) {
            return;
        }
        self.emit_error("the web video element cannot step by frame");
    }

    /// The element's duration, when finite; a live stream's is infinite and
    /// leaves the binding at zero.
    fn report_duration(&self) {
        let duration = self.element.duration();
        if duration.is_finite() && duration >= 0.0 {
            self.bindings.duration_seconds.set(duration);
        }
        self.update_live_window();
    }

    /// A live stream's timeline, from the element's seekable range.
    fn update_live_window(&self) {
        if self.element.duration().is_finite() {
            return;
        }
        let seekable = self.element.seekable();
        if seekable.length() == 0 {
            return;
        }
        let last = seekable.length() - 1;
        let start = seekable.start(0).unwrap_or(0.0).max(0.0);
        let end = seekable.end(last).unwrap_or(start).max(start);
        let seconds = core::time::Duration::from_secs_f64;
        self.bindings.live_window.set(Some(LiveWindow::new(
            seconds(start),
            seconds(end),
            seconds(end),
            seconds(end),
        )));
    }

    /// The source's size, once the element reports it: the `Fit` leaf
    /// answers by its aspect ratio.
    fn report_size(&self) {
        let (width, height) = (self.element.video_width(), self.element.video_height());
        if width == 0 || height == 0 {
            return;
        }
        let width = crate::num_cast::u32_as_f32(width);
        let aspect = width / crate::num_cast::u32_as_f32(height);
        if self.aspect.snapshot().to_bits() != aspect.to_bits() {
            self.aspect.set(aspect);
        }
        if self.natural_width.snapshot() != Some(width) {
            self.natural_width.set(Some(width));
        }
    }

    /// The buffered extent ahead of the playhead, in milliseconds.
    fn buffered_ahead_ms(&self) -> u32 {
        let position = self.element.current_time();
        let buffered = self.element.buffered();
        (0..buffered.length())
            .find_map(|index| {
                let start = buffered.start(index).ok()?;
                let end = buffered.end(index).ok()?;
                (start <= position && position <= end).then_some(end - position)
            })
            .map_or(0, |ahead| {
                crate::num_cast::f64_as_u32((ahead * 1000.0).round())
            })
    }

    /// The playhead moved: position, buffer level and metrics.
    fn tick(&self) {
        let position = self.element.current_time();
        self.bindings.position_seconds.set(position);
        let buffered_ms = self.buffered_ahead_ms();
        if self.last_buffer_level_ms.replace(Some(buffered_ms)) != Some(buffered_ms) {
            self.emit(Event::BufferLevel { buffered_ms });
        }
        let mut metrics = PlaybackMetrics::new(
            core::time::Duration::from_secs_f64(position.max(0.0)),
            core::time::Duration::from_millis(u64::from(buffered_ms)),
            self.loaded_at.get().elapsed(),
        );
        let quality = self.element.get_video_playback_quality();
        metrics = metrics.dropped_video_frames(u64::from(quality.dropped_video_frames()));
        self.emit(Event::PlaybackMetrics { metrics });
    }

    /// Data flows again: a reported stall ends.
    fn end_stall(&self) {
        if self.stalled.replace(false) {
            self.emit(Event::BufferingEnded);
        }
    }

    fn can_play(self: &Rc<Self>) {
        self.end_stall();
        if self.ready.replace(true) {
            return;
        }
        self.report_duration();
        self.emit(Event::ReadyToPlay);
        self.set_phase(PlaybackPhase::Ready);
        self.apply_subtitle_selection();
        self.apply_track_selections();
        if self.bindings.desired_playing.snapshot() {
            self.start_playing();
        }
    }

    fn playing(&self) {
        self.end_stall();
        self.set_phase(PlaybackPhase::Playing);
        self.emit(Event::PlaybackStateChanged { playing: true });
        // The browser's own controls started it.
        if !self.bindings.desired_playing.snapshot() {
            self.bindings.desired_playing.set(true);
        }
    }

    fn paused(&self) {
        // The end of the source pauses the element too; `ended` handles it.
        if self.element.ended() {
            return;
        }
        if matches!(
            self.bindings.phase.snapshot(),
            PlaybackPhase::Playing | PlaybackPhase::Buffering
        ) {
            self.set_phase(PlaybackPhase::Paused);
        }
        self.emit(Event::PlaybackStateChanged { playing: false });
        // Loading a new source pauses the element before it is ready; only
        // a pause of a ready source withdraws the request to play, so a
        // playlist that advances keeps playing.
        if self.ready.get() && self.bindings.desired_playing.snapshot() {
            self.bindings.desired_playing.set(false);
        }
    }

    fn waiting(&self) {
        if !self.stalled.replace(true) {
            self.emit(Event::Buffering);
            self.set_phase(PlaybackPhase::Buffering);
        }
    }

    /// The end of the source: loop, repeat, advance, or stop.
    fn ended(self: &Rc<Self>) {
        if self.ended.replace(true) {
            return;
        }
        self.emit(Event::Ended);
        let repeat = self.bindings.repeat.snapshot();
        if self.loops || repeat == RepeatMode::One {
            self.element.set_current_time(0.0);
            self.ended.set(false);
            self.start_playing();
            return;
        }
        if (repeat == RepeatMode::All || self.bindings.has_next.snapshot())
            && self.controller.next().is_ok()
        {
            return;
        }
        self.set_phase(PlaybackPhase::Ended);
    }

    fn failed(&self) {
        let message = self.element.error().map_or_else(
            || String::from("the video element failed without an error"),
            |error| {
                let message = error.message();
                if message.is_empty() {
                    format!(
                        "the video element failed (MediaError code {})",
                        error.code()
                    )
                } else {
                    message
                }
            },
        );
        self.emit_error(message);
        self.set_phase(PlaybackPhase::Failed);
    }

    /// The browser's controls changed the volume or mute state.
    fn volume_changed(&self) {
        let level = crate::num_cast::f64_as_f32(self.element.volume());
        if self.bindings.volume.snapshot().level().to_bits() != level.to_bits() {
            self.bindings.volume.set(Volume::new(level));
        }
        let muted = self.element.muted();
        if self.bindings.muted.snapshot() != muted {
            self.bindings.muted.set(muted);
        }
    }

    /// The browser's controls changed the playback rate.
    fn rate_changed(&self) {
        let rate = crate::num_cast::f64_as_f32(self.element.playback_rate());
        if self.bindings.playback_rate.snapshot().to_bits() != rate.to_bits() {
            self.bindings.playback_rate.set(rate);
        }
    }

    /// Registers the element's media, picture-in-picture and focus events.
    fn listen(self: &Rc<Self>) {
        let handlers: [(&str, Handler); 17] = [
            ("canplay", Self::can_play),
            ("playing", |player| player.playing()),
            ("pause", |player| player.paused()),
            ("waiting", |player| player.waiting()),
            ("timeupdate", |player| player.tick()),
            ("loadedmetadata", |player| {
                player.report_duration();
                player.report_size();
            }),
            ("resize", |player| player.report_size()),
            ("durationchange", |player| player.report_duration()),
            ("progress", |player| player.update_live_window()),
            ("ended", Self::ended),
            ("error", |player| player.failed()),
            ("volumechange", |player| player.volume_changed()),
            ("ratechange", |player| player.rate_changed()),
            ("enterpictureinpicture", |player| {
                player.emit(Event::PictureInPictureChanged { active: true });
            }),
            ("leavepictureinpicture", |player| {
                player.emit(Event::PictureInPictureChanged { active: false });
            }),
            ("focusin", |player| player.focused.set(true)),
            ("focusout", |player| player.focused.set(false)),
        ];
        let mut listeners = self.listeners.borrow_mut();
        for (name, handler) in handlers {
            let weak = Rc::downgrade(self);
            let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |_event| {
                if let Some(player) = weak.upgrade() {
                    handler(&player);
                }
            });
            self.element
                .add_event_listener_with_callback(name, closure.as_ref().unchecked_ref())
                .unwrap_or_else(|_| panic!("hydrolysis web video: failed to listen for {name}"));
            listeners.push(closure);
        }
    }

    /// Watches every binding the element follows.
    fn watch(self: &Rc<Self>) {
        let bindings = &self.bindings;
        let mut guards = self.guards.borrow_mut();
        guards.push(Box::new(bindings.source.watch(watcher(
            self,
            |player, media| {
                player.load(&media);
            },
        ))));
        guards.push(Box::new(bindings.desired_playing.watch(watcher(
            self,
            |player, desired| {
                player.apply_desired_playing(desired);
            },
        ))));
        guards.push(Box::new(
            bindings
                .volume
                .watch(watcher(self, |player, _: Volume| player.apply_audio())),
        ));
        guards.push(Box::new(
            bindings
                .muted
                .watch(watcher(self, |player, _: bool| player.apply_audio())),
        ));
        guards.push(Box::new(
            bindings
                .playback_rate
                .watch(watcher(self, |player, _: f32| player.apply_audio())),
        ));
        guards.push(Box::new(
            bindings
                .preserve_pitch
                .watch(watcher(self, |player, _: bool| player.apply_audio())),
        ));
        guards.push(Box::new(bindings.seek_generation.watch(watcher(
            self,
            |player, generation| {
                player.apply_seek(generation);
            },
        ))));
        guards.push(Box::new(bindings.step_forward_generation.watch(watcher(
            self,
            |player, generation| {
                player.apply_step(true, generation);
            },
        ))));
        guards.push(Box::new(bindings.step_backward_generation.watch(watcher(
            self,
            |player, generation| {
                player.apply_step(false, generation);
            },
        ))));
        guards.push(Box::new(bindings.subtitle_selection.watch(watcher(
            self,
            |player, _: SubtitleSelection| {
                player.apply_subtitle_selection();
            },
        ))));
        guards.push(Box::new(bindings.audio_track_selection.watch(watcher(
            self,
            |player, _: AudioTrackSelection| {
                player.apply_track_selections();
            },
        ))));
        guards.push(Box::new(bindings.video_track_selection.watch(watcher(
            self,
            |player, _: VideoTrackSelection| {
                player.apply_track_selections();
            },
        ))));
    }

    /// Stops the element and releases its source and listeners.
    fn release(&self) {
        self.element
            .pause()
            .expect("hydrolysis web video: HTMLMediaElement.pause threw");
        self.element
            .remove_attribute("src")
            .expect("hydrolysis web video: failed to clear the source");
        self.element.load();
        self.guards.take();
        for closure in self.listeners.take() {
            drop(closure);
        }
    }
}

/// A binding watcher that reaches the coordinator through a weak handle.
fn watcher<T: 'static>(
    player: &Rc<Player>,
    apply: impl Fn(&Rc<Player>, T) + 'static,
) -> impl Fn(nami::watcher::Context<T>) + 'static {
    let weak: Weak<Player> = Rc::downgrade(player);
    move |context| {
        if let Some(player) = weak.upgrade() {
            apply(&player, context.into_value());
        }
    }
}

/// The hosted half of a video leaf.
struct VideoContent {
    player: Rc<Player>,
}

impl HostedContent for VideoContent {
    fn mount(&self, occlusion: HostedOcclusion) -> HostedObject {
        let element: HtmlElement = self.player.element.clone().unchecked_into();
        let redirect = crate::platform::redirect_occluded_input(&element, occlusion, true);
        self.player.listeners.borrow_mut().extend(redirect);
        cherenkov_gpu::interop::web::HostedElement::new(element)
    }

    fn unmount(&self) {
        self.player.release();
    }

    fn focused(&self) -> Computed<bool> {
        self.player.focused.clone().into()
    }

    fn request_focus(&self) {
        let _ = self.player.element.focus();
    }
}
