# Media Components

WaterUI provides a set of components for displaying images, videos, and other media in your applications. These components are designed to be flexible, reactive, and easy to use.

## `Photo`

The `Photo` component is used to display static images. You can create a `Photo` from a URL or a local file path.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::Photo;

fn my_image() -> impl View {
    Photo::new("https://www.example.com/my-image.png")
}
```

You can also provide a placeholder view to be displayed while the image is loading.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::Photo;

fn image_with_placeholder() -> impl View {
    Photo::new("https://www.example.com/my-image.png")
        .placeholder(text("Loading..."))
}
```

## `Video`

The `Video` component is used to display and control video playback. You can create a `Video` from a URL or a local file path.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::{Video, VideoPlayer};

fn my_video() -> impl View {
    let video = Video::new("https://www.example.com/my-video.mp4");
    VideoPlayer::new(video)
}
```

You can control the video playback using a `Binding`.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::{Video, VideoPlayer};

fn controllable_video() -> impl View {
    let video = Video::new("https://www.example.com/my-video.mp4");
    let muted = binding(false);

    vstack((
        VideoPlayer::new(video).muted(muted),
        button("Toggle Mute", move || {
            muted.toggle();
        })
    ))
}
```

## `LivePhoto`

The `LivePhoto` component is used to display Apple Live Photos. A Live Photo consists of a still image and a short video clip.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::{LivePhoto, LivePhotoSource};

fn my_live_photo() -> impl View {
    let source = LivePhotoSource::new(
        "my-live-photo.jpg".into(),
        "my-live-photo.mov".into()
    );
    LivePhoto::new(source)
}
```

## `Media` Enum

The `Media` enum is a convenient way to display different types of media. It can hold a `Photo`, a `Video`, or a `LivePhoto`.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::Media;

fn my_media() -> impl View {
    let media = binding(Media::Image("https://www.example.com/my-image.png".into()));

    // ...
}
```

## `MediaPicker`

The `MediaPicker` component allows users to select media from their device's library. This feature is only available when the `media-picker` feature is enabled.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::picker::{MediaPicker, MediaFilter};

fn my_media_picker() -> impl View {
    let selection = binding(None);

    MediaPicker::new()
        .filter(MediaFilter::Image)
        .selection(selection)
}
```

This will present a platform-native media picker that allows the user to select an image. The `selection` binding will be updated with the selected media.