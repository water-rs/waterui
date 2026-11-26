# waterui-media

Media components for `WaterUI` providing reactive photo, video, Live Photo, and media-picker support.

## Overview

`waterui-media` combines local/remote image loading (`Photo`), decoded image presentation (`Image`, re-exported from `waterui-image`), video playback (`Video`, `VideoPlayer`, `PlaybackSession`, and `PlayerController`, re-exported from `waterui-video` behind the `video` feature), Rust-composed `LivePhoto`, and platform-native selection (`MediaPicker`, behind the `media-picker` feature).

## Installation

Add `waterui-media` to your `Cargo.toml`:

```toml
[dependencies]
waterui-media = "0.5"
```

### Cargo Features

| Feature | Default | Effect |
| --- | --- | --- |
| `std` | Yes | Enables standard library support (`waterui-core/std`) |
| `media-picker` | Yes | Enables `MediaPicker`, `MediaFilter`, `PickerStatus`, `SingleSelection`, and `MultipleSelection` via `waterkit-dialog` |
| `video` | No | Re-exports `waterui-video` (`Video`, `VideoPlayer`, `PlaybackSession`, `PlayerController`, `Volume`, `LivePhoto`, and video/Live Photo `Media`/`MediaFilter` variants) |
| `remote` | No | Enables HTTP/HTTPS fetching in `Photo` via `zenwave` (`waterui-url/remote`) |

## Quick Start

```rust
use waterui::prelude::*;
use waterui_media::{Photo, Url, video_player};

fn gallery() -> impl View {
    vstack((
        Photo::new(Url::new("https://assets.waterui.dev/images/logo.png")),
        video_player(Url::new(
            "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
        )),
    ))
}
```

## Core Concepts

### URL Handling

`waterui-media` re-exports `waterui_url::Url` for web URLs, local file paths, `data:` URLs, and `blob:` URLs:

```rust
use waterui_media::Url;

// Compile-time static URL
const REMOTE: Url = Url::new(
    "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
);

// Runtime web URL parsing and local/data constructors
let parsed = Url::parse("https://assets.waterui.dev/images/logo.png").unwrap();
let local = Url::from_file_path("/path/to/video.mp4");
let data_url = Url::from_data("image/png", b"...");
# let _ = (REMOTE, parsed, local, data_url);
```

### Photo Loading Pipeline & Cache

`Photo::new(source)` accepts any `IntoPhotoSource` (a `Url`, reactive `Signal` of `Url`, `&'static str`, or environment-resolved asset handle). On `Appear`, `Photo` runs the following pipeline:

1. Checks the `Environment`'s `PhotoCache` (an LRU cache of decoded `Image`s with a 64 MiB default pixel budget, configurable via `PhotoCache::new` or `PhotoCache::set_max_bytes`).
2. On a cache miss, reads local paths on a background thread (`blocking::unblock`, or `waterkit-fs` on `wasm32`) or streams remote URLs when the `remote` feature is enabled.
3. Decodes into `waterui_image::Image` (streaming progressive chunks for remote sources and `RGBA16F` when HDR pixel data is decoded), caches the final frame, and pushes updates to a persistent `ReactiveImage` surface.
4. Tracks load lifecycle in `.status(&Binding<PhotoStatus>)` (`Loading`, `Ready`, `Failed(Str)`), optionally shows `.placeholder(...)` and `.on_failure(...)` views, and emits `PhotoEvent::Loaded` only after the final decoded frame registers with the scene backend (or `PhotoEvent::Error(String)` if fetching, decoding, or backend registration fails).

### Video Components, Volume, and Mute

Enabling the `video` feature exposes two video views backed by `PlaybackSession` and `PlayerController`:

- `Video` (and the `video::video(source)` helper): raw video surface without transport controls, defaulting to autoplay and `RepeatMode::One` when constructed from a source.
- `VideoPlayer` (and the `video_player(source)` helper): interactive player view with platform controls (`.show_controls(bool)`).

Audio level and mute state are separate controls on `PlayerController`:

- `Volume` is a validated finite `f32` in `0.0..=1.0` (`Volume::SILENT` is `0.0`, `Volume::FULL` is `1.0`, `Volume::default()` is `0.5`). Passing a negative, `> 1.0`, or non-finite value to `Volume::new` triggers a contract violation (panicking in debug builds, clamping to `0.0..=1.0` in release builds with `NaN` becoming `Volume::SILENT`).
- `PlayerController::muted()` returns an independent `Binding<bool>`, and `PlayerController::volume()` returns `Binding<Volume>`. Toggling mute leaves the stored `Volume` intact.

### Media Picker and `MediaFilter`

With `media-picker` enabled, `MediaPicker::new(label, &selection)` binds `Binding<Option<Media>>` for single selection, and `MediaPicker::multiple(label, &selection)` binds `Binding<Vec<Media>>` for multi-selection (with optional `.limit(n)`). The picker loads selected media via `waterkit-dialog` directly into the `Media` binding while updating `.status(&Binding<PickerStatus>)` (`Idle`, `Loading`, `Failed(Str)`).

`MediaFilter` is deliberately a plain single-choice enum (`MediaFilter::Image`, plus `MediaFilter::Video` and `MediaFilter::LivePhoto` with `video`) without combinators, because platform pickers accept one media kind per presentation.

## Examples

### Photo with Status, Placeholder, and Events

```rust
use waterui::prelude::*;
use waterui_media::{Photo, PhotoCache, PhotoEvent, PhotoStatus, Url};

fn avatar(url: Url) -> impl View {
    let status = binding(PhotoStatus::Loading);

    Photo::new(url)
        .resizable()
        .content_mode(waterui_media::video::ContentMode::Fit)
        .status(&status)
        .placeholder(|| text("Loading..."))
        .on_failure(|reason| text(format!("Failed: {reason}")))
        .on_event(|event: PhotoEvent| match event {
            PhotoEvent::Loaded => tracing::debug!("Image registered on screen"),
            PhotoEvent::Error(msg) => tracing::debug!("Failed to load: {msg}"),
            _ => {}
        })
        .with(PhotoCache::new(32 * 1024 * 1024))
}
```

### Video Volume and Mute Control

```rust
use waterui::prelude::*;
use waterui_media::{PlaybackSession, Playlist, Url, VideoPlayer, Volume};

fn player_with_audio_controls(url: Url) -> impl View {
    let session = PlaybackSession::new(Playlist::single(url));
    let controller = session.controller();
    let volume = controller.volume();
    let muted = controller.muted();

    volume.set(Volume::new(0.75));
    muted.set(false);

    vstack((
        VideoPlayer::new(session).show_controls(true),
        button("Mute")
            .action(move || muted.set(!muted.snapshot())),
        button("Half Volume")
            .action(move || volume.set(Volume::new(0.5))),
    ))
}
```

### Raw Video View

```rust
use waterui::prelude::*;
use waterui_media::{
    Url,
    video::{self, ContentMode, VideoEvent},
};

fn raw_background_video() -> impl View {
    video::video(Url::new(
        "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
    ))
    .resizable()
    .content_mode(ContentMode::Fill)
    .on_event(|event: VideoEvent| {
        if let VideoEvent::Error { message } = event {
            tracing::warn!("Video error: {message}");
        }
    })
}
```

### Live Photo Display

```rust
use waterui::prelude::*;
use waterui_media::{
    LivePhoto, Url,
    live::{LivePhotoEvent, LivePhotoSource},
};

fn live_photo_view(still_image: Url, motion_video: Url) -> impl View {
    LivePhoto::new(LivePhotoSource::new(still_image, motion_video))
        .resizable()
        .on_event(|event: LivePhotoEvent| match event {
            LivePhotoEvent::MotionStarted => tracing::debug!("Playing motion"),
            LivePhotoEvent::MotionEnded => tracing::debug!("Motion ended"),
            LivePhotoEvent::MotionFailed(msg) => tracing::warn!("Motion failed: {msg}"),
            _ => {}
        })
}
```

### Media Picker

```rust
use waterui::prelude::*;
use waterui_media::{
    Media, MediaPicker,
    media_picker::{MediaFilter, PickerStatus},
};

fn picker_demo() -> impl View {
    let selection = binding::<Option<Media>>(None);
    let status = binding(PickerStatus::Idle);

    vstack((
        MediaPicker::new("Choose Video", &selection)
            .filter(MediaFilter::Video)
            .status(&status),
        watch(selection, |selected| match selected {
            Some(media) => AnyView::new(media),
            None => AnyView::new(text("No media selected")),
        }),
    ))
}
```

### Unified `Media` View

```rust
use waterui::prelude::*;
use waterui_media::{Media, Url, live::LivePhotoSource};

fn media_items() -> impl View {
    let image = Media::Image(Url::new("https://assets.waterui.dev/images/logo.png"));
    let video = Media::Video(Url::new(
        "https://commondatastorage.googleapis.com/gtv-videos-bucket/sample/BigBuckBunny.mp4",
    ));
    let live = Media::LivePhoto(LivePhotoSource::new(
        Url::new("https://assets.waterui.dev/images/still.jpg"),
        Url::new("https://assets.waterui.dev/videos/motion.mov"),
    ));

    vstack((image, video, live))
}
```

## API Overview

### Components

- `Photo`: Reactive image loader backed by `PhotoCache` and `ReactiveImage`
- `Image`: Decoded pixel image view (re-exported from `waterui-image`)
- `Video`: Raw video surface without built-in controls (`video` feature)
- `VideoPlayer`: Video player with platform controls (`video` feature)
- `LivePhoto`: Rust-composed still photo + long-press muted video (`video` feature)
- `MediaPicker`: Native single/multiple media selection button (`media-picker` feature)

### Key Types

- `Url`: Type-safe URL (`web`, local path, `data:`, `blob:`)
- `Media`: `#[non_exhaustive]` enum (`Image(Url)`, plus `Video(Url)` and `LivePhoto(LivePhotoSource)` under `video`) that implements `View`
- `PhotoStatus` / `PhotoEvent` / `PhotoCache`: Photo load state, registration settlement events, and LRU decoded pixel cache
- `PlaybackSession` / `PlayerController` / `Playlist`: Reactive video playback state and controls (`volume()`, `muted()`, `play()`, `pause()`, `seek()`, etc.)
- `Volume`: Validated `0.0..=1.0` linear playback level (`SILENT`, `FULL`)
- `video::ContentMode` / `video::VideoEvent`: Video gravity mode and playback lifecycle events
- `LivePhotoSource` / `LivePhotoEvent`: Paired still/motion URLs and motion playback events
- `MediaFilter` / `PickerStatus`: Plain single-choice picker filter (`Image`, `Video`, `LivePhoto`) and picker load status

## Platform Implementation

- **Apple (iOS / macOS / tvOS)**: `Photo` decodes in Rust and renders through `ReactiveImage` on the scene backend; `Video` and `VideoPlayer` bridge `AVPlayer` / `AVKit`; `LivePhoto` composes `Photo`, `LongPressGesture`, and `Video` in Rust; `MediaPicker` uses `waterkit-dialog` (`PHPickerViewController`).
- **Other Platforms (Android / Desktop / Web)**: `Photo` decodes in Rust and renders through `ReactiveImage` on the scene backend; `Video` and `VideoPlayer` use the `WaterKit` / GPU-surface player (`video-gpu`); `LivePhoto` uses the same Rust composition; `MediaPicker` uses `waterkit-dialog` (such as Android `PickVisualMedia`).
