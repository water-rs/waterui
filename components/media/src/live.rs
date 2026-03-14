use executor_core::spawn;
use waterui_core::dynamic::Dynamic;
use waterui_core::gesture::{GestureObserver, LongPressGesture};
use waterui_core::{
    AnyView, Binding, Computed, Environment, Metadata, SignalExt, View,
    reactive::signal::IntoComputed,
};

#[cfg(feature = "std")]
use waterkit_haptic::{Haptic, Intensity};

use crate::{
    Url,
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

/// A composed live photo widget built from photo, video, gesture, and haptic primitives.
#[derive(Debug)]
pub struct LivePhoto {
    source: Computed<LivePhotoSource>,
    activation_duration_ms: u32,
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
        let is_playing = Binding::bool(false);
        let source = self.source;
        let activation_duration_ms = self.activation_duration_ms;

        Dynamic::watch(source.zip(&is_playing), move |(source, is_playing_now)| {
            if is_playing_now {
                AnyView::new(live_photo_video(source, &is_playing))
            } else {
                AnyView::new(live_photo_still(
                    source,
                    &is_playing,
                    activation_duration_ms,
                ))
            }
        })
    }
}

fn live_photo_still(
    source: LivePhotoSource,
    is_playing: &Binding<bool>,
    activation_duration_ms: u32,
) -> Metadata<GestureObserver> {
    let is_playing = is_playing.clone();
    Metadata::new(
        Photo::new(source.image),
        GestureObserver::new(LongPressGesture::new(activation_duration_ms)).action(move || {
            trigger_live_photo_feedback();
            is_playing.set(true);
        }),
    )
}

fn live_photo_video(source: LivePhotoSource, is_playing: &Binding<bool>) -> Video {
    let is_playing = is_playing.clone();
    let muted = Binding::bool(true);
    Video::new(source.video)
        .loops(false)
        .muted(&muted)
        .on_event(move |event| {
            if matches!(event, VideoEvent::Ended | VideoEvent::Error { .. }) {
                is_playing.set(false);
            }
        })
}

fn trigger_live_photo_feedback() {
    #[cfg(feature = "std")]
    {
        spawn(async move {
            let _ = Haptic::impact(Intensity::MEDIUM).await;
        })
        .detach();
    }
}
