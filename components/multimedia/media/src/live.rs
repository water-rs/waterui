use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::handler::{
    BoxedEventAction, EventHandler, ReportExtraction, boxed_event_handler,
};
use waterui_core::{
    Binding, Computed, Dynamic, Environment, Metadata, SerialDispatch, Signal, SignalExt, View,
    reactive::signal::IntoComputed,
};
use waterui_image::ContentMode;
use waterui_interaction::gesture::{GestureObserver, LongPressGesture};
use waterui_layout::overlay;

use crate::{
    PlaybackSession, Playlist, Url,
    photo::Photo,
    video::{ContentMode as VideoContentMode, Event as VideoEvent, Video},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Represents the source URLs for a live photo, including the image and video components.
#[non_exhaustive]
pub struct LivePhotoSource {
    /// The URL for the still image component of the live photo.
    image: Url,
    /// The URL for the video component of the live photo.
    video: Url,
}

impl LivePhotoSource {
    /// The URL for the still image component of the live photo.
    #[must_use]
    pub const fn image(&self) -> &Url {
        &self.image
    }

    /// The URL for the video component of the live photo.
    #[must_use]
    pub const fn video(&self) -> &Url {
        &self.video
    }
}

/// Events emitted by the `LivePhoto` component.
///
/// [`Self::MotionEnded`] and [`Self::MotionFailed`] both leave the still photo
/// on screen, which is why they must be distinguishable here: without them an
/// observer — an application deciding whether to offer replay, or a test
/// asserting that the motion played — cannot tell a video that finished from
/// one that never ran.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Event {
    /// A long press held past the activation duration started motion playback.
    MotionStarted,
    /// Motion playback reached the end of the video; the still photo is back.
    MotionEnded,
    /// Motion playback could not run; the still photo is back.
    MotionFailed(String),
}

/// A live photo widget composed in Rust from photo, video, and gesture primitives.
///
/// Keeping this composition in Rust avoids adding a dedicated live-photo type
/// to every native FFI surface.
pub struct LivePhoto {
    source: Computed<LivePhotoSource>,
    activation_duration: Duration,
    is_playing: Binding<bool>,
    resizable: bool,
    content_mode: Option<ContentMode>,
    on_event: Option<BoxedEventAction<Event>>,
}

impl core::fmt::Debug for LivePhoto {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LivePhoto")
            .field("source", &self.source)
            .field("activation_duration", &self.activation_duration)
            .field("is_playing", &self.is_playing)
            .field("resizable", &self.resizable)
            .field("content_mode", &self.content_mode)
            .finish_non_exhaustive()
    }
}

impl LivePhotoSource {
    /// Creates a new `LivePhotoSource` instance.
    #[must_use]
    pub const fn new(image: Url, video: Url) -> Self {
        Self { image, video }
    }
}

impl LivePhoto {
    /// Creates a new `LivePhoto` instance.
    #[must_use]
    pub fn new(source: impl IntoComputed<LivePhotoSource>) -> Self {
        Self {
            source: source.into_computed(),
            activation_duration: Duration::from_millis(250),
            is_playing: Binding::bool(false),
            resizable: false,
            content_mode: None,
            on_event: None,
        }
    }

    /// Observes motion playback: when it starts, and how it ends.
    #[must_use]
    pub fn on_event<H, A>(mut self, handler: H) -> Self
    where
        H: EventHandler<Event, A, ()> + 'static,
    {
        self.on_event = Some(boxed_event_handler(handler));
        self
    }

    /// Sets the long-press duration required to activate motion playback.
    #[must_use]
    pub const fn activation_duration(mut self, activation_duration: Duration) -> Self {
        self.activation_duration = activation_duration;
        self
    }

    /// Allows the live photo to stretch to the bounds proposed by its parent.
    #[must_use]
    pub const fn resizable(mut self) -> Self {
        self.resizable = true;
        self
    }

    /// Preserves the live photo's aspect ratio inside the bounds the parent
    /// proposes: [`ContentMode::Fit`] letterboxes, [`ContentMode::Fill`] covers
    /// the bounds and crops the overflow.
    #[must_use]
    pub const fn content_mode(mut self, mode: ContentMode) -> Self {
        self.content_mode = Some(mode);
        self
    }
}

impl View for LivePhoto {
    fn body(self, env: &Environment) -> impl View {
        let source = self.source;
        let activation_duration_ms =
            u32::try_from(self.activation_duration.as_millis()).unwrap_or(u32::MAX);
        let is_playing = self.is_playing;
        let resizable = self.resizable;
        let content_mode = self.content_mode;
        let motion_source = source.clone();
        let still_source = source.map(|source| source.image).computed();
        let playback_state = is_playing.clone();
        let is_playing_on_press = is_playing.clone();
        let report = LivePhotoReporter::new(self.on_event, env.clone());
        let report_on_press = report.clone();
        let motion = Dynamic::watch(is_playing, move |is_playing_now| {
            is_playing_now.then(|| {
                live_photo_video(
                    motion_source.snapshot(),
                    playback_state.clone(),
                    report.clone(),
                    content_mode,
                )
            })
        });

        let mut still = Photo::new(still_source);
        if resizable {
            still = still.resizable();
        }
        if let Some(mode) = content_mode {
            still = still.content_mode(mode);
        }

        Metadata::new(
            overlay(still, motion),
            GestureObserver::new(LongPressGesture::new(activation_duration_ms), move || {
                if !is_playing_on_press.snapshot() {
                    is_playing_on_press.set(true);
                    report_on_press.emit(Event::MotionStarted);
                }
            }),
        )
    }
}

/// Shared handle to the live photo's event handler.
///
/// The handler is installed once but reached from both the press gesture and
/// every video instance the motion overlay mounts, so it is shared rather than
/// moved into one of them.
#[derive(Clone)]
struct LivePhotoReporter {
    inner: Rc<LivePhotoReporterInner>,
}

struct LivePhotoReporterInner {
    handler: RefCell<Option<BoxedEventAction<Event>>>,
    /// Events emitted while the handler still runs; they are delivered after
    /// it returns, in order, rather than re-entering it.
    events: SerialDispatch<Event>,
    env: Environment,
}

impl LivePhotoReporter {
    fn new(handler: Option<BoxedEventAction<Event>>, env: Environment) -> Self {
        Self {
            inner: Rc::new(LivePhotoReporterInner {
                handler: RefCell::new(handler),
                events: SerialDispatch::new(),
                env,
            }),
        }
    }

    fn emit(&self, event: Event) {
        self.inner.events.deliver(event, |event| {
            if let Some(handler) = self.inner.handler.borrow_mut().as_mut() {
                handler(event, &self.inner.env).report_extraction();
            }
        });
    }
}

fn live_photo_video(
    source: LivePhotoSource,
    is_playing: Binding<bool>,
    report: LivePhotoReporter,
    content_mode: Option<ContentMode>,
) -> Video {
    let session = PlaybackSession::new(Playlist::single(source.video)).autoplay();
    session.controller().muted().set(true);
    let mut video = Video::new(session);
    if let Some(mode) = content_mode {
        let video_mode = match mode {
            ContentMode::Fit => VideoContentMode::Fit,
            ContentMode::Fill => VideoContentMode::Fill,
        };
        video = video.content_mode(video_mode);
    }
    video.on_event(move |event| {
        // Both outcomes put the still photo back, but they are not the same
        // thing: one played, the other never could. An observer that cannot
        // tell them apart reads a failure as a completed playback.
        match event {
            VideoEvent::Ended => {
                is_playing.set(false);
                report.emit(Event::MotionEnded);
            }
            VideoEvent::Error { message } => {
                is_playing.set(false);
                report.emit(Event::MotionFailed(message));
            }
            _ => {}
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// An event emitted while the handler still runs must queue behind the
    /// in-flight call instead of re-entering the handler.
    #[test]
    fn a_nested_emit_is_delivered_after_the_handler_returns() {
        let reporter = LivePhotoReporter::new(None, Environment::new());
        let delivered = Rc::new(RefCell::new(Vec::<String>::new()));
        let inside = Cell::new(false);
        let once = Cell::new(false);
        *reporter.inner.handler.borrow_mut() = Some(Box::new({
            let reporter = reporter.clone();
            let delivered = Rc::clone(&delivered);
            move |event: Event, _env: &Environment| {
                assert!(
                    !inside.replace(true),
                    "the handler must not run re-entrantly"
                );
                delivered.borrow_mut().push(format!("{event:?}"));
                if !once.replace(true) {
                    reporter.emit(Event::MotionEnded);
                }
                inside.set(false);
                Ok(())
            }
        }));

        reporter.emit(Event::MotionStarted);

        assert_eq!(&*delivered.borrow(), &["MotionStarted", "MotionEnded"]);
    }
}
