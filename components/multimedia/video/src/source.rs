//! Shared media source model for native and self-drawn video realizations.

use std::{num::NonZeroUsize, time::Duration};

use nami::impl_constant;
use uuid::Uuid;

use crate::Url;

/// User-provided metadata associated with a media item.
///
/// This UI-facing value deliberately has no dependency on a playback engine.
/// Native and self-drawn backends translate it into their own media-session
/// representation when resolving a [`MediaItem`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    artwork_url: Option<String>,
    duration: Option<Duration>,
}

impl MediaMetadata {
    /// Creates empty media metadata.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            title: None,
            artist: None,
            album: None,
            artwork_url: None,
            duration: None,
        }
    }

    /// Assigns the media title.
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Assigns the media artist.
    #[must_use]
    pub fn with_artist(mut self, artist: impl Into<String>) -> Self {
        self.artist = Some(artist.into());
        self
    }

    /// Assigns the media album.
    #[must_use]
    pub fn with_album(mut self, album: impl Into<String>) -> Self {
        self.album = Some(album.into());
        self
    }

    /// Assigns the media artwork URL.
    #[must_use]
    pub fn with_artwork_url(mut self, artwork_url: impl Into<String>) -> Self {
        self.artwork_url = Some(artwork_url.into());
        self
    }

    /// Assigns the expected media duration.
    #[must_use]
    pub const fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Returns the media title.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Returns the media artist.
    #[must_use]
    pub fn artist(&self) -> Option<&str> {
        self.artist.as_deref()
    }

    /// Returns the media album.
    #[must_use]
    pub fn album(&self) -> Option<&str> {
        self.album.as_deref()
    }

    /// Returns the media artwork URL.
    #[must_use]
    pub fn artwork_url(&self) -> Option<&str> {
        self.artwork_url.as_deref()
    }

    /// Returns the expected media duration.
    #[must_use]
    pub const fn duration(&self) -> Option<Duration> {
        self.duration
    }
}

/// Transport/delivery strategy for a media item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(i32)]
pub enum Delivery {
    /// Progressive file playback (local file or HTTP object URL).
    #[default]
    Progressive = 0,
    /// HTTP Live Streaming multivariant or media playlist.
    Hls = 1,
    /// MPEG-DASH Media Presentation Description.
    Dash = 2,
}

const DEFAULT_MAXIMUM_LICENSE_RESPONSE_BYTES: NonZeroUsize =
    NonZeroUsize::new(4 * 1024 * 1024).expect("license response bound must be non-zero");
const DEFAULT_LICENSE_RENEWAL_THRESHOLD: Duration = Duration::from_mins(1);

/// Opaque, non-empty persistent-license identity returned by a platform CDM.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OfflineDrmKeySet(Vec<u8>);

impl OfflineDrmKeySet {
    /// Creates an opaque persistent-license identity.
    ///
    /// # Panics
    ///
    /// Panics when `bytes` is empty because an empty identity cannot restore keys.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        assert!(
            !bytes.is_empty(),
            "offline DRM key-set identity must not be empty"
        );
        Self(bytes)
    }

    /// Returns the platform-owned key-set bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Per-item platform-CDM network configuration.
///
/// The player obtains opaque challenges from the platform CDM and sends them
/// through its configured license server. Clear media does not use this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrmConfiguration {
    license_url: Option<Url>,
    provisioning_url: Option<Url>,
    request_headers: Vec<(String, String)>,
    maximum_response_bytes: NonZeroUsize,
    renewal_threshold: Duration,
    offline_key_set: Option<OfflineDrmKeySet>,
}

impl DrmConfiguration {
    /// Creates a configuration that follows platform-provided service URLs.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            license_url: None,
            provisioning_url: None,
            request_headers: Vec::new(),
            maximum_response_bytes: DEFAULT_MAXIMUM_LICENSE_RESPONSE_BYTES,
            renewal_threshold: DEFAULT_LICENSE_RENEWAL_THRESHOLD,
            offline_key_set: None,
        }
    }

    /// Overrides the license endpoint returned by the platform CDM.
    #[must_use]
    pub fn license_url(mut self, url: impl Into<Url>) -> Self {
        self.license_url = Some(url.into());
        self
    }

    /// Overrides the device-provisioning endpoint returned by the platform CDM.
    #[must_use]
    pub fn provisioning_url(mut self, url: impl Into<Url>) -> Self {
        self.provisioning_url = Some(url.into());
        self
    }

    /// Adds an HTTP header to provisioning and license exchanges.
    ///
    /// Header syntax is validated by the selected network implementation when
    /// a challenge is created, before any request is sent.
    #[must_use]
    pub fn request_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.request_headers.push((name.into(), value.into()));
        self
    }

    /// Bounds every provisioning or license response body.
    #[must_use]
    pub const fn maximum_response_bytes(mut self, bytes: NonZeroUsize) -> Self {
        self.maximum_response_bytes = bytes;
        self
    }

    /// Sets how early finite streaming keys are renewed before expiration.
    #[must_use]
    pub const fn renewal_threshold(mut self, threshold: Duration) -> Self {
        self.renewal_threshold = threshold;
        self
    }

    /// Restores a previously acquired persistent license for offline playback.
    #[must_use]
    pub fn offline_key_set(mut self, key_set: OfflineDrmKeySet) -> Self {
        self.offline_key_set = Some(key_set);
        self
    }

    /// Returns the explicit license endpoint, if present.
    #[must_use]
    pub const fn resolved_license_url(&self) -> Option<&Url> {
        self.license_url.as_ref()
    }

    /// Returns the explicit provisioning endpoint, if present.
    #[must_use]
    pub const fn resolved_provisioning_url(&self) -> Option<&Url> {
        self.provisioning_url.as_ref()
    }

    /// Returns request headers in insertion order.
    #[must_use]
    pub fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    /// Returns the maximum accepted response size.
    #[must_use]
    pub const fn resolved_maximum_response_bytes(&self) -> NonZeroUsize {
        self.maximum_response_bytes
    }

    /// Returns the proactive streaming-key renewal threshold.
    #[must_use]
    pub const fn resolved_renewal_threshold(&self) -> Duration {
        self.renewal_threshold
    }

    /// Returns the persistent key-set identity selected for offline playback.
    #[must_use]
    pub const fn resolved_offline_key_set(&self) -> Option<&OfflineDrmKeySet> {
        self.offline_key_set.as_ref()
    }
}

impl Default for DrmConfiguration {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod drm_tests {
    use std::{num::NonZeroUsize, time::Duration};

    use super::{DrmConfiguration, OfflineDrmKeySet};
    use crate::Url;

    #[test]
    fn defaults_bound_responses_and_renew_before_expiry() {
        let drm = DrmConfiguration::new();

        assert_eq!(
            drm.resolved_maximum_response_bytes(),
            NonZeroUsize::new(4 * 1024 * 1024).expect("fixture bound must be non-zero")
        );
        assert_eq!(drm.resolved_renewal_threshold(), Duration::from_mins(1));
        assert!(drm.resolved_license_url().is_none());
        assert!(drm.resolved_provisioning_url().is_none());
        assert!(drm.resolved_offline_key_set().is_none());
    }

    #[test]
    fn builder_preserves_per_item_network_policy() {
        let license = Url::parse("https://license.widevine.com/cenc/getcontentkey/widevine_test")
            .expect("Google Widevine test license URL must remain valid");
        let drm = DrmConfiguration::new()
            .license_url(license.clone())
            .request_header("X-Playback-Session", "waterui-test")
            .maximum_response_bytes(
                NonZeroUsize::new(1024).expect("fixture bound must be non-zero"),
            )
            .renewal_threshold(Duration::from_secs(15))
            .offline_key_set(OfflineDrmKeySet::new([1, 2, 3, 4]));

        assert_eq!(drm.resolved_license_url(), Some(&license));
        assert_eq!(
            drm.request_headers(),
            &[(
                String::from("X-Playback-Session"),
                String::from("waterui-test")
            )]
        );
        assert_eq!(drm.resolved_renewal_threshold(), Duration::from_secs(15));
        assert_eq!(
            drm.resolved_offline_key_set()
                .expect("offline key set must be preserved")
                .as_bytes(),
            [1, 2, 3, 4]
        );
    }

    #[test]
    #[should_panic(expected = "offline DRM key-set identity must not be empty")]
    fn offline_key_set_rejects_empty_identity() {
        let _ = OfflineDrmKeySet::new(Vec::new());
    }
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
    /// Stable identity used by playlists and item-transition events.
    pub id: MediaItemId,
    /// The primary video source.
    pub source: Url,
    /// Transport strategy used to load the item.
    pub delivery: Delivery,
    /// Sidecar subtitle tracks associated with this item.
    pub subtitle_tracks: Vec<SubtitleTrack>,
    /// User-provided metadata used for system media sessions and now playing UI.
    pub metadata: MediaMetadata,
    /// Platform-CDM request policy for protected content.
    pub drm: Option<DrmConfiguration>,
}

impl_constant!(MediaItem);

impl MediaItem {
    /// Creates a media item with an explicit transport strategy.
    pub fn new(source: impl Into<Url>, delivery: Delivery) -> Self {
        Self {
            id: MediaItemId::new(),
            source: source.into(),
            delivery,
            subtitle_tracks: Vec::new(),
            metadata: MediaMetadata::new(),
            drm: None,
        }
    }

    /// Replaces the stable playlist identity.
    #[must_use]
    pub const fn id(mut self, id: MediaItemId) -> Self {
        self.id = id;
        self
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

    /// Enables protected playback for this item.
    #[must_use]
    pub fn drm(mut self, configuration: DrmConfiguration) -> Self {
        self.drm = Some(configuration);
        self
    }

    /// Replaces the delivery strategy.
    #[must_use]
    pub const fn delivery(mut self, delivery: Delivery) -> Self {
        self.delivery = delivery;
        self
    }
}

/// Stable identity of one media item across playlist mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaItemId(Uuid);

impl MediaItemId {
    /// Creates a new globally unique media-item identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates an identity from UUID bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(Uuid::from_bytes(bytes))
    }

    /// Returns the canonical UUID bytes.
    #[must_use]
    pub const fn into_bytes(self) -> [u8; 16] {
        self.0.into_bytes()
    }
}

impl Default for MediaItemId {
    fn default() -> Self {
        Self::new()
    }
}

impl From<Url> for MediaItem {
    fn from(value: Url) -> Self {
        Self::new(value, Delivery::Progressive)
    }
}

impl From<&'static str> for MediaItem {
    fn from(value: &'static str) -> Self {
        Self::new(value, Delivery::Progressive)
    }
}

// Deliberately no `From<String>` / `From<Cow<str>>`: those routed through
// `Url`'s deleted silent-degrade conversions. Runtime strings must parse into
// a `Url` explicitly (or use `Url::from_file_path_str` for paths) so a
// malformed address fails at the call site, not inside the player.

#[cfg(test)]
mod tests {
    use super::{Delivery, MediaItem, MediaMetadata, SubtitleTrack};

    #[test]
    fn media_item_defaults_to_progressive_delivery() {
        let item = MediaItem::from(
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
        );
        assert_eq!(item.delivery, Delivery::Progressive);
        assert_eq!(
            item.source.as_str(),
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4"
        );
        assert!(item.subtitle_tracks.is_empty());
        assert_eq!(item.metadata, MediaMetadata::new());
    }

    #[test]
    fn media_item_tracks_sidecar_subtitles() {
        let item = MediaItem::from(
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
        )
        .subtitle_track(
            SubtitleTrack::new("https://waterui.dev/subtitles/en.vtt")
                .label("English")
                .language("en"),
        );

        assert_eq!(item.subtitle_tracks.len(), 1);
        let track = &item.subtitle_tracks[0];
        assert_eq!(
            track.source.as_str(),
            "https://waterui.dev/subtitles/en.vtt"
        );
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
            .with_artwork_url("https://waterui.dev/poster.png");
        let item = MediaItem::from(
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
        )
        .metadata(metadata.clone());

        assert_eq!(item.metadata, metadata);
    }

    #[test]
    fn adaptive_delivery_is_explicit() {
        let hls = MediaItem::new(
            "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_ts/master.m3u8",
            Delivery::Hls,
        );
        let dash = MediaItem::new(
            "https://dash.akamaized.net/akamai/bbb_30fps/bbb_30fps.mpd",
            Delivery::Dash,
        );

        assert_eq!(hls.delivery, Delivery::Hls);
        assert_eq!(dash.delivery, Delivery::Dash);
    }
}
