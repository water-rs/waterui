//! # Media Picker
//!
//! Platform-native single and multi-item media selection through [`MediaPicker`].
//!
//! This module is compiled only when native picker support is enabled (`media-picker`
//! + `std`): a build without native picker support fails at compile time rather than
//! silently failing on tap.

use alloc::string::ToString;
use alloc::vec::Vec;
use core::future::Future;
use core::num::NonZeroUsize;
#[cfg(feature = "video")]
use std::path::Path;

use futures::stream::{self, StreamExt};
use waterkit_dialog::{DialogError, LoadedMedia, MediaType, PhotoPicker as KitPhotoPicker};
use waterui_controls::{IntoLabel, button};
use waterui_core::reactive::{impl_constant, signal::IntoComputed};
use waterui_core::{Binding, Computed, Environment, Signal, Str, View, contract_violation};

use crate::Media;
#[cfg(feature = "video")]
use crate::live::LivePhotoSource;
use crate::url::Url;

/// Maximum number of selected items loaded concurrently after a multi-select pick.
const MAX_CONCURRENT_MEDIA_LOADS: usize = 4;

/// Lifecycle status of a [`MediaPicker`] presentation and media load.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PickerStatus {
    /// No picker presentation or load is active.
    #[default]
    Idle,
    /// The native picker dialog is open or loading the selected media items.
    Loading,
    /// Presenting the picker or loading the selected media failed.
    Failed(Str),
}

impl PickerStatus {
    /// Returns whether the picker is idle.
    #[must_use]
    pub const fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Returns whether the picker is currently presenting or loading media.
    #[must_use]
    pub const fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// Returns the failure message if the last picker operation failed.
    #[must_use]
    pub const fn error(&self) -> Option<&str> {
        match self {
            Self::Failed(message) => Some(message.as_str()),
            Self::Idle | Self::Loading => None,
        }
    }
}

impl_constant!(PickerStatus);

/// Single-item selection mode for [`MediaPicker::new`].
#[derive(Debug, Clone)]
pub struct SingleSelection {
    selection: Binding<Option<Media>>,
}

/// Multi-item selection mode for [`MediaPicker::multiple`].
#[derive(Debug, Clone)]
pub struct MultipleSelection {
    selection: Binding<Vec<Media>>,
    limit: Option<NonZeroUsize>,
}

/// A media picker view that lets users select one or more photos, videos, or
/// live photos.
///
/// `MediaPicker` renders as a button that, when activated, presents the native
/// platform media picker via `WaterKit` dialog APIs, populates the bound
/// [`Media`] selection directly, and reports progress or errors through an
/// optional [`PickerStatus`] binding.
#[derive(Debug)]
pub struct MediaPicker<Label, Mode = SingleSelection> {
    mode: Mode,
    filter: Computed<MediaFilter>,
    status: Option<Binding<PickerStatus>>,
    label: Label,
}

impl<Label> MediaPicker<Label, SingleSelection>
where
    Label: IntoLabel + 'static,
{
    /// Creates a single-selection `MediaPicker` with the given label and
    /// [`Media`] binding.
    ///
    /// Pre-populating `selection` seeds the initial selection; cancelling or
    /// failing a later pick leaves the existing selection intact.
    #[must_use]
    pub fn new(label: Label, selection: &Binding<Option<Media>>) -> Self {
        Self {
            mode: SingleSelection {
                selection: selection.clone(),
            },
            filter: MediaFilter::Image.into_computed(),
            status: None,
            label,
        }
    }
}

impl<Label> MediaPicker<Label, MultipleSelection>
where
    Label: IntoLabel + 'static,
{
    /// Creates a multi-selection `MediaPicker` with the given label and
    /// `Vec<Media>` binding.
    ///
    /// Use [`.limit(n)`](Self::limit) to cap the number of items the picker
    /// allows, and [`.status(&status)`](Self::status) to observe loading and
    /// failure states.
    #[must_use]
    pub fn multiple(label: Label, selection: &Binding<Vec<Media>>) -> Self {
        Self {
            mode: MultipleSelection {
                selection: selection.clone(),
                limit: None,
            },
            filter: MediaFilter::Image.into_computed(),
            status: None,
            label,
        }
    }

    /// Caps the maximum number of items the user may select.
    ///
    /// # Contract violation
    ///
    /// `limit` must be at least `1`. Passing `0` panics in debug builds and
    /// leaves the selection limit unchanged in release builds.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        if let Some(non_zero) = NonZeroUsize::new(limit) {
            self.mode.limit = Some(non_zero);
        } else {
            contract_violation!(
                "MediaPicker::limit must be at least 1; got 0, leaving limit unchanged"
            );
        }
        self
    }
}

impl<Label, Mode> MediaPicker<Label, Mode>
where
    Label: IntoLabel + 'static,
{
    /// Sets the media filter for this picker.
    #[must_use]
    pub fn filter(mut self, filter: impl IntoComputed<MediaFilter>) -> Self {
        self.filter = filter.into_computed();
        self
    }

    /// Binds a [`PickerStatus`] target that tracks [`PickerStatus::Loading`],
    /// [`PickerStatus::Idle`], and [`PickerStatus::Failed`] transitions.
    #[must_use]
    pub fn status(mut self, status: &Binding<PickerStatus>) -> Self {
        self.status = Some(status.clone());
        self
    }

    /// Replaces the label shown on the picker button.
    #[must_use]
    pub fn label<NewLabel: IntoLabel + 'static>(
        self,
        label: NewLabel,
    ) -> MediaPicker<NewLabel, Mode> {
        MediaPicker {
            mode: self.mode,
            filter: self.filter,
            status: self.status,
            label,
        }
    }
}

trait SelectionMode: Clone + 'static {
    fn execute(
        self,
        filter: MediaFilter,
        status: Option<Binding<PickerStatus>>,
    ) -> impl Future<Output = ()> + 'static;
}

impl SelectionMode for SingleSelection {
    #[expect(
        clippy::future_not_send,
        reason = "runs on the UI local executor and updates `!Send` reactive `Binding`s across the picker await"
    )]
    async fn execute(self, filter: MediaFilter, status: Option<Binding<PickerStatus>>) {
        let picker = KitPhotoPicker::new().with_media_type(media_type_from_filter(filter));
        execute_single_pick(&self.selection, status.as_ref(), || async move {
            let Some(handle) = picker.pick().await? else {
                return Ok(None);
            };
            handle.load_media().await.map(Some)
        })
        .await;
    }
}

impl SelectionMode for MultipleSelection {
    #[expect(
        clippy::future_not_send,
        reason = "runs on the UI local executor and updates `!Send` reactive `Binding`s across the picker await"
    )]
    async fn execute(self, filter: MediaFilter, status: Option<Binding<PickerStatus>>) {
        let picker = KitPhotoPicker::new().with_media_type(media_type_from_filter(filter));
        let limit = self.limit;
        execute_multiple_pick(&self.selection, status.as_ref(), || async move {
            let Some(handles) = picker.pick_multiple(limit).await? else {
                return Ok(None);
            };
            let mut loaded = Vec::with_capacity(handles.len());
            let mut loads = stream::iter(
                handles
                    .into_iter()
                    .map(|handle| async move { handle.load_media().await }),
            )
            .buffered(MAX_CONCURRENT_MEDIA_LOADS);
            while let Some(result) = loads.next().await {
                loaded.push(result?);
            }
            Ok(Some(loaded))
        })
        .await;
    }
}

impl<Label, Mode> View for MediaPicker<Label, Mode>
where
    Label: IntoLabel + 'static,
    Mode: SelectionMode,
{
    fn body(self, _env: &Environment) -> impl View {
        let mode = self.mode;
        let filter = self.filter;
        let status = self.status;

        button(self.label).action_async(move || {
            let mode = mode.clone();
            let filter = filter.clone();
            let status = status.clone();
            async move {
                let requested_filter = filter.snapshot();
                mode.execute(requested_filter, status).await;
            }
        })
    }
}

fn set_status(status: Option<&Binding<PickerStatus>>, value: PickerStatus) {
    if let Some(binding) = status {
        binding.set(value);
    }
}

#[expect(
    clippy::future_not_send,
    reason = "runs on the UI local executor and updates `!Send` reactive `Binding`s across the picker await"
)]
async fn execute_single_pick<F, Fut>(
    selection: &Binding<Option<Media>>,
    status: Option<&Binding<PickerStatus>>,
    pick_and_load: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<LoadedMedia>, DialogError>>,
{
    set_status(status, PickerStatus::Loading);
    match pick_and_load().await {
        Ok(Some(loaded)) => {
            selection.set(Some(media_from_loaded_selection(loaded)));
            set_status(status, PickerStatus::Idle);
        }
        Ok(None) => {
            set_status(status, PickerStatus::Idle);
        }
        Err(error) => {
            let message = error.to_string();
            tracing::warn!("MediaPicker failed: {message}");
            set_status(status, PickerStatus::Failed(Str::from(message)));
        }
    }
}

#[expect(
    clippy::future_not_send,
    reason = "runs on the UI local executor and updates `!Send` reactive `Binding`s across the picker await"
)]
async fn execute_multiple_pick<F, Fut>(
    selection: &Binding<Vec<Media>>,
    status: Option<&Binding<PickerStatus>>,
    pick_and_load: F,
) where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Option<Vec<LoadedMedia>>, DialogError>>,
{
    set_status(status, PickerStatus::Loading);
    match pick_and_load().await {
        Ok(Some(items)) => {
            let media = items.into_iter().map(media_from_loaded_selection).collect();
            selection.set(media);
            set_status(status, PickerStatus::Idle);
        }
        Ok(None) => {
            set_status(status, PickerStatus::Idle);
        }
        Err(error) => {
            let message = error.to_string();
            tracing::warn!("MediaPicker failed: {message}");
            set_status(status, PickerStatus::Failed(Str::from(message)));
        }
    }
}

const fn media_type_from_filter(filter: MediaFilter) -> MediaType {
    match filter {
        MediaFilter::Image => MediaType::Image,
        #[cfg(feature = "video")]
        MediaFilter::Video => MediaType::Video,
        #[cfg(feature = "video")]
        MediaFilter::LivePhoto => MediaType::LivePhoto,
    }
}

fn media_from_loaded_selection(loaded: LoadedMedia) -> Media {
    match loaded {
        LoadedMedia::Image(path) => {
            Media::Image(Url::from_file_path_str(path.to_string_lossy().to_string()))
        }
        #[cfg(feature = "video")]
        LoadedMedia::Video(path) => {
            Media::Video(Url::from_file_path_str(path.to_string_lossy().to_string()))
        }
        #[cfg(feature = "video")]
        LoadedMedia::LivePhoto(live_photo) => {
            let (image, video) = live_photo.into_parts();
            live_photo_from_paths(&image, &video)
        }
        #[cfg(not(feature = "video"))]
        _ => unreachable!("an image-filtered picker cannot load video media"),
    }
}

#[cfg(feature = "video")]
fn live_photo_from_paths(image: &Path, video: &Path) -> Media {
    Media::LivePhoto(LivePhotoSource::new(
        Url::from_file_path_str(image.to_string_lossy().to_string()),
        Url::from_file_path_str(video.to_string_lossy().to_string()),
    ))
}

/// The kind of media a [`MediaPicker`] offers for selection.
///
/// Platform pickers accept exactly one media kind per presentation, so the
/// filter is deliberately a plain choice rather than a boolean algebra: every
/// value here is honored exactly by the native picker, never approximated.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum MediaFilter {
    /// Filter for live photos.
    #[cfg(feature = "video")]
    LivePhoto,
    /// Filter for videos.
    #[cfg(feature = "video")]
    Video,
    /// Filter for images.
    Image,
}

impl_constant!(MediaFilter);

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;
    use core::num::NonZeroUsize;
    use std::path::PathBuf;

    use futures_lite::future::block_on;
    use waterkit_dialog::{DialogError, LoadedMedia, MediaType};
    use waterui_core::{Binding, Signal};

    #[cfg(feature = "video")]
    use super::live_photo_from_paths;
    use super::{
        MediaFilter, MediaPicker, PickerStatus, execute_multiple_pick, execute_single_pick,
        media_type_from_filter,
    };
    #[cfg(feature = "video")]
    use crate::live::LivePhotoSource;
    use crate::{Media, Url};

    #[cfg(feature = "video")]
    #[test]
    fn live_photo_selection_preserves_paired_resources() {
        let image = PathBuf::from("waterui-live-photo.heic");
        let video = PathBuf::from("waterui-live-photo.mov");

        assert_eq!(
            live_photo_from_paths(&image, &video),
            Media::LivePhoto(LivePhotoSource::new(
                Url::from_file_path_str(image.to_string_lossy().to_string()),
                Url::from_file_path_str(video.to_string_lossy().to_string()),
            ))
        );
    }

    #[cfg(feature = "video")]
    #[test]
    fn every_filter_maps_to_its_exact_platform_media_type() {
        assert_eq!(
            media_type_from_filter(MediaFilter::LivePhoto),
            MediaType::LivePhoto
        );
        assert_eq!(media_type_from_filter(MediaFilter::Image), MediaType::Image);
        assert_eq!(media_type_from_filter(MediaFilter::Video), MediaType::Video);
    }

    #[test]
    fn seeded_single_selection_survives_cancel_and_failure() {
        let seeded = Media::Image(Url::from_file_path_str("/tmp/avatar.png"));
        let selection = Binding::new(Some(seeded.clone()));
        let status = Binding::new(PickerStatus::Idle);
        let _picker = MediaPicker::new("Avatar", &selection).status(&status);

        // Cancel preserves the seeded selection and returns to Idle.
        block_on(execute_single_pick(&selection, Some(&status), || async {
            assert_eq!(status.snapshot(), PickerStatus::Loading);
            Ok(None)
        }));
        assert_eq!(selection.snapshot(), Some(seeded.clone()));
        assert_eq!(status.snapshot(), PickerStatus::Idle);

        // Failure records PickerStatus::Failed without clobbering the seeded selection.
        block_on(execute_single_pick(&selection, Some(&status), || async {
            Err(DialogError::Unsupported("picker unavailable".into()))
        }));
        assert_eq!(selection.snapshot(), Some(seeded));
        assert!(
            status
                .snapshot()
                .error()
                .is_some_and(|message| message.contains("picker unavailable")),
            "expected Failed status carrying the dialog error, got {:?}",
            status.snapshot()
        );
    }

    #[test]
    fn multiple_selection_populates_vec_and_respects_limit() {
        let photos = Binding::new(Vec::<Media>::new());
        let status = Binding::new(PickerStatus::Idle);
        let picker = MediaPicker::multiple("Photos", &photos)
            .limit(10)
            .status(&status);
        assert_eq!(picker.mode.limit, NonZeroUsize::new(10));

        block_on(execute_multiple_pick(&photos, Some(&status), || async {
            assert_eq!(status.snapshot(), PickerStatus::Loading);
            Ok(Some(vec![
                LoadedMedia::Image(PathBuf::from("/tmp/one.png")),
                LoadedMedia::Image(PathBuf::from("/tmp/two.png")),
            ]))
        }));

        assert_eq!(status.snapshot(), PickerStatus::Idle);
        assert_eq!(
            photos.snapshot(),
            vec![
                Media::Image(Url::from_file_path_str("/tmp/one.png")),
                Media::Image(Url::from_file_path_str("/tmp/two.png")),
            ]
        );
    }

    #[test]
    fn multiple_selection_reports_failure_status() {
        let existing = vec![Media::Image(Url::from_file_path_str("/tmp/seed.png"))];
        let photos = Binding::new(existing.clone());
        let status = Binding::new(PickerStatus::Idle);

        block_on(execute_multiple_pick(&photos, Some(&status), || async {
            Err(DialogError::Unsupported(
                "denied photo library access".into(),
            ))
        }));

        assert_eq!(photos.snapshot(), existing);
        assert!(
            status
                .snapshot()
                .error()
                .is_some_and(|message| message.contains("denied photo library access")),
            "expected Failed status carrying the error, got {:?}",
            status.snapshot()
        );
    }
}
