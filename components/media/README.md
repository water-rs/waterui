# waterui-media

Media components for WaterUI providing reactive photo, video, and Live Photo display with native platform rendering.

## Overview

`waterui-media` delivers a comprehensive media handling system for the WaterUI framework. It bridges Rust's type safety with platform-native media rendering (AVFoundation on Apple platforms, ExoPlayer on Android) while maintaining WaterUI's reactive programming model. The crate supports static images, video playback with controls, Apple Live Photos, and platform-native media picking.

Key features include reactive volume control with mute state preservation, configurable aspect ratios, event-driven loading states, and seamless integration with WaterUI's environment system. Media components automatically render to native widgets: AVPlayerViewController on iOS, AVPlayerLayer for raw video views, and ExoPlayer with PlayerView on Android.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-media = "0.1.0"
```

## Quick Start

```rust
use waterui_media::{Photo, VideoPlayer, Url};
use waterui_core::prelude::*;

fn main() -> impl View {
    VStack::new((
        // Display a remote photo
        Photo::new(Url::new("https://example.com/image.jpg")),

        // Video player with native controls
        VideoPlayer::new(Url::new("https://example.com/video.mp4"))
            .show_controls(true),
    ))
}
```

## Core Concepts

### URL Handling

The crate uses `waterui_url::Url` for type-safe resource addressing. URLs support web resources, local file paths, data URLs, and blob URLs:

```rust
use waterui_media::Url;

// Compile-time web URLs
const REMOTE: Url = Url::new("https://cdn.example.com/video.mp4");

// Runtime parsing
let local: Url = "/path/to/video.mp4".parse().unwrap();
let data_url = Url::from_data("image/png", image_bytes);
```

### Video Components: Raw vs Player

`waterui-media` provides two distinct video components:

- `Video`: Raw video view using AVPlayerLayer/SurfaceView without controls - ideal for custom UIs
- `VideoPlayer`: Full-featured player with native platform controls (play/pause, seek, fullscreen)

### Volume Control System

Video components use a special volume encoding that preserves the original level when muting:

- Positive values (`> 0`): Audible volume level (0.0-1.0)
- Negative values (`< 0`): Muted state storing the original volume as the absolute value
- When unmuting, the absolute value is restored

This approach eliminates the need for separate mute flags while maintaining volume memory.

### Reactive State Integration

All media components integrate with WaterUI's reactive system via `Binding` and `Computed` signals. Changes to bindings automatically propagate to native platform components through the FFI layer.

## Examples

### Photo with Loading Events

```rust
use waterui_media::{Photo, photo::Event, Url};

let photo = Photo::new(Url::new("https://example.com/large-image.jpg"))
    .on_event(|event| {
        match event {
            Event::Loaded => tracing::debug!("Image loaded successfully"),
            Event::Error(msg) => tracing::debug!("Failed to load: {}", msg),
        }
    });
```

### Reactive Video Volume Control

```rust
use waterui_core::{binding, prelude::*};
use waterui_media::{VideoPlayer, Url};
use waterui_controls::Toggle;

fn volume_demo() -> impl View {
    let muted = binding(false);

    VStack::new((
        VideoPlayer::new(Url::new("https://example.com/video.mp4"))
            .muted(&muted),

        HStack::new((
            Text::new("Mute"),
            Toggle::new(&muted),
        )),
    ))
}
```

### Raw Video View with Custom Controls

```rust
use waterui_media::{Video, AspectRatio, Url};
use waterui_core::binding;

let video = Video::new(Url::new("https://example.com/loop.mp4"))
    .aspect_ratio(AspectRatio::Fill)
    .loops(true)
    .on_event(|event| {
        // Handle buffering, playback errors, etc.
    });
```

### Live Photo Display

```rust
use waterui_media::{LivePhoto, live::LivePhotoSource, Url};

let source = LivePhotoSource::new(
    Url::new("https://example.com/live-photo.jpg"),
    Url::new("https://example.com/live-photo.mov"),
);

let live_photo = LivePhoto::new(source);
```

### Media Picker

```rust
use waterui_core::{binding, prelude::*};
use waterui_media::{MediaPicker, media_picker::{Selected, MediaFilter}, Media};

fn picker_demo() -> impl View {
    let selection = binding::<Option<Selected>>(None);

    VStack::new((
        MediaPicker::new(&selection)
            .filter(MediaFilter::Video)
            .label(Text::new("Choose Video")),

        // Display selected media
        selection.get().map(|sel| {
            // Load media asynchronously and display
            VStack::new(Text::new("Media selected"))
        }),
    ))
}
```

### Unified Media Type

```rust
use waterui_media::{Media, Url};
use waterui_core::prelude::*;

fn display_media(media: Media) -> impl View {
    // Media enum automatically chooses the right component
    match media {
        Media::Image(_) => AnyView::new(Text::new("Displaying Photo")),
        Media::Video(_) => AnyView::new(Text::new("Displaying VideoPlayer")),
        Media::LivePhoto(_) => AnyView::new(Text::new("Displaying LivePhoto")),
    }
}

let image = Media::Image(Url::new("https://example.com/photo.jpg"));
let video = Media::Video(Url::new("https://example.com/clip.mp4"));
```

## API Overview

### Components

- `Photo` - Display static images with event callbacks for load/error states
- `Video` - Raw video view without controls (AVPlayerLayer/SurfaceView)
- `VideoPlayer` - Full-featured video player with native controls
- `LivePhoto` - Apple Live Photo display combining image and video
- `MediaPicker` - Platform-native media selection UI

### Types

- `Url` - Type-safe URL representation (web, local, data, blob)
- `Media` - Unified enum for Image, Video, or LivePhoto
- `LivePhotoSource` - Pairing of image and video URLs for Live Photos
- `AspectRatio` - Video scaling modes: Fit, Fill, Stretch
- `Volume` - f32 type with special encoding for mute state
- `Event` - Photo and video event types (loaded, error, buffering, etc.)
- `MediaFilter` - Filters for media picker (Image, Video, LivePhoto, combinators)
- `Selected` - Selected media item with async loading capability

### Image Processing (image module)

The `Image` type provides async image manipulation:

- `Image::new(mime, data)` - Decode image from raw data on background thread
- `resize()`, `resize_to_fit()`, `resize_to_fill()`, `resize_exact()` - Resize operations
- `rotate()`, `rotate_90()`, `rotate_180()`, `rotate_270()` - Rotation
- `flip_horizontal()`, `flip_vertical()` - Flipping
- `crop()`, `blur()`, `brighten()`, `adjust_contrast()` - Filters
- `grayscale()`, `invert()`, `huerotate()` - Color adjustments
- `encode_png()`, `encode_jpeg()` - Export to bytes
- `url()` - Generate base64 data URL

All processing operations run on background threads via `blocking::unblock` to prevent UI blocking.

## Features

- `default` - Enables `std` feature
- `std` - Standard library support (enables file path handling)

## Platform Implementation

### iOS/macOS/tvOS

- `Photo`: Uses `AsyncImage` with URLSession for loading
- `Video`: Uses `AVPlayerLayer` directly for custom UIs
- `VideoPlayer`: Uses `AVPlayerViewController` (iOS/tvOS) or `AVPlayerView` (macOS)
- `LivePhoto`: Uses `PHLivePhotoView`
- `MediaPicker`: Uses `PHPickerViewController` with `PHImageManager` for loading

### Android

- `Photo`: Uses Coil image loading library
- `Video`: Uses ExoPlayer with SurfaceView
- `VideoPlayer`: Uses ExoPlayer with PlayerView (full controls)
- `LivePhoto`: Maps to Motion Photos via MediaStore
- `MediaPicker`: Uses `ActivityResultContracts.PickVisualMedia` with ContentResolver
