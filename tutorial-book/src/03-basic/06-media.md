# Media Components

WaterUI provides a comprehensive set of media components for displaying images, videos, and Live Photos in your applications. These components are designed to be reactive, configurable, and type-safe.

## Overview

The media components in WaterUI include:

- **Photo**: Display static images with customizable placeholders
- **Video**: Video sources and players with reactive controls
- **VideoPlayer**: Advanced video playback with volume control
- **LivePhoto**: Apple Live Photo display with image and video components
- **MediaPicker**: Platform-native media selection (when available)
- **Media**: Unified enum for different media types

## Photo Component

The `Photo` component displays images from URLs with support for placeholder views while loading.

### Basic Usage

```rust
use waterui::View;
use waterui_media::Photo;
use waterui_text::text;

pub fn photo_example() -> impl View {
    Photo::new("https://example.com/image.jpg")
        .placeholder(text!("Loading image..."))
}
```

### Local Images

```rust
pub fn local_photo() -> impl View {
    Photo::new("assets/photo.jpg")
        .placeholder(
            waterui_layout::stack::vstack((text!("📷"), text!("Loading...")))
        )
}
```

### Responsive Placeholders

You can create sophisticated placeholder views that match your app's design:

```rust
use waterui::{View, ViewExt, background::Background};
use waterui_layout::stack::{vstack, zstack};
use waterui_text::text;
use waterui::component::layout::{Edge, Frame};

pub fn styled_photo() -> impl View {
    Photo::new("https://example.com/large-image.jpg")
        .placeholder(
            zstack((
                // Background color layer
                ().background(Background::color((0.1, 0.1, 0.1))),
                // Loading indicator
                vstack((
                    text!("📸").size(48.0),
                    text!("Loading image...").foreground((0.6, 0.6, 0.6)),
                )),
            ))
            .frame(Frame::new().width(300.0).height(200.0))
        )
}
```

## Video Components

WaterUI provides both `Video` sources and `VideoPlayer` components for video playback.

### Basic Video

```rust
use waterui::View;
use waterui_media::Video;

pub fn basic_video() -> impl View {
    Video::new("https://example.com/video.mp4")
}
```

When a `Video` is used as a view, it automatically creates a `VideoPlayer`.

### Video Player with Controls

For more control over video playback, use `VideoPlayer` directly:

```rust
use waterui::{View, ViewExt};
use waterui_media::{Video, VideoPlayer};
use waterui_text::text;
use waterui::reactive::binding;
use waterui_layout::stack::{vstack, hstack};

pub fn video_with_controls() -> impl View {
    let video = Video::new("assets/demo.mp4");
    let muted = binding(false);
    
    vstack((
        VideoPlayer::new(video).muted(&muted),
        
        // Mute toggle button
        waterui::component::button::button("Toggle Mute")
            .action_with(&muted, |muted| muted.update(|m| !m))
    ))
}
```

### Volume Control System

The video player uses a unique volume system where:

- **Positive values (> 0)**: Audible volume level
- **Negative values (< 0)**: Muted state that preserves the original volume level
- When unmuting, the absolute value is restored

```rust
use waterui::{View};
use waterui_media::{Video, VideoPlayer};
use waterui::reactive::binding;
use waterui_layout::stack::{vstack, hstack};
use waterui_text::text;

pub fn volume_control_example() -> impl View {
    let video = Video::new("video.mp4");
    let muted = binding(false);
    let volume = binding(0.7); // 70% volume
    
    vstack((
        VideoPlayer::new(video).muted(&muted),
            
        // Volume controls
        hstack((
            waterui::component::button::button("🔇")
                .action_with(&muted, |muted| muted.set(true)),
            waterui::component::button::button("🔉")
                .action_with(&muted, |muted| muted.set(false)),
            // Volume internally stored as -0.7 when muted, +0.7 when unmuted
        ))
    ))
}
```

## Live Photos

Live Photos combine a still image with a short video, similar to Apple's Live Photos feature.

### Basic Live Photo

```rust
use waterui::View;
use waterui_media::{LivePhoto, LivePhotoSource};

pub fn live_photo_example() -> impl View {
    let source = LivePhotoSource::new(
        "photo.jpg".into(),
        "photo_video.mov".into()
    );
    
    LivePhoto::new(source)
}
```

### Reactive Live Photo

```rust
use waterui::{View};
use waterui_media::{LivePhoto, LivePhotoSource};
use waterui_text::text;
use waterui_layout::stack::{vstack, hstack};
use waterui::reactive::binding;

pub fn reactive_live_photo() -> impl View {
    let photo_index = binding(0);
    
    let live_source = s!({
        LivePhotoSource::new(
            format!("photo_{}.jpg", photo_index).into(),
            format!("photo_{}.mov", photo_index).into()
        )
    });
    
    vstack((
        LivePhoto::new(live_source),
        
        hstack((
            waterui::component::button::button("Previous")
                .action_with(&photo_index, |photo_index| {
                    photo_index.update(|idx| if idx > 0 { idx - 1 } else { idx })
                }),
            waterui::component::button::button("Next")
                .action_with(&photo_index, |photo_index| photo_index.update(|idx| idx + 1)),
        ))
    ))
}
```

## Unified Media Type

The `Media` enum provides a unified way to handle different types of media content:

```rust
use waterui::View;
use waterui_media::{Media, LivePhotoSource};
use waterui_layout::stack::vstack;

pub fn media_gallery() -> impl View {
    let media_items = vec![
        Media::Image("photo1.jpg".into()),
        Media::Video("video1.mp4".into()),
        Media::LivePhoto(LivePhotoSource::new(
            "live1.jpg".into(),
            "live1.mov".into()
        )),
    ];
    
    // Each Media automatically renders as the appropriate component
    vstack(
        media_items
            .into_iter()
            .map(|media| media.frame(waterui::component::layout::Frame::new().width(300.0).height(200.0)))
            .collect::<Vec<_>>()
    )
}
```

## Media Picker

The `MediaPicker` component provides platform-native media selection when available:

```rust
use waterui::View;
use waterui_media::{MediaPicker, MediaFilter};
use waterui::widget::condition::when;
use waterui_layout::stack::vstack;
use waterui_text::text;
use waterui::reactive::binding;

pub fn photo_picker_example() -> impl View {
    let selected_media = binding(None);
    
    vstack((
        waterui::component::button::button("Select Photo")
            .action(|| { /* open picker */ }),
            
        // Display selected media
        when(waterui::reactive::s!(selected_media.is_some()), || {
            // Simplest approach: show placeholder text when none is selected
            text!("Media selected")
        })
        .or(|| text!("No media selected"))
    ))
}
```

### Media Filters

You can filter the types of media shown in the picker:

```rust
use waterui::media::MediaFilter;

// Only show images
let image_filter = MediaFilter::Image;

// Only show videos
let video_filter = MediaFilter::Video;

// Show images and live photos
let mixed_filter = MediaFilter::Any(vec![
    MediaFilter::Image,
    MediaFilter::LivePhoto
]);

// Show everything except videos
let no_videos = MediaFilter::Not(vec![MediaFilter::Video]);
```

## Practical Example: Media Gallery

Here's a complete example of a media gallery with different types of content:

```rust
use waterui::{View, ViewExt};
use waterui_media::*;
use waterui_layout::stack::{vstack, hstack};
use waterui_text::text;
use waterui::reactive::binding;
use waterui::component::layout::{Edge, Frame};

pub fn media_gallery_app() -> impl View {
    let selected_index = binding(0);
    
    let media_items = vec![
        Media::Image("gallery/sunset.jpg".into()),
        Media::Video("gallery/timelapse.mp4".into()),
        Media::LivePhoto(LivePhotoSource::new(
            "gallery/action.jpg".into(),
            "gallery/action.mov".into()
        )),
        Media::Image("gallery/landscape.jpg".into()),
    ];
    
    let current_media = s!(media_items.get(selected_index).cloned());
    
    vstack((
        // Header
        text!("Media Gallery")
            .size(24.0)
            .frame(Frame::new().margin(Edge::round(16.0))),
            
        // Main media display
        waterui::component::Dynamic::watch(current_media, |media| {
            if let Some(media) = media {
                media
                    .frame(Frame::new().width(600.0).height(400.0))
                    .frame(Frame::new().margin(Edge::round(16.0)))
                    .anyview()
            } else {
                text!("No media available").anyview()
            }
        }),
        
        // Navigation controls
        hstack((
            waterui::component::button::button("◀ Previous")
                .action_with(&selected_index, |selected_index| {
                    selected_index.update(|idx| if idx > 0 { idx - 1 } else { idx })
                }),
                
            text!("{} / {}", 
                s!(selected_index + 1), 
                media_items.len()
            )
            .frame(Frame::new().margin(Edge::horizontal(16.0))),
            
            waterui::component::button::button("Next ▶")
                .action_with(&selected_index, |selected_index| {
                    selected_index.update(|idx| if idx + 1 < media_items.len() { idx + 1 } else { idx })
                })
        ))
        .frame(Frame::new().margin(Edge::round(16.0))),
        
        // Thumbnail strip
        hstack(
            media_items
                .iter()
                .enumerate()
                .map(|(index, media)| {
                    waterui::component::button::button( media.clone().frame(Frame::new().width(80.0).height(60.0)) )
                        .action_with(&selected_index, move |selected_index| selected_index.set(index))
                })
                .collect::<Vec<_>>()
        )
        .frame(Frame::new().margin(Edge::round(16.0)))
    ))
}
```

## Best Practices

### Loading States

Always provide meaningful placeholder views for images:

```rust
// Good: Informative placeholder
Photo::new("large-image.jpg")
    .placeholder(
        vstack((
            text!("📸"),
            text!("Loading high-resolution image...")
        ))
    )

// Avoid: No placeholder (jarring loading experience)
Photo::new("large-image.jpg")
```

### Performance

For large media galleries, consider lazy loading:

```rust
use waterui::View;
use waterui_text::text;
use waterui_layout::stack::vstack;

pub fn efficient_gallery() -> impl View {
    let visible_items = waterui::reactive::Computed::new({
        // Only load visible items based on scroll position
        // Placeholder for example
        Vec::<Media>::new()
    });
    
    // Lazy stacks are experimental; use a regular vstack for now
    vstack(
        visible_items.get().into_iter()
            .map(|media| media.frame(Frame::new().width(300.0).height(200.0)))
            .collect::<Vec<_>>()
    )
}
```

### Responsive Design

Make media components responsive to different screen sizes:

```rust
// Example placeholder: responsive sizing requires reading screen metrics from the backend
```
