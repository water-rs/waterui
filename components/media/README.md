# WaterUI Media Components

Media handling and display components for the WaterUI framework.

## Overview

`waterui-media` provides components for displaying and handling various media types including images, videos, and Live Photos. The library offers a unified interface for working with different media formats and provides reactive, configurable components that integrate seamlessly with the WaterUI framework.

## Components

### Photo

Display static images with customizable placeholder views:

```rust
use waterui_media::Photo;
use waterui_core::{Text, AnyView};

// Basic image from URL
let photo = Photo::new("https://example.com/image.jpg");

// With placeholder
let photo_with_placeholder = Photo::new("https://example.com/image.jpg")
    .placeholder(Text::new("Loading..."));

// Placeholder can be any view
let custom_placeholder = Photo::new("path/to/image.png")
    .placeholder(AnyView::new(custom_loading_view()));
```

### Video

Display and control video playback with reactive volume control:

```rust
use waterui_media::{Video, VideoPlayer};
use waterui_core::{binding, Binding};

// Create a video from URL
let video = Video::new("https://example.com/video.mp4");

// The video automatically creates a player when used as a View
let video_view = video.view();

// Or create a player directly with custom configuration
let mut volume = binding(0.7); // 70% volume
let muted = binding(false);

let player = VideoPlayer::new(video)
    .muted(&muted);

// Volume is controlled through the binding system
volume.set(0.3); // Change to 30% volume
muted.set(true); // Mute the video
```

### Live Photo

Display Apple Live Photos with image and video components:

```rust
use waterui_media::{LivePhoto, LivePhotoSource};

// Create a LivePhotoSource with image and video URLs
let source = LivePhotoSource::new(
    "photo.jpg".into(),
    "photo.mov".into()
);

// Create the LivePhoto component
let live_photo = LivePhoto::new(source);

// Can also use reactive data
use waterui_core::compute;
let reactive_source = compute(|| {
    LivePhotoSource::new("dynamic_photo.jpg".into(), "dynamic_video.mov".into())
});
let reactive_live_photo = LivePhoto::new(reactive_source);
```

### Media

Unified media type that automatically chooses the appropriate component:

```rust
use waterui_media::{Media, LivePhotoSource};

// Different media types
let image = Media::Image("https://example.com/photo.jpg".into());
let video = Media::Video("https://example.com/video.mp4".into());
let live_photo = Media::LivePhoto(LivePhotoSource::new(
    "photo.jpg".into(),
    "video.mov".into()
));

// The Media enum automatically renders the appropriate component
// when used as a View - no need to match manually
```

## Media Picker (Feature: `media-picker`)

Platform-native media selection interface:

```rust
#[cfg(feature = "media-picker")]
use waterui_media::picker::{MediaPicker, MediaFilter, Selected};
use waterui_core::{compute, binding};

#[cfg(feature = "media-picker")]
{
    let selection = binding(Selected(0));
    let filter = compute(|| MediaFilter::Image);
    
    let picker = MediaPicker::new()
        .selection(selection.clone())
        .filter(filter);
    
    // Load the selected media asynchronously
    let selected = selection.get();
    let media = selected.load().await;
}
```

### Media Filters

Control what types of media can be selected:

```rust
use waterui_media::picker::MediaFilter;

// Individual types
let images_only = MediaFilter::Image;
let videos_only = MediaFilter::Video;
let live_photos_only = MediaFilter::LivePhoto;

// Combinations
let images_and_videos = MediaFilter::All(vec![
    MediaFilter::Image,
    MediaFilter::Video
]);

let everything_except_live_photos = MediaFilter::Not(vec![
    MediaFilter::LivePhoto
]);

let any_media = MediaFilter::Any(vec![
    MediaFilter::Image,
    MediaFilter::Video,
    MediaFilter::LivePhoto
]);
```

## Reactive Programming

All components integrate with WaterUI's reactive system:

```rust
use waterui_core::{binding, compute};
use waterui_media::{Photo, Video, VideoPlayer};

// Reactive image URL
let image_url = binding("initial.jpg".into());
let photo = Photo::new(image_url.clone());

// Change the image dynamically
image_url.set("new_image.jpg".into());

// Computed values
let video_url = compute(|| {
    format!("video_{}.mp4", get_current_id())
});
let video = Video::new(video_url);

// Reactive volume control
let volume = binding(0.5);
let is_muted = binding(false);
let player = VideoPlayer::new(video).muted(&is_muted);
```

## Volume Control

The video player uses a unique volume system:

```rust
use waterui_core::binding;
use waterui_media::VideoPlayer;

let volume = binding(0.7); // 70% volume
let muted = binding(false);

let player = VideoPlayer::new(video)
    .muted(&muted);

// When muted = true, the volume internally becomes negative (-0.7)
// This preserves the original volume level for when unmuted
// When muted = false, volume returns to positive (0.7)
```

The volume system works as follows:
- Positive values (> 0): Audible volume
- Negative values (< 0): Muted state, but preserves the volume level
- When unmuting, the absolute value is restored

## Async Media Loading

Media picker provides async loading capabilities:

```rust
#[cfg(feature = "media-picker")]
use waterui_media::picker::Selected;

#[cfg(feature = "media-picker")]
async fn load_selected_media(selected: Selected) -> Media {
    // This will asynchronously load the selected media
    selected.load().await
}
```

## Dependencies

- `waterui-core`: Core framework functionality providing the View trait, reactive system, and configuration macros

## Features

- `default = ["media-picker"]`: Includes media picker functionality
- `media-picker`: Enables platform-native media selection interface

## Architecture

The media components are built using WaterUI's configuration pattern:

- **Configurable Components**: Each component (Photo, VideoPlayer, LivePhoto) uses the `configurable!` macro
- **Reactive Data**: All properties can be reactive using `Computed<T>` and `Binding<T>`
- **View Integration**: Components implement the `View` trait for seamless integration

## Examples

### Basic Photo Gallery

```rust
use waterui_media::{Photo, Media};
use waterui_layout::vstack;
use waterui_text::text;
use waterui_core::{binding, View, Environment};

struct PhotoGallery {
    photos: Vec<String>,
    current_index: binding<usize>,
}

impl View for PhotoGallery {
    fn body(self, _env: &Environment) -> impl View {
        vstack([
            // Main photo display
            Photo::new(&self.photos[self.current_index.get()])
                .placeholder(text("Loading...")),
                
            // Photo counter - using text! macro for reactivity
            text!("{} of {}", 
                self.current_index.get() + 1, 
                self.photos.len()
            ),
        ])
    }
}
```

### Video with Custom Controls

```rust
use waterui_media::{Video, VideoPlayer};
use waterui_layout::vstack;
use waterui_core::{Button, binding, View, Environment};

fn video_with_controls(video_url: &str) -> impl View {
    let muted = binding(false);
    let player = VideoPlayer::new(Video::new(video_url))
        .muted(&muted);
    
    vstack([
        player,
        Button::new("Toggle Mute")
            .on_press({
                let muted = muted.clone();
                move |_| muted.set(!muted.get())
            }),
    ])
}
```

### Media Type Switching

```rust
use waterui_media::{Media, LivePhotoSource};
use waterui_core::{compute, binding};

fn dynamic_media_view() -> impl View {
    let media_type = binding("image");
    
    let media = compute(move || {
        match media_type.get().as_str() {
            "image" => Media::Image("photo.jpg".into()),
            "video" => Media::Video("video.mp4".into()),
            "live" => Media::LivePhoto(LivePhotoSource::new(
                "live.jpg".into(),
                "live.mov".into()
            )),
            _ => Media::Image("default.jpg".into()),
        }
    });
    
    media
}
```
