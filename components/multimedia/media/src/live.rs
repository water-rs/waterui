use std::cell::RefCell;
use std::rc::Rc;

use waterui_core::gesture::{GestureObserver, LongPressGesture};
use waterui_core::handler::{BoxedEventAction, EventHandler, boxed_event_handler};
use waterui_core::{
    AnyView, Binding, Computed, Dynamic, Environment, Metadata, Signal, SignalExt, View,
    reactive::signal::IntoComputed,
};
use waterui_layout::overlay;

use crate::{
    PlaybackSession, Playlist, Url,
    photo::Photo,
    video::{Event as VideoEvent, Video},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Represents the source URLs for a live photo, including the image and video components.
pub struct LivePhotoSource {
    /// The URL for the still image component of the live photo.
    pub image: Url,
    /// The URL for the video component of the live photo.
    pub video: Url,
}

/// Events emitted by the `LivePhoto` component.
///
/// [`Self::MotionEnded`] and [`Self::MotionFailed`] both leave the still photo
/// on screen, which is why they must be distinguishable here: without them an
/// observer — an application deciding whether to offer replay, or a test
/// asserting that the motion played — cannot tell a video that finished from
/// one that never ran.
#[derive(Debug, Clone)]
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
    activation_duration_ms: u32,
    is_playing: Binding<bool>,
    on_event: Option<BoxedEventAction<Event>>,
}

impl core::fmt::Debug for LivePhoto {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("LivePhoto")
            .field("source", &self.source)
            .field("activation_duration_ms", &self.activation_duration_ms)
            .field("is_playing", &self.is_playing)
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
            activation_duration_ms: 250,
            is_playing: Binding::bool(false),
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
    pub const fn activation_duration_ms(mut self, activation_duration_ms: u32) -> Self {
        self.activation_duration_ms = activation_duration_ms;
        self
    }
}

impl View for LivePhoto {
    fn body(self, env: &Environment) -> impl View {
        let source = self.source;
        let activation_duration_ms = self.activation_duration_ms;
        let is_playing = self.is_playing;
        let motion_source = source.clone();
        let still_source = source.map(|source| source.image).computed();
        let playback_state = is_playing.clone();
        let is_playing_on_press = is_playing.clone();
        let report = LivePhotoReporter::new(self.on_event, env.clone());
        let report_on_press = report.clone();
        let motion = Dynamic::watch(is_playing, move |is_playing_now| {
            if is_playing_now {
                AnyView::new(live_photo_video(
                    motion_source.get(),
                    playback_state.clone(),
                    report.clone(),
                ))
            } else {
                AnyView::new(())
            }
        });

        Metadata::new(
            overlay(Photo::new(still_source).resizable(), motion),
            GestureObserver::new(LongPressGesture::new(activation_duration_ms), move || {
                if !is_playing_on_press.get() {
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
    handler: Rc<RefCell<Option<BoxedEventAction<Event>>>>,
    env: Environment,
}

impl LivePhotoReporter {
    fn new(handler: Option<BoxedEventAction<Event>>, env: Environment) -> Self {
        Self {
            handler: Rc::new(RefCell::new(handler)),
            env,
        }
    }

    fn emit(&self, event: Event) {
        if let Some(handler) = self.handler.borrow_mut().as_mut() {
            handler(event, &self.env);
        }
    }
}

fn live_photo_video(
    source: LivePhotoSource,
    is_playing: Binding<bool>,
    report: LivePhotoReporter,
) -> Video {
    let session = PlaybackSession::new(Playlist::single(source.video)).autoplay();
    session.controller().muted().set(true);
    Video::new(session).loops(false).on_event(move |event| {
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
