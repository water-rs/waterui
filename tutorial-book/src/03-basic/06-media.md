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
use waterui::*;

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
            vstack((
                text!("📷"),
                text!("Loading...")
            ))
        )
}
```

### Responsive Placeholders

You can create sophisticated placeholder views that match your app's design:

```rust
use waterui::*;

pub fn styled_photo() -> impl View {
    Photo::new("https://example.com/large-image.jpg")
        .placeholder(
            zstack((
                // Background
                Rectangle::new()
                    .fill(Color::gray(0.1)),
                // Loading indicator
                vstack((
                    text!("📸")
                        .font_size(48.0),
                    text!("Loading image...")
                        .foreground_color(Color::gray(0.6))
                ))
            ))
            .frame(width: 300.0, height: 200.0)
        )
}
```

## Video Components

WaterUI provides both `Video` sources and `VideoPlayer` components for video playback.

### Basic Video

```rust
use waterui::*;

pub fn basic_video() -> impl View {
    Video::new("https://example.com/video.mp4")
}
```

When a `Video` is used as a view, it automatically creates a `VideoPlayer`.

### Video Player with Controls

For more control over video playback, use `VideoPlayer` directly:

```rust
use waterui::*;

pub fn video_with_controls() -> impl View {
    let video = Video::new("assets/demo.mp4");
    let muted = binding(false);
    
    vstack((
        VideoPlayer::new(video)
            .muted(&muted),
        
        // Mute toggle button
        Button::new(text!("Toggle Mute"))
            .on_click(move || {
                muted.update(|m| !m);
            })
    ))
}
```

### Volume Control System

The video player uses a unique volume system where:

- **Positive values (> 0)**: Audible volume level
- **Negative values (< 0)**: Muted state that preserves the original volume level
- When unmuting, the absolute value is restored

```rust
use waterui::*;

pub fn volume_control_example() -> impl View {
    let video = Video::new("video.mp4");
    let muted = binding(false);
    let volume = binding(0.7); // 70% volume
    
    vstack((
        VideoPlayer::new(video)
            .muted(&muted),
            
        // Volume controls
        hstack((
            Button::new(text!("🔇"))
                .on_click(move || muted.set(true)),
            Button::new(text!("🔉"))
                .on_click(move || muted.set(false)),
            // Volume internally stored as -0.7 when muted, +0.7 when unmuted
        ))
    ))
}
```

## Live Photos

Live Photos combine a still image with a short video, similar to Apple's Live Photos feature.

### Basic Live Photo

```rust
use waterui::*;
use waterui::media::{LivePhoto, LivePhotoSource};

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
use waterui::*;
use waterui::media::{LivePhoto, LivePhotoSource};

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
            Button::new(text!("Previous"))
                .on_click(move || {
                    photo_index.update(|idx| if idx > 0 { idx - 1 } else { idx });
                }),
            Button::new(text!("Next"))
                .on_click(move || {
                    photo_index.update(|idx| idx + 1);
                })
        ))
    ))
}
```

## Unified Media Type

The `Media` enum provides a unified way to handle different types of media content:

```rust
use waterui::*;
use waterui::media::Media;

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
            .map(|media| media.frame(width: 300.0, height: 200.0))
            .collect::<Vec<_>>()
    )
}
```

## Media Picker

The `MediaPicker` component provides platform-native media selection when available:

```rust
use waterui::*;
use waterui::media::{MediaPicker, MediaFilter};

pub fn photo_picker_example() -> impl View {
    let selected_media = binding(None);
    
    vstack((
        Button::new(text!("Select Photo"))
            .on_click(move || {
                // Platform-native photo picker would open
            }),
            
        // Display selected media
        when(selected_media.clone(), |media| if let Some(media) = media {
            media.into_view()
        } else {
            text!("No media selected").into_view()
        })
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
use waterui::*;
use waterui::media::*;

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
            .font_size(24.0)
            .padding(16.0),
            
        // Main media display
        when(current_media.clone(), |media| if let Some(media) = media {
            media
                .frame(width: 600.0, height: 400.0)
                .padding(16.0)
                .into_view()
        } else {
            text!("No media available").into_view()
        }),
        
        // Navigation controls
        hstack((
            Button::new(text!("◀ Previous"))
                .on_click(move || {
                    selected_index.update(|idx| if idx > 0 { idx - 1 } else { idx });
                }),
                
            text!("{} / {}", 
                s!(selected_index + 1), 
                media_items.len()
            )
            .padding_horizontal(16.0),
            
            Button::new(text!("Next ▶"))
                .on_click(move || {
                    selected_index.update(|idx| {
                        if idx < media_items.len() - 1 { idx + 1 } else { idx }
                    });
                })
        ))
        .padding(16.0),
        
        // Thumbnail strip
        hstack(
            media_items
                .iter()
                .enumerate()
                .map(|(index, media)| {
                    Button::new(
                        media.clone()
                            .frame(width: 80.0, height: 60.0)
                    )
                    .on_click(move || selected_index.set(index))
                })
                .collect::<Vec<_>>()
        )
        .padding(16.0)
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
use waterui::*;

pub fn efficient_gallery() -> impl View {
    let visible_items = computed(|| {
        // Only load visible items based on scroll position
        get_visible_media_items()
    });
    
    LazyVStack::new(
        visible_items,
        |media| media.frame(width: 300.0, height: 200.0)
    )
}
```

### Responsive Design

Make media components responsive to different screen sizes:

```rust
pub fn responsive_photo() -> impl View {
    Photo::new("responsive-image.jpg")
        .frame(
            width: env.screen_width * 0.9,
            height: env.screen_width * 0.6
        )
        .placeholder(
            Rectangle::new()
                .fill(Color::gray(0.1))
                .frame(
                    width: env.screen_width * 0.9,
                    height: env.screen_width * 0.6
                )
        )
}
```

## Platform Considerations

### File Formats

Different platforms support different media formats:

- **Images**: JPEG, PNG, WebP (web), HEIC (iOS)
- **Videos**: MP4, MOV, WebM (web)
- **Live Photos**: Platform-specific implementations

### URLs and Paths

- Use proper URL schemes: `https://`, `file://`, `assets://`
- Handle network connectivity for remote media
- Provide fallbacks for unsupported formats

## Next Steps

In the next chapter, we'll explore [Navigation and Routing](12-navigation.md) to learn how to create multi-screen applications with proper navigation patterns.

The media components integrate seamlessly with WaterUI's reactive system, making it easy to build rich, interactive media experiences in your applications.