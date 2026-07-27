use waterui_core::gesture::{GestureObserver, LongPressGesture};
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

/// A live photo widget composed in Rust from photo, video, and gesture primitives.
///
/// Keeping this composition in Rust avoids adding a dedicated live-photo type
/// to every native FFI surface.
#[derive(Debug)]
pub struct LivePhoto {
    source: Computed<LivePhotoSource>,
    activation_duration_ms: u32,
    is_playing: Binding<bool>,
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
        }
    }

    /// Sets the long-press duration required to activate motion playback.
    #[must_use]
    pub const fn activation_duration_ms(mut self, activation_duration_ms: u32) -> Self {
        self.activation_duration_ms = activation_duration_ms;
        self
    }
}

impl View for LivePhoto {
    fn body(self, _env: &Environment) -> impl View {
        let source = self.source;
        let activation_duration_ms = self.activation_duration_ms;
        let is_playing = self.is_playing;
        let motion_source = source.clone();
        let still_source = source.map(|source| source.image).computed();
        let playback_state = is_playing.clone();
        let is_playing_on_press = is_playing.clone();
        let motion = Dynamic::watch(is_playing, move |is_playing_now| {
            if is_playing_now {
                AnyView::new(live_photo_video(
                    motion_source.get(),
                    playback_state.clone(),
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
                }
            }),
        )
    }
}

fn live_photo_video(source: LivePhotoSource, is_playing: Binding<bool>) -> Video {
    let session = PlaybackSession::new(Playlist::single(source.video)).autoplay();
    session.controller().muted().set(true);
    Video::new(session).loops(false).on_event(move |event| {
        if matches!(event, VideoEvent::Ended | VideoEvent::Error { .. }) {
            is_playing.set(false);
        }
    })
}
