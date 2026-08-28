//! Snippets from `.claude/skills/waterui/references/media.md`, in file order.
//! Transcription conventions are documented in the crate README.

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 1/13
// An import pair, then two independent constructors.
// ---------------------------------------------------------------------------
pub fn media_block_01() {
    let radius = Binding::f64(2.0);
    let sat = Binding::f64(1.0);
    let rgba_pixels = vec![0u8; 200 * 150 * 4];

    use waterui::media::photo::Event as PhotoEvent; // the event type needs this alias import
    use waterui::media::{Image, Photo, Url};

    let _ = {
        Photo::new("https://waterui.dev/logo.png") // impl IntoComputed<Url>; Url: From<&'static str>
            .on_event(|event| match event {
                // inherent on Photo — attach it BEFORE filters,
                PhotoEvent::Loaded => (), // which wrap the view in a Filtered<..> type
                PhotoEvent::Error(_msg) => (),
            })
            .blur(radius.clone())
            .saturation(sat.clone())
    };

    let _ = {
        Image::new(rgba_pixels, 200, 150) // in-memory RGBA8; asserts len == w * h * 4
    };

    let _: Option<Url> = None;
}

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 2/13
// ---------------------------------------------------------------------------
pub fn media_block_02() {
    use waterui::media::{Photo, Url};

    let entered = Binding::container(Str::from("https://waterui.dev/logo.png"));
    let (photo_slot, _view) = Dynamic::new();

    let Some(parsed) = Url::parse(entered.get().as_str()) else {
        return;
    }; // parse returns Option
    photo_slot.set(Photo::new(parsed));
}

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 3/13
// A statement sequence, then a handler-side line with its own receiver.
// ---------------------------------------------------------------------------
pub fn media_block_03() {
    use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};

    let selection: Binding<Option<Selected>> = Binding::default();
    let _ = {
        MediaPicker::new(&selection)
            .filter(MediaFilter::Image) // Image | Video | LivePhoto; takes a signal
            .label(text("Pick a photo")) // type-changing builder — call before storing
    };

    // The remaining filter variants named in the trailing comment.
    let _ = (MediaFilter::Video, MediaFilter::LivePhoto);
}

/// The handler-side half of block 3, which needs a `Selected` value in hand.
pub fn media_block_03_handler(selected: waterui::media::media_picker::Selected) {
    // In a handler: Selected::load() is synchronous and consumes the selection.
    let media = selected.load(); // Media::Image(Url) | Video(Url) | LivePhoto(source)

    let _ = media;
}

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 4/13
// ---------------------------------------------------------------------------
pub fn media_block_04() -> impl View {
    use waterui::video::{self, MediaItem, PlaybackSession, Playlist, VideoPlayer};

    let first_item = MediaItem::from(
        waterui::video::Url::parse("https://waterui.dev/a.mp4").expect("valid url"),
    );
    let more_items: Vec<MediaItem> = Vec::new();

    let playlist = Playlist::new(first_item, more_items); // deliberately non-empty
    let session = PlaybackSession::new(playlist).autoplay();
    let controller = session.controller(); // Clone; capture before the move

    let _ = controller;

    VideoPlayer::new(session)
        .show_controls(true)
        .content_mode(video::ContentMode::Fit) // Fit | Fill | Stretch
        .on_event(|event| match event {
            video::Event::ReadyToPlay | video::Event::Ended => (),
            video::Event::Error { message: _ } => (),
            _ => (), // Buffering, PlaybackStateChanged, …
        })
}

// ---------------------------------------------------------------------------
// media.md § "## Media" (prose): `MediaItem::from(url)` exposes a `Copy`
// `id: MediaItemId`; `PlayerController` readers, writable bindings via methods,
// and commands. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_controller_prose() {
    use waterui::video::{MediaItem, MediaItemId, PlaybackSession, Playlist};

    let item = MediaItem::from(
        waterui::video::Url::parse("https://waterui.dev/a.mp4").expect("valid url"),
    );
    let id: MediaItemId = item.id;
    let session = PlaybackSession::new(Playlist::new(item, Vec::new()));
    let controller = session.controller();

    let _: Computed<_> = controller.position();
    let _: Computed<_> = controller.duration();
    let _: Computed<_> = controller.current_item_index();

    let _ = controller.volume();
    let _ = controller.muted();
    let _ = controller.playback_rate();
    let _ = controller.repeat_mode();

    let _ = controller.seek_to_item(id);
    let _ = controller.next();
    let _ = controller.previous();
    controller.play();
    controller.pause();
    controller.stop();
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" — rust block 5/13
// ---------------------------------------------------------------------------
pub fn media_block_05() -> impl View {
    let allow = Binding::bool(true);
    let ua = Binding::container(Str::from("WaterUI"));

    use waterui::webview::{ScriptInjectionTime, Url, WebView, WebViewEvent, WebViewProxy};

    let _: Option<Url> = None;
    let _: Option<WebViewProxy> = None;

    WebView::open("https://waterui.dev")
        .redirects_enabled(allow.clone())
        .user_agent(ua.clone())
        .inject(
            "marker",
            "document.body.dataset.app = 'waterui';",
            ScriptInjectionTime::DocumentEnd,
        )
        .on_event(|event| match event {
            WebViewEvent::WillNavigate { url: _ } => (),
            WebViewEvent::Loading { progress: _ } => (), // f32
            WebViewEvent::Loaded => (),
            WebViewEvent::Redirect { from: _, to: _ } => (),
            WebViewEvent::Error(_) => (),
        })
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" — rust block 6/13
// ---------------------------------------------------------------------------
pub fn media_block_06() -> impl View {
    use waterui::webview::{Url, WebView, WebViewProxy};

    let open = WebView::open("https://waterui.dev");
    let address = Binding::container(Str::from("waterui.dev"));

    open.with_proxy(move || {
        hstack((
            button("Back").action(|proxy: WebViewProxy| proxy.go_back()),
            button("Go")
                .action(|proxy: WebViewProxy, State(addr): State<Binding<Str>>| {
                    // parse_user_input tolerates human input (missing scheme); returns Option.
                    if let Some(url) = Url::parse_user_input(addr.get().as_str()) {
                        proxy.go_to(url);
                    }
                })
                .state(&address),
        ))
    })
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" — rust block 7/13
// ---------------------------------------------------------------------------
pub mod media_block_07 {
    use waterui::prelude::*;

    /// Glue: the payload type the `greet` method returns.
    #[derive(serde::Serialize)]
    pub struct Greeting {
        message: String,
    }
    impl Greeting {
        fn for_name(name: String) -> Self {
            Self {
                message: format!("Hello, {name}"),
            }
        }
    }

    use waterui::js_api;
    use waterui::webview::Json;

    pub struct PageApi {
        address: Binding<Str>,
        greetings: Binding<u32>,
    }

    #[js_api]
    impl PageApi {
        // async fn  ->  page calls `await waterui.invoke("greet", {name})`
        async fn greet(&self, name: String) -> Json<Greeting> {
            Json(Greeting::for_name(name))
        }
        // fn returning a signal  ->  mirrored state: `waterui.state.address`, `waterui.watch("address", cb)`
        fn address(&self) -> Binding<Str> {
            self.address.clone()
        } // Binding: JS writes flow back
        fn greetings(&self) -> Computed<u32> {
            self.greetings.clone().computed()
        } // read-only mirror
    }

    pub fn body() -> impl View {
        use waterui::webview::WebView;

        let url = "https://waterui.dev";
        let address = Binding::container(Str::from(""));
        let greetings = Binding::container(0_u32);

        WebView::open(url).serve(PageApi { address, greetings })
    }
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 8/13
// Listing: independent graphics constructors, interleaved with imports.
// ---------------------------------------------------------------------------
pub fn media_block_08() {
    let source = Str::from("<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>");

    use waterui_canvas::{Canvas, DrawingContext}; // its own crate — add waterui-canvas to Cargo.toml

    let _ = {
        Canvas::new(|ctx: &mut DrawingContext| {
            let _ = ctx; // [ellipsis filled]
        })
    };

    use waterui_barcode::Barcode; // its own crate — add waterui-barcode to Cargo.toml
    let _ = { Barcode::qr("https://waterui.dev").size(120.0, 120.0) };
    let _ = { Barcode::code128("012345").size(160.0, 60.0) };

    use waterui::svg::Svg;
    let _ = { Svg::new(source) };
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 9/13
//
// `src/starfield.wgsl` is copied from `examples/starfield` so the path the
// macro resolves is real.
// ---------------------------------------------------------------------------
pub fn media_block_09() -> impl View {
    use waterui::graphics::shader;
    shader!("starfield.wgsl").size(400.0, 500.0) // fragment shader from src/, no build.rs, no wgpu dep
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 10/13
// ---------------------------------------------------------------------------
pub fn media_block_10() -> impl View {
    use waterui::color::Srgb;
    use waterui_particle::ParticleSystem;

    ParticleSystem::new(10_000) // max particles
        .emit_from_rect(1.5, 0.1)
        .at(0.5, -0.05) // positions/sizes are normalized 0..1 view space
        .rate(2_500.0)
        .life(0.6, 0.8)
        .speed(2.5, 4.5) // (min, max) randomization ranges
        .color(
            Color::from(Srgb::new(0.8, 0.9, 1.0)).with_opacity(0.4),
            Color::from(Srgb::new(0.85, 0.95, 1.0)).with_opacity(0.0),
        ) // start -> end over lifetime
}

// ---------------------------------------------------------------------------
// media.md § "## Data: charts and maps" — rust block 11/13
// ---------------------------------------------------------------------------
pub fn media_block_11() -> impl View {
    use waterui::color::Srgb;
    use waterui::reactive::binding;

    use waterui_chart::{AxisConfig, BarChart, ChartExt, DataBounds, DataPoint, LineChart};

    let points = vec![DataPoint::new(0.0, 1.0), DataPoint::new(1.0, 2.0)];

    let _: Option<BarChart<Computed<Vec<DataPoint>>>> = None;

    let bounds = DataBounds::from_points(&points);
    LineChart::new(binding(points))
        .color(Srgb::from_hex("#22C55E"))
        .line_width(3.0)
        .axes(bounds) // ChartExt; axes first…
        .y_axis(AxisConfig::new().tick_count(5)) // …then per-axis config on the wrapper
        .size(350.0, 280.0)
}

// ---------------------------------------------------------------------------
// media.md § "## Data: charts and maps" — rust block 12/13
// ---------------------------------------------------------------------------
pub fn media_block_12() -> impl View {
    use waterui::reactive::binding;

    use waterui_map::{Annotation, Coordinate, Map, MapStyle, Region};

    let pins = Binding::container(Vec::<Annotation>::new());
    let loc = Binding::container(None::<waterui_map::Location>);

    let center = Coordinate::from_degrees(37.33, -122.03).expect("valid coordinate");
    let region: Binding<Region> = binding(Region::new(center, 0.05, 0.05)); // deltas are degree
    // spans; the annotation is needed — `binding`'s `impl Into<T>` cannot infer `T` here

    let _ = (MapStyle::Satellite, MapStyle::Hybrid);

    Map::new(region.clone()) // impl IntoComputed<Region>
        .style(MapStyle::Standard) // Standard | Satellite | Hybrid
        .annotations(pins.clone()) // impl IntoComputed<Vec<Annotation>>
        .optional_user_location(loc.clone()) // starts None while permission is pending
        .shows_compass(true)
        .shows_scale(true) // plain bools
}

// ---------------------------------------------------------------------------
// media.md § "## Data: charts and maps" — rust block 13/13
// ---------------------------------------------------------------------------
pub fn media_block_13(mut env: Environment) {
    use waterui_map_gpu::MapGpuOptions;
    env.insert(MapGpuOptions::new(waterui_url::Url::new(
        "https://tiles.openfreemap.org/styles/positron",
    )));
    #[cfg(not(target_vendor = "apple"))] // Apple bridges MapKit; installing here would bypass it
    waterui_map_gpu::install(&mut env);
}
