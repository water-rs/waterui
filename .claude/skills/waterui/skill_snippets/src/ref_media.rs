//! Snippets from `.claude/skills/waterui/references/media.md`, in file order.
//! Transcription conventions are documented in the crate README.
//!
//! Not transcribed, by design (README rule 6): `include_web!` (the macro stats a
//! web project's `package.json` at expansion), `js_file!` / `exec_file!` (they read a
//! script file at expansion), `.navigation_policy(..)` (it compiles only for an Apple
//! deployment target of OS 27, which this crate does not set).

use waterui::media::media_picker::Selected;
use waterui::prelude::*;

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 1/15
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
                _ => (), // the event enum is #[non_exhaustive]
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
// media.md § "## Media" (prose): `Photo::from_path`, `Image::from_encoded`,
// `.resizable()` and `.content_mode(ContentMode::Fit)` on both. Not counted as a
// rust block.
// ---------------------------------------------------------------------------
pub fn media_image_prose(encoded: &[u8]) -> impl View {
    use waterui::layout::ContentMode;
    use waterui::media::{Image, Photo};

    let _decoded: Result<Image, String> = Image::from_encoded(encoded);
    let _fill = Image::new(vec![0u8; 4], 1, 1)
        .resizable()
        .content_mode(ContentMode::Fill);

    Photo::from_path("photos/cover.jpg")
        .resizable()
        .content_mode(ContentMode::Fit)
}

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 2/15
// ---------------------------------------------------------------------------
pub fn media_block_02() {
    use waterui::media::{Photo, Url};

    let entered = Binding::container(Str::from("https://waterui.dev/logo.png"));
    let (photo_slot, _view) = Dynamic::new();

    let Some(parsed) = Url::parse(entered.snapshot().as_str()) else {
        return;
    }; // parse returns Option
    photo_slot.set(Photo::new(parsed));
}

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 3/15
// A statement sequence, then a handler-side line with its own receiver.
// ---------------------------------------------------------------------------
pub fn media_block_03() {
    use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};

    let selection: Binding<Option<Selected>> = Binding::default();
    let _ = {
        MediaPicker::new(&selection)
            .filter(MediaFilter::Image) // Image | Video | LivePhoto (the last two need `video`); takes a signal
            .label(text("Pick a photo")) // type-changing builder — call before storing
    };

    // The remaining filter variants named in the trailing comment.
    let _ = (MediaFilter::Video, MediaFilter::LivePhoto);
}

/// The handler-side half of block 3, which needs a `Selected` value in hand.
pub fn media_block_03_handler(selected: Selected) {
    // In a handler: Selected::load() is synchronous and consumes the selection.
    let media = selected.load(); // Media::Image(Url) | Video(Url) | LivePhoto(source)

    let _ = media;
}

// ---------------------------------------------------------------------------
// media.md § "## Media" (prose): `Selected::media()`, `LivePhoto::new(
// LivePhotoSource::new(image_url, video_url))` and its events, `video(url)`,
// `video_player(url)`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_live_prose(selected: &Selected) -> impl View {
    use waterui::media::live::{Event as LiveEvent, LivePhotoSource};
    use waterui::media::{LivePhoto, Media, Url};

    let _: &Media = selected.media();
    let _chromeless = waterui::video::video("https://waterui.dev/a.mp4");
    let _controls = waterui::video::video_player("https://waterui.dev/a.mp4");

    LivePhoto::new(LivePhotoSource::new(
        Url::new("https://waterui.dev/still.heic"),
        Url::new("https://waterui.dev/motion.mov"),
    ))
    .on_event(|event| match event {
        LiveEvent::MotionStarted | LiveEvent::MotionEnded => (),
        LiveEvent::MotionFailed(_message) => (),
        _ => (),
    })
}

// ---------------------------------------------------------------------------
// media.md § "## Media" — rust block 4/15
// ---------------------------------------------------------------------------
pub fn media_block_04() -> impl View {
    use waterui::video::{self, MediaItem, PlaybackSession, Playlist, Url, VideoPlayer};

    let first_item = MediaItem::from(Url::parse("https://waterui.dev/a.mp4").expect("valid url"));
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
// media.md § "## Media" (prose): `MediaItem::from(url)`, `MediaItem::new(url,
// Delivery::Hls)` with `.subtitle_track(..)` / `.metadata(..)`, `item.get_id()`;
// `PlayerController` readers, writable bindings via methods, and commands. Not
// counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_controller_prose() {
    use std::time::Duration;
    use waterui::video::{
        Delivery, MediaItem, MediaItemId, MediaMetadata, PlaybackError, PlaybackPhase,
        PlaybackSession, Playlist, RepeatMode, SubtitleTrack, Url, Volume,
    };

    let item = MediaItem::from(Url::parse("https://waterui.dev/a.mp4").expect("valid url"));
    let id: MediaItemId = item.get_id();
    let live = MediaItem::new("https://waterui.dev/live.m3u8", Delivery::Hls)
        .subtitle_track(SubtitleTrack::new("https://waterui.dev/en.vtt").label("English"))
        .metadata(MediaMetadata::new().with_title("Live"));
    let session = PlaybackSession::new(Playlist::new(item, Vec::new()));
    let controller = session.controller();

    let _: Computed<_> = controller.position();
    let _: Computed<_> = controller.duration();
    let _: Computed<_> = controller.current_item_index();
    let _: Computed<MediaItemId> = controller.current_item_id();
    let _: Computed<PlaybackPhase> = controller.phase();

    let _ = controller.volume();
    let _ = Volume::new(0.5);
    let _ = controller.muted();
    let _ = controller.playback_rate();
    let _ = controller.repeat_mode();
    let _ = controller.shuffle_enabled();
    let _ = controller.subtitle_selection();

    let _ = controller.seek_to_item(id);
    let _: Result<(), PlaybackError> = controller.seek(Duration::from_secs(30));
    let _ = controller.seek_relative(-10.0);
    let _ = controller.next();
    let _ = controller.previous();
    controller.play();
    controller.pause();
    controller.stop();
    controller.set_repeat(RepeatMode::Off);
    controller.set_shuffle(true);
    controller.add_item(live);
    controller.replace_playlist(Playlist::single("https://waterui.dev/b.mp4"));
}

// ---------------------------------------------------------------------------
// media.md § "## Media" (prose): `VideoPlayer::projection(..)`, `PlaybackSession::
// with_policy(..)`, `track_catalog()`, `live_window()`, `seek_to_live_edge()`,
// `step_forward()` / `step_backward()`, `preserve_pitch()` and the
// `Event::PlaybackMetrics { metrics }` event. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_video_advanced_prose() -> impl View {
    use waterui::video::{
        EquirectangularProjection, Event, LiveWindow, MediaItem, PlaybackError, PlaybackMetrics,
        PlaybackPolicy, PlaybackSession, Playlist, SphericalViewport, TrackCatalog, Url,
        VideoPlayer, VideoProjection,
    };

    let item = MediaItem::from(Url::parse("https://waterui.dev/a.mp4").expect("valid url"));
    let session = PlaybackSession::with_policy(
        Playlist::new(item, Vec::new()),
        PlaybackPolicy::live_default(),
    );
    let controller = session.controller();

    let _: Computed<TrackCatalog> = controller.track_catalog();
    let _: Computed<Option<LiveWindow>> = controller.live_window();
    let _: Result<(), PlaybackError> = controller.seek_to_live_edge();
    controller.step_forward();
    controller.step_backward();
    let _: Binding<bool> = controller.preserve_pitch();

    VideoPlayer::new(session)
        .projection(VideoProjection::Equirectangular(
            EquirectangularProjection::new(SphericalViewport::new(0.0, 0.0, 75.0)),
        ))
        .on_event(|event| {
            if let Event::PlaybackMetrics { metrics } = event {
                let _: PlaybackMetrics = metrics;
            }
        })
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" — rust block 5/15
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
            _ => (), // the event enum is #[non_exhaustive]
        })
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" (prose): `WebView::open_assets(server, entry)`
// with `DirectoryServer::new(dir).into_server_fn()` or a closure, and
// `.bridge_origins(..)` / `.expose(name, signal)` / `.handler(name, closure)`. Not
// counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_web_bundled_prose() -> impl View {
    use waterui::webview::{
        AssetRequest, AssetResponse, BridgeOrigins, DirectoryServer, Json, WebView,
    };

    let address = Binding::container(Str::from(""));

    let _directory = WebView::open_assets(
        DirectoryServer::new("web/dist").into_server_fn(),
        "index.html",
    );
    let _closure = WebView::open_assets(
        |_request: &AssetRequest| AssetResponse::not_found(),
        "index.html",
    );
    let _any = WebView::open("https://waterui.dev").bridge_origins(BridgeOrigins::Any);

    WebView::open("https://waterui.dev")
        .bridge_origins(["https://waterui.dev"])
        .expose("address", address)
        .handler("echo", |Json(text): Json<String>| async move { Json(text) })
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" (prose): a `WebView` handle from
// `WebViewController::open()` and `webview.proxy_scope(|| controls)`. Not counted
// as a rust block.
// ---------------------------------------------------------------------------
pub fn media_web_handle_prose(controller: waterui::webview::WebViewController) -> impl View {
    use waterui::webview::{WebView, WebViewProxy};

    let webview: WebView = controller.open();
    let controls =
        webview.proxy_scope(|| button("Back").action(|proxy: WebViewProxy| proxy.go_back()));
    vstack((controls, webview))
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" (prose): the proxy's `run_javascript`,
// `go_forward`, `refresh`, `stop`, `set_cookie` / `get_cookies`, `set_user_agent`,
// and `eval!` / `exec!` on a `WebView` handle. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub async fn media_web_script_prose(
    webview: &waterui::webview::WebView,
    proxy: waterui::webview::WebViewProxy,
) -> Result<(), waterui::webview::JsError> {
    let _value: Result<Str, Str> = proxy.run_javascript("1 + 1").await;
    let _cookies = proxy.get_cookies().await;
    proxy.go_forward();
    proxy.refresh();
    proxy.stop();
    proxy.set_user_agent("WaterUI");

    let _title: String = waterui::eval!(webview, "document.title").await?;
    let offset = 3_u32;
    waterui::exec!(webview, "window.scrollTo(0, @{offset})").await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" — rust block 6/15
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
                    if let Some(url) = Url::parse_user_input(addr.snapshot().as_str()) {
                        proxy.go_to(url);
                    }
                })
                .state(&address),
        ))
    })
}

// ---------------------------------------------------------------------------
// media.md § "## Web content" — rust block 7/15
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
        // fn returning a signal  ->  mirrored state: after `await waterui.ready`, `waterui.state.address`, `waterui.watch("address", cb)`
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
// media.md § "## Web content" (prose): `#[js_api(namespace = "app")]`,
// `#[js(skip)]`, `#[js(rename = "..")]`, `Result<Json<_>, E>` rejecting the
// page's promise, and `typescript_declarations`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub mod media_js_api_prose {
    use waterui::js_api;
    use waterui::prelude::*;
    use waterui::webview::{Json, WebView};

    #[derive(serde::Serialize)]
    pub struct Greeting {
        message: String,
    }

    #[derive(Debug)]
    pub struct ApiError;
    impl core::fmt::Display for ApiError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("api error")
        }
    }
    impl core::error::Error for ApiError {}

    pub struct App {
        theme: Binding<Str>,
    }

    #[js_api(namespace = "app")]
    impl App {
        fn theme(&self) -> Binding<Str> {
            self.theme.clone()
        }
        async fn greet(&self, name: String) -> Result<Json<Greeting>, ApiError> {
            Ok(Json(Greeting { message: name }))
        }
        #[js(skip)]
        pub async fn internal(&self) {}
        #[js(rename = "hello")]
        async fn greet_renamed(&self, name: String) -> Json<Greeting> {
            Json(Greeting { message: name })
        }
    }

    pub fn body(theme: Binding<Str>) -> impl View {
        let _declarations: String = waterui::webview::typescript_declarations::<App>();
        WebView::open("https://waterui.dev").serve(App { theme })
    }
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 8/15
// Listing: independent graphics constructors, interleaved with imports.
// ---------------------------------------------------------------------------
pub fn media_block_08() {
    use waterui::layout::{Point, Rect};
    let source = Str::from("<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>");
    let radius = Binding::f32(20.0);
    let accent = Color::srgb(0, 122, 255);

    use waterui::canvas::{Canvas, Circle, StrokeStyle}; // facade feature `canvas`

    let _ = {
        Canvas::new(move |cx| {
            // runs once per mount; signals stay bound
            cx.fill(cx.size().map(Rect::from_size), Color::srgb(240, 240, 240));
            cx.stroke(
                radius.map(|r| Circle::new(Point::new(40.0, 40.0), r)),
                StrokeStyle::new(2.0),
                accent,
            );
        })
    };

    use waterui::barcode::Barcode; // facade feature `barcode` (the crate is also usable directly)
    let _ = { Barcode::qr("https://waterui.dev").size(120.0, 120.0) };
    let _ = { Barcode::code128("012345").size(160.0, 60.0) };

    use waterui::svg::Svg;
    let _ = { Svg::new(source) };
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" (prose): the canvas model — `cx.each`,
// `cx.scope`, `cx.size()`, `cx.time()`, the scoped `clip`/`transform`/`layer`,
// `cx.shadow` and `cx.text`, `Drawing::record` with `cx.draw` and `Canvas::from`,
// `ImageSource::status`, `.intrinsic_size(..)` and `.offered_label(..)`. Not counted
// as a rust block.
// ---------------------------------------------------------------------------
pub fn media_canvas_prose(image: waterui::canvas::ImageSource) -> impl View {
    use waterui::canvas::{Canvas, Circle, Drawing, Layer, Sampling, Shadow, TextPlacement};
    use waterui::layout::{Affine2, Point, Rect, Size};
    use waterui::text::styled::StyledStr;

    #[derive(Clone, PartialEq)]
    struct Dot {
        id: u32,
        at: Point,
    }
    let dots: Binding<Vec<Dot>> = Binding::container(Vec::new());
    let detail = Binding::u32(1);
    let drawing = Drawing::record(Size::new(24.0, 24.0), |cx| {
        cx.fill(
            Circle::new(Point::new(12.0, 12.0), 12.0),
            waterui::graphics::WorkingColor::WHITE,
        );
    });
    let _status: Computed<waterui::canvas::ImageStatus> = image.status();
    let _icon = Canvas::from(drawing.clone());

    Canvas::new(move |cx| {
        let _size: Computed<Size> = cx.size();
        let _time: Computed<core::time::Duration> = cx.time();
        cx.each(
            dots,
            |dot: &Dot| dot.id,
            |cx, dot| {
                cx.fill(
                    dot.map(|dot| Circle::new(dot.at, 3.0)),
                    Color::srgb(0, 0, 0),
                )
            },
        );
        cx.scope(detail, |cx, detail: &u32| {
            for _ in 0..*detail {
                cx.fill(Rect::from_size(Size::new(4.0, 4.0)), Color::srgb(0, 0, 0));
            }
        });
        cx.shadow(
            Circle::new(Point::new(40.0, 40.0), 30.0),
            Shadow::new(4.0, waterui::graphics::WorkingColor::BLACK).offset(0.0, 2.0),
        );
        cx.text(
            StyledStr::plain("Dots"),
            TextPlacement::at(Point::new(4.0, 4.0)),
        );
        cx.clip(Circle::new(Point::new(40.0, 40.0), 30.0), |cx| {
            cx.transform(Affine2::translate(10.0, 0.0), |cx| {
                cx.layer(Layer::opacity(0.5), |cx| {
                    cx.image(
                        image,
                        Rect::from_size(Size::new(80.0, 80.0)),
                        Sampling::Linear,
                    );
                    cx.draw(&drawing, Affine2::IDENTITY);
                });
            });
        });
    })
    .intrinsic_size(Size::new(80.0, 80.0))
    .offered_label(Str::from("Dots"))
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" (prose): `BarcodeSource::qr` /
// `::code128` pre-validation, `.dark_color` / `.light_color`, and the SVG
// `.tint(..)` / `Svg::from_path`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_barcode_svg_prose() -> impl View {
    use waterui::barcode::{Barcode, BarcodeError, BarcodeSource};
    use waterui::svg::Svg;

    let _qr: Result<BarcodeSource, BarcodeError> = BarcodeSource::qr("https://waterui.dev");
    let _code: Result<BarcodeSource, BarcodeError> = BarcodeSource::code128("012345");
    let _path = Svg::from_path("M0 0 L10 10", 24.0, 24.0);

    vstack((
        Barcode::qr("https://waterui.dev")
            .dark_color(Color::srgb(0, 0, 0))
            .light_color(Color::srgb(255, 255, 255)),
        Svg::new("<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").tint(Color::srgb(0, 122, 255)),
    ))
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 9/15
//
// `src/starfield.wgsl` is copied from `examples/starfield` so the path the
// macro resolves is real.
// ---------------------------------------------------------------------------
pub fn media_block_09() -> impl View {
    use waterui::graphics::shader;
    shader!("starfield.wgsl").size(400.0, 500.0) // static fragment shader from the .wgsl file beside this source file, no build.rs, no wgpu dep
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" (prose): `shader!(..)` is a
// `ShaderPaintView`; `.animated(signal)` and `.uniforms(signal)`. Not counted as a
// rust block.
// ---------------------------------------------------------------------------
pub fn media_shader_prose() -> impl View {
    use waterui::graphics::shader;
    let level = Binding::container(vec![0.5_f32]);
    shader!("starfield.wgsl")
        .animated(true)
        .uniforms(level)
        .size(400.0, 500.0)
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 10/15
//
// Glue: a `GpuContent` for `terminal`, and the bindings the handler writes.
// ---------------------------------------------------------------------------
pub fn media_block_10() -> impl View {
    use waterui::graphics::{Context, Frame, GpuContent, GpuContentView};

    struct Terminal;
    impl GpuContent for Terminal {
        fn setup(&mut self, _gpu: &Context<'_>) {}
        fn render(&mut self, _frame: &mut Frame<'_>) {}
    }
    let terminal = Terminal;
    let typed = Binding::container(Str::from(""));
    let interrupt = Binding::bool(false);
    let submit = Binding::bool(false);
    let pointer_x = Binding::f64(0.0);

    use waterui::graphics::input::{Code, Key, Modifiers, NamedKey, SurfaceInputEvent};

    GpuContentView::new(terminal).on_input(move |event| match event {
        // IME and injected insertions arrive as text events, never as `Key`
        SurfaceInputEvent::TextInput(text) | SurfaceInputEvent::CompositionCommit(text) => {
            typed.set(text.clone())
        }
        // `key` is the logical key; `code` is the physical one, for layout-independent bindings
        SurfaceInputEvent::Key {
            pressed: true,
            code: Code::KeyC,
            modifiers,
            ..
        } if modifiers.contains(Modifiers::CONTROL) => interrupt.set(true),
        SurfaceInputEvent::Key {
            pressed: true,
            key: Key::Named(NamedKey::Enter),
            ..
        } => submit.set(true),
        // remember the pointer: under `.context_menu` the secondary press never reaches here
        SurfaceInputEvent::PointerMove { position } => pointer_x.set(position.x),
        _ => (),
    })
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" (prose): `.labeled(..)` / `.described(..)`,
// `.on_ime_caret(..)`, and a `SceneContent` handling input. Not counted as a rust
// block.
// ---------------------------------------------------------------------------
pub fn media_gpu_input_prose<C: waterui::graphics::GpuContent>(content: C) -> impl View {
    use waterui::graphics::GpuContentView;
    use waterui::graphics::gpu::Rect;

    GpuContentView::new(content)
        .labeled("Terminal")
        .described("Shell session")
        .on_ime_caret(|| Option::<Rect>::None)
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" (prose): the `FilterViewExt` family table —
// one method per family row, plus `.filter(F)`, the auxiliary-input paths and
// `FilterImage`'s constructors. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_filter_table_prose(png: &[u8]) -> Result<impl View, waterui::Error> {
    use waterui::graphics::filter_view::filters::Invert;
    use waterui::graphics::filter_view::{BlendMode, FilterImage, LutImage, TransitionDirection};
    use waterui::media::Photo;

    let _: Option<(LutImage, TransitionDirection)> = None;
    let decoded = FilterImage::from_encoded(png)?;
    let _ = Photo::new("https://waterui.dev/logo.png").filter(Invert);
    let _ = text("b")
        .blend_with_image(
            FilterImage::from_rgba8(1, 1, vec![0; 4]),
            0.5,
            BlendMode::Multiply,
        )
        .displacement_warp(decoded, 1.0, 1.0);
    let _ = text("a")
        .tone_curve(0.1, 0.5, 0.9, 1.0, 1.0)
        .color_matrix([[1.0, 0.0, 0.0, 0.0]; 3])
        .motion_blur(4.0, 30.0)
        .median3x3()
        .convolution3x3([0.0_f32; 9])
        .photo_effect_noir()
        .dot_halftone(1.0, 2.0, 3.0, 4.0)
        .twirl_distortion(1.0, 2.0, 3.0, 4.0)
        .perspective_transform([[0.0, 0.0]; 4]);

    Ok(Photo::new("https://waterui.dev/logo.png")
        .sepia(0.8)
        .vignette(0.6, 0.4)
        .pixellate(6.0))
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 11/15
//
// `src/crt.wgsl` is a real shader effect, so `include_str!` resolves and
// `ShaderEffect::new` validates the module. `?` puts the function's return
// type on the hook: `ShaderEffectError` is the error `new` returns.
// ---------------------------------------------------------------------------
pub fn media_block_11() -> Result<impl View, waterui::graphics::filter_view::ShaderEffectError> {
    let terminal = || text("terminal");
    let strength = Binding::f32(0.35);
    let focused = Binding::bool(true);

    use waterui::graphics::filter_view::ShaderEffect;

    let crt = ShaderEffect::new(include_str!("crt.wgsl"))?; // invalid WGSL is an Err, never a blank view
    Ok(terminal()
        .effect(crt.clone()) // a clone shares the validated module; constant params carry
        .param(strength.clone()) // a signal — the shader reads it as effect_param(0u)
        .animated(focused.clone())) // a bool signal — a frame after every frame while true
}

// ---------------------------------------------------------------------------
// media.md § "## Graphics and codes" — rust block 12/15
// ---------------------------------------------------------------------------
pub fn media_block_12() -> impl View {
    use waterui::color::Srgb;
    use waterui::particle::ParticleSystem;

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
// media.md § "## Graphics and codes" (prose): the further `ParticleSystem`
// builders. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_particle_prose() -> impl View {
    use waterui::particle::{ParticleShape, ParticleSystem};

    ParticleSystem::new(1_000)
        .emit_from_circle(0.2)
        .angle(0.0, core::f32::consts::TAU)
        .spin(0.0, 1.0)
        .shape(ParticleShape::Rect)
        .additive()
        .softness(0.5)
        .stretch_with_velocity()
        .gravity(0.0, 0.5)
        .wind(0.1, 0.0)
        .turbulence(0.2)
        .drag(0.1)
        .collide_with_viewport()
        .collide_with_rect(0.0, 0.0, 1.0, 1.0)
        .collide_with_circle_obstacle(0.5, 0.5, 0.1)
        .collide_with_particles(0.02, 0.5)
        .bounce(0.4)
        .surface_friction(0.1)
}

// ---------------------------------------------------------------------------
// media.md § "## Data: charts and maps" — rust block 13/15
// ---------------------------------------------------------------------------
pub fn media_block_13() -> impl View {
    use waterui::color::Srgb;
    use waterui::reactive::binding;

    use waterui::chart::{AxisConfig, BarChart, ChartExt, DataBounds, DataPoint, LineChart};

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
// media.md § "## Data: charts and maps" (prose): `.focused(&binding)`,
// `.focused_tooltip(..)`, `Tooltip` / `TooltipContent`, `Legend`, and
// `LineChart::fill`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_chart_interaction_prose() -> impl View {
    use waterui::chart::{
        DataPoint, HitResult, Legend, LegendItem, LineChart, Tooltip, TooltipContent,
    };
    use waterui::color::Srgb;

    let points = Binding::container(vec![DataPoint::new(0.0, 1.0), DataPoint::new(1.0, 2.0)]);
    let focused: Binding<Option<HitResult<DataPoint>>> = Binding::container(None);

    vstack((
        LineChart::new(points)
            .fill(0.3)
            .focused(&focused)
            .focused_tooltip(|_hit| {
                Tooltip::new(TooltipContent::new().title("Sales").value("Total", "2.0"))
            }),
        Legend::new(vec![LegendItem::new("Sales", Srgb::from_hex("#22C55E"))]),
    ))
}

// ---------------------------------------------------------------------------
// media.md § "## Data: charts and maps" — rust block 14/15
// ---------------------------------------------------------------------------
pub fn media_block_14() -> impl View {
    use waterui::reactive::binding;

    use waterui_map::{Annotation, Coordinate, Location, Map, MapStyle, Region};

    let pins = Binding::container(Vec::<Annotation>::new());
    let loc = Binding::container(None::<Location>);

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
// media.md § "## Data: charts and maps" (prose): `Map::centered_on`,
// `Annotation::new(..).subtitle(..)`, `.is_interactive`, `.shows_user_location`
// and `.status(&binding)`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_map_prose() -> impl View {
    use waterui_map::{Annotation, Coordinate, Map, MapStatus};

    let coordinate = Coordinate::from_degrees(37.33, -122.03).expect("valid coordinate");
    let pins = Binding::container(vec![
        Annotation::new(coordinate, "Apple Park").subtitle("Cupertino"),
    ]);
    let status = Binding::container(MapStatus::Loading);

    Map::centered_on(coordinate)
        .annotations(pins)
        .is_interactive(false)
        .shows_user_location(true)
        .status(&status)
}

// ---------------------------------------------------------------------------
// media.md § "## Data: charts and maps" — rust block 15/15
// ---------------------------------------------------------------------------
// media.md writes `waterui_url::Url` qualified to show which crate owns it.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn media_block_15(mut env: Environment) {
    use waterui_map_gpu::MapGpuOptions;
    env.insert(MapGpuOptions::new(waterui_url::Url::new(
        "https://tiles.openfreemap.org/styles/positron",
    )));
    #[cfg(not(target_vendor = "apple"))]
    // Apple bridges MapKit; keeps the GPU map out of Apple builds
    waterui_map_gpu::install(&mut env);
}

// ---------------------------------------------------------------------------
// media.md § "## Data: charts and maps" (prose): `MapGpuOptions::
// satellite_style_url` / `hybrid_style_url`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn media_map_gpu_styles_prose() -> waterui_map_gpu::MapGpuOptions {
    use waterui_map_gpu::MapGpuOptions;

    MapGpuOptions::new(waterui_url::Url::new(
        "https://tiles.openfreemap.org/styles/positron",
    ))
    .satellite_style_url(waterui_url::Url::new(
        "https://tiles.example/satellite.json",
    ))
    .hybrid_style_url(waterui_url::Url::new("https://tiles.example/hybrid.json"))
}
