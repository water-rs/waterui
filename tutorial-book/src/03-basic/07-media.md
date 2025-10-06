# Media Components

Media surfaces are first-class citizens in WaterUI. The `waterui_media` crate provides declarative
views for images (`Photo`), video playback (`Video` + `VideoPlayer`), Live Photos, and a unified
`Media` enum that dynamically chooses the right renderer. This chapter explores the API from basic
usage through advanced configuration.

## Photos: Static Images with Placeholders

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::Photo;

pub fn cover_image() -> impl View {
    Photo::new("https://assets.waterui.dev/cover.png")
        .placeholder(text("Loading…"))
}
```

Key features:

- `Photo::new` accepts anything convertible into `waterui_media::Url` (web URLs, `file://`, etc.).
- `.placeholder(view)` renders while the backend fetches the asset.
- `.on_failure(view)` handles network errors gracefully.
- You can compose standard modifiers (`.padding()`, `.frame(...)`, `.background(...)`) around the
  `Photo` like any other view.

## Video Playback

`Video` represents a source, while `VideoPlayer` renders controls. Create one `Video` per asset and
reuse it if multiple players should point at the same file.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::{Video, VideoPlayer};
use waterui::reactive::binding;

pub fn trailer_player() -> impl View {
    let video = Video::new("https://media.waterui.dev/trailer.mp4");
    let muted = binding(false);

    vstack((
        VideoPlayer::new(video.clone()).muted(&muted),
        button("Toggle Mute").action_with(&muted, |state| state.toggle()),
    ))
}
```

### Muting Model

- `VideoPlayer::muted(&Binding<bool>)` maps a boolean binding onto the player’s internal volume.
- `VideoPlayer` stores the pre-mute volume so toggling restores the last audible level.

### Styling Considerations

The video chrome (play/pause controls) depends on the backend. SwiftUI renders native controls,
whereas Web/Gtk4 use their respective toolkit widgets. Keep platform conventions in mind when
layering overlays or gestures on top.

## Live Photos

Apple’s Live Photos combine a still image and a short video clip. WaterUI packages the pair inside
`LivePhotoSource`:

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::{LivePhoto, LivePhotoSource};

pub fn vacation_memory() -> impl View {
    let source = LivePhotoSource::new(
        "IMG_1024.jpg".into(),
        "IMG_1024.mov".into(),
    );

    LivePhoto::new(source)
}
```

Backends that don’t support Live Photos fall back to the still image.

## The `Media` Enum

When the media type is decided at runtime, wrap it in `Media`. Rendering becomes a single view
binding instead of a large `match` statement.

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::Media;
use waterui::reactive::binding;

pub fn dynamic_media() -> impl View {
    let media = binding(Media::Image("https://example.com/photo.png".into()));

    // Later you can switch to Media::Video or Media::LivePhoto and the UI updates automatically.
    media
}
```

`Media` implements `View`, so you can drop it directly into stacks, grids, or navigation views. To
switch the content, update the binding—WaterUI rebuilds the appropriate concrete view.

## Media Picker (Feature Flag: `media-picker`)

Enable the crate feature in `Cargo.toml`:

```toml
[dependencies.waterui]
features = ["media-picker"]
```

Then present the picker:

```rust,ignore
use waterui::prelude::*;
use waterui::components::media::picker::{MediaFilter, MediaPicker, Selected};
use waterui::reactive::binding;

pub fn choose_photo() -> impl View {
    let selection = binding::<Selected>(Selected(0));

    MediaPicker::new()
        .filter(MediaFilter::Image)
        .selection(selection.clone())
}
```

The `Selected` binding stores an identifier. Use `Selected::load()` asynchronously (via `task`) to
receive the actual `Media` item and pipe it into your view tree.

```rust,ignore
use waterui::components::media::Media;
use waterui::reactive::binding;
use waterui::task::task;

let gallery = binding(Vec::<Media>::new());

button("Import").action_with(&selection, move |selected| {
    let gallery = gallery.clone();
    task(async move {
        let media = selected.get().load().await;
        gallery.push(media);
    });
});
```

## Best Practices

- **Defer heavy processing** – Image decoding and video playback happen in the backend. Avoid
  blocking the UI thread; let the renderer stream data.
- **Provide fallbacks** – Always set `.placeholder` so the UI communicates status during network
  hiccups (future versions of the component will expose explicit failure hooks).
- **Reuse sources** – Clone `Video`/`LivePhotoSource` handles instead of recreating them in every
  recomposition.
- **Respect platform capabilities** – Some backends may not implement Live Photos or media pickers
  yet. Feature-gate your UI or supply alternate paths.

With these components you can build media-heavy experiences—galleries, video players, immersive
feeds—while keeping the code declarative and reactive.
