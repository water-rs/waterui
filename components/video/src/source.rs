//! Shared media source model for the self-drawn video player.

use std::borrow::Cow;

use crate::Url;
use waterkit_audio::MediaMetadata;

/// Transport/delivery strategy for a media item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Delivery {
    /// Progressive file playback (local file or HTTP object URL).
    #[default]
    Progressive,
}

/// Sidecar subtitle track associated with a media item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleTrack {
    /// Location of the subtitle asset.
    pub source: Url,
    /// Human-readable label shown in track pickers.
    pub label: Option<String>,
    /// Optional BCP-47 language tag.
    pub language: Option<String>,
    /// `true` when the track is forced narrative text.
    pub forced: bool,
}

impl SubtitleTrack {
    /// Creates a new subtitle track.
    pub fn new(source: impl Into<Url>) -> Self {
        Self {
            source: source.into(),
            label: None,
            language: None,
            forced: false,
        }
    }

    /// Assigns a display label to the track.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Assigns a language tag to the track.
    #[must_use]
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Marks this track as forced subtitles.
    #[must_use]
    pub const fn forced(mut self, forced: bool) -> Self {
        self.forced = forced;
        self
    }
}

/// Canonical description of one playable media item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaItem {
    /// The primary video source.
    pub source: Url,
    /// Transport strategy used to load the item.
    pub delivery: Delivery,
    /// Sidecar subtitle tracks associated with this item.
    pub subtitle_tracks: Vec<SubtitleTrack>,
    /// User-provided metadata used for system media sessions and now playing UI.
    pub metadata: MediaMetadata,
}

impl MediaItem {
    /// Creates a new progressive media item.
    pub fn new(source: impl Into<Url>) -> Self {
        Self {
            source: source.into(),
            delivery: Delivery::Progressive,
            subtitle_tracks: Vec::new(),
            metadata: MediaMetadata::new(),
        }
    }

    /// Adds a subtitle track to the item.
    #[must_use]
    pub fn subtitle_track(mut self, track: SubtitleTrack) -> Self {
        self.subtitle_tracks.push(track);
        self
    }

    /// Replaces the item metadata.
    #[must_use]
    pub fn metadata(mut self, metadata: MediaMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Replaces the delivery strategy.
    #[must_use]
    pub const fn delivery(mut self, delivery: Delivery) -> Self {
        self.delivery = delivery;
        self
    }
}

impl From<Url> for MediaItem {
    fn from(value: Url) -> Self {
        Self::new(value)
    }
}

impl From<&'static str> for MediaItem {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl From<String> for MediaItem {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl<'a> From<Cow<'a, str>> for MediaItem {
    fn from(value: Cow<'a, str>) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{Delivery, MediaItem, SubtitleTrack};
    use waterkit_audio::MediaMetadata;

    #[test]
    fn media_item_defaults_to_progressive_delivery() {
        let item = MediaItem::from("https://example.com/video.mp4");
        assert_eq!(item.delivery, Delivery::Progressive);
        assert_eq!(item.source.as_str(), "https://example.com/video.mp4");
        assert!(item.subtitle_tracks.is_empty());
        assert_eq!(item.metadata, MediaMetadata::new());
    }

    #[test]
    fn media_item_tracks_sidecar_subtitles() {
        let item = MediaItem::from("https://example.com/video.mp4").subtitle_track(
            SubtitleTrack::new("https://example.com/subs/en.vtt")
                .label("English")
                .language("en"),
        );

        assert_eq!(item.subtitle_tracks.len(), 1);
        let track = &item.subtitle_tracks[0];
        assert_eq!(track.source.as_str(), "https://example.com/subs/en.vtt");
        assert_eq!(track.label.as_deref(), Some("English"));
        assert_eq!(track.language.as_deref(), Some("en"));
        assert!(!track.forced);
    }

    #[test]
    fn media_item_preserves_user_metadata() {
        let metadata = MediaMetadata::new()
            .with_title("Trailer")
            .with_artist("WaterUI")
            .with_album("Demo")
            .with_artwork_url("https://example.com/poster.png");
        let item = MediaItem::from("https://example.com/video.mp4").metadata(metadata.clone());

        assert_eq!(item.metadata, metadata);
    }
}
