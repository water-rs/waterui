# Media, web, graphics, and data views

Photos and in-memory images, media picking, video playback, web views and the JS bridge,
Chromium, GPU shaders and particles, charts, and maps. Everything follows the catalog
conventions in [components.md](components.md): lowercase ergonomic constructors,
`Type::new` general constructors, signal-taking parameters.

## Contents

- Media
- Web content
- Graphics and codes
- Data: charts and maps

## Media

```rust
use waterui::media::photo::Event as PhotoEvent;   // the event type needs this alias import
use waterui::media::{Image, Photo, Url};

Photo::new("https://waterui.dev/logo.png")   // impl IntoComputed<Url>; Url: From<&'static str>
    .on_event(|event| match event {          // inherent on Photo — attach it BEFORE filters,
        PhotoEvent::Loaded => (),            // which wrap the view in a Filtered<..> type
        PhotoEvent::Error(_msg) => (),
    })
    .blur(radius.clone())
    .saturation(sat.clone())

Image::new(rgba_pixels, 200, 150)        // in-memory RGBA8; asserts len == w * h * 4
```

`Photo` displays a URL; `Image` is the only route for decoded/generated pixel buffers. A
runtime string is not a `Url` — parse it, and report the failure rather than letting a
bad address reach the loader as a local path:

```rust
let Some(parsed) = Url::parse(entered.get().as_str()) else { return };   // parse returns Option
photo_slot.set(Photo::new(parsed));
```

Media picking — the picker itself is the tappable control:

```rust
use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};

let selection: Binding<Option<Selected>> = Binding::default();
MediaPicker::new(&selection)
    .filter(MediaFilter::Image)          // Image | Video | LivePhoto; takes a signal
    .label(text("Pick a photo"))         // type-changing builder — call before storing

// In a handler: Selected::load() is synchronous and consumes the selection.
let media = selected.load();             // Media::Image(Url) | Video(Url) | LivePhoto(source)
```

`LivePhoto::new(source)` displays the live-photo variant.

Video — `video_player(url)` is the one-item shorthand; the general form is a playlist
session whose controller you grab *before* the session moves into the player:

```rust
let playlist = Playlist::new(first_item, more_items);   // deliberately non-empty
let session = PlaybackSession::new(playlist).autoplay();
let controller = session.controller();                  // Clone; capture before the move

VideoPlayer::new(session)
    .show_controls(true)
    .content_mode(video::ContentMode::Fit)              // Fit | Fill | Stretch
    .on_event(|event| match event {
        video::Event::ReadyToPlay | video::Event::Ended => (),
        video::Event::Error { message: _ } => (),
        _ => (),                                        // Buffering, PlaybackStateChanged, …
    })
```

`MediaItem::from(url)` builds items and exposes a `Copy` `id: MediaItemId` — collect the
ids before the items move into the playlist; they are what `controller.seek_to_item(id)`
takes. `PlayerController` exposes reactive readers (`position()`, `duration()`,
`current_item_index()` → `Computed<T>`), writable bindings via methods (`volume()`,
`muted()`, `playback_rate()`, `repeat_mode()`), fallible commands (`seek`, `next`,
`previous` → `Result<_, PlaybackError>`) and infallible ones (`play`, `pause`, `stop`).

Runtime capture permissions (camera, microphone, location) are requested through
`waterkit-permission` — see [project.md](project.md), Permissions.

## Web content

```rust
use waterui::webview::{ScriptInjectionTime, Url, WebView, WebViewEvent, WebViewProxy};

WebView::open("https://waterui.dev")
    .redirects_enabled(allow.clone())
    .user_agent(ua.clone())
    .inject("marker", "document.body.dataset.app = 'waterui';", ScriptInjectionTime::DocumentEnd)
    .on_event(|event| match event {
        WebViewEvent::WillNavigate { url: _ } => (),
        WebViewEvent::Loading { progress: _ } => (),    // f32
        WebViewEvent::Loaded => (),
        WebViewEvent::Redirect { from: _, to: _ } => (),
        WebViewEvent::Error(_) => (),
    })
```

`.inject(key, script, time)` re-injects on every page load; injecting again under the
same key *replaces* the earlier script. `.on_event` takes `Fn` — capture cloned bindings.

Driving a live page: `.with_proxy(|| controls)` consumes the builder, installs a
`WebViewProxy` into the closure's environment, and returns `vstack((controls, page))` —
controls above, page below. Inside any handler in that scope the proxy is a bare
parameter, and navigation is fire-and-forget:

```rust
open.with_proxy(move || hstack((
    button("Back").action(|proxy: WebViewProxy| proxy.go_back()),
    button("Go").action(|proxy: WebViewProxy, State(addr): State<Binding<Str>>| {
        // parse_user_input tolerates human input (missing scheme); returns Option.
        if let Some(url) = Url::parse_user_input(addr.get().as_str()) {
            proxy.go_to(url);
        }
    }).state(&address),
)))
```

`proxy.run_javascript(expr)` is async (`Result<Str, _>`) — await it in `.action_async`.

The Rust↔page bridge is declared with `#[js_api]` on an impl block and served on the
builder; the method's *asyncness* is the discriminator:

```rust
use waterui::js_api;
use waterui::webview::Json;

struct PageApi { address: Binding<Str>, greetings: Binding<u32> }

#[js_api]
impl PageApi {
    // async fn  ->  page calls `await waterui.invoke("greet", {name})`
    async fn greet(&self, name: String) -> Json<Greeting> { Json(Greeting::for_name(name)) }
    // fn returning a signal  ->  mirrored state: `waterui.state.address`, `waterui.watch("address", cb)`
    fn address(&self) -> Binding<Str> { self.address.clone() }        // Binding: JS writes flow back
    fn greetings(&self) -> Computed<u32> { self.greetings.clone().computed() }   // read-only mirror
}

WebView::open(url).serve(PageApi { address, greetings })
```

A typed `Json<T>` payload requires `T: Serialize`/`Deserialize`, so the app needs `serde`
(with the `derive` feature) as a direct dependency.

Engine selection is a project setting (`webview_backend` in `Water.toml`), not a code
decision.

`waterui-chromium` is the separate, heavier component for a full Chromium surface,
headless pages, screenshots, and DevTools Protocol access. It is a direct crate
dependency (not a `waterui` feature): the backend installs a `ChromiumController` into
the environment (`env.get::<ChromiumController>()`); `controller.open(config)` returns a
value that is both a `View` and a handle (`.page()` — clone the page out *before* the
view moves into the layout); `page.watch(|event: ChromiumEvent| ..)` observes lifecycle
(this is an event subscription, not the reactive `watch`); `page.cdp()` executes raw or
typed CDP commands; `controller.headless(config).await` gives an off-screen page you must
`page.close().await` yourself.

## Graphics and codes

```rust
use waterui_canvas::{Canvas, DrawingContext};   // its own crate — add waterui-canvas to Cargo.toml

Canvas::new(|ctx: &mut DrawingContext| { /* immediate-mode drawing */ })

use waterui::barcode::Barcode;           // feature = "barcode"
Barcode::qr("https://waterui.dev").size(120.0, 120.0)
Barcode::code128("012345").size(160.0, 60.0)

use waterui::svg::Svg;
Svg::new(source)
```

Custom GPU content, from cheapest to fullest control:

```rust
use waterui::graphics::shader;
shader!("starfield.wgsl").size(400.0, 500.0)   // fragment shader from src/, no build.rs, no wgpu dep
```

`shader!` resolves the path against the calling crate's `src/` and expands to a view. For
full control, implement `GpuView` (async `setup(&mut self, ctx, env)` owns persistent GPU
resources; sync `render(&mut self, frame)` draws — call `frame.request_redraw()` at the
end to keep animating) and wrap it: `GpuSurface::new(renderer).size(w, h)`. One renderer
instance lives for the surface's lifetime. Inside `render` you are outside the reactive
graph: holding cloned `Binding`s on the renderer struct and `.get()`ing them per frame is
correct there. `waterui::graphics` re-exports `bytemuck`. Verify GPU components with
offscreen rendering, never by reasoning about the code.

Particles (feature `particle`, or the `waterui-particle` crate directly):

```rust
ParticleSystem::new(10_000)             // max particles
    .emit_from_rect(1.5, 0.1).at(0.5, -0.05)     // positions/sizes are normalized 0..1 view space
    .rate(2_500.0).life(0.6, 0.8).speed(2.5, 4.5)  // (min, max) randomization ranges
    .color(Color::from(Srgb::new(0.8, 0.9, 1.0)).with_opacity(0.4),
           Color::from(Srgb::new(0.85, 0.95, 1.0)).with_opacity(0.0))   // start -> end over lifetime
```

Every builder parameter is a signal slot (`impl IntoSignalF32`). Careful: on
`ParticleSystem`, `.size(min, max)` is the *particle* size range in normalized units —
it is not the frame modifier.

Shapes are views that fill the space they are given; `.fill()` and `.clip()` are the two
ways to use them — the styling reference covers them, including the normalized-radius
rule for `RoundedRectangle`.

## Data: charts and maps

Charts (feature `chart`, or `waterui-chart` directly). Two facts govern the whole family:

- Every constructor takes a **`Signal` of the data**, not `impl IntoComputed`. `Vec<T>`
  of the point types is a constant signal already (`BarChart::new(vec![..])` compiles);
  the composite data structs (`DepthData`, `HeatmapData`, `RadarData`, `AreaData`,
  `GaugeData`, `ContourData`) are not — wrap them: `HeatmapChart::new(binding(data))`.
- Chart colors take **`Srgb`**, not `Color` — `Srgb::from_hex("#3B82F6")`, `Srgb::WHITE`.
  Multi-series builders (`RadarSeries`, `AreaSeries`, `BubblePoint::with_color`) instead
  take four normalized `f32` components `(r, g, b, a)`.

```rust
use waterui::chart::{AxisConfig, BarChart, ChartExt, DataBounds, DataPoint, LineChart};

let bounds = DataBounds::from_points(&points);
LineChart::new(binding(points))
    .color(Srgb::from_hex("#22C55E"))
    .line_width(3.0)
    .axes(bounds)                                // ChartExt; axes first…
    .y_axis(AxisConfig::new().tick_count(5))     // …then per-axis config on the wrapper
    .size(350.0, 280.0)
```

Constructor shapes for the rest — argument order is the part you cannot guess:
`DataPoint::new(x, y)`; `PieChart::new(data).donut(0.5)` (normalized inner radius);
`Candle::new(x, open, high, low, close, volume)`;
`DepthData::new(bids, asks)` of `DepthLevel::new(price, cumulative_qty)`;
`HeatmapData::new(rows, cols, row_major_values)`; `ContourData::new(rows, cols, values,
levels)`; `RadarData::new(axis_count).labels(..).series(RadarSeries::new(name, values))`
(every vec exactly `axis_count` long); `AreaData::new(x_values).series(..).stacked(true)`;
`BubblePoint::with_color(x, y, size, r, g, b, a)`;
`GaugeData::new(value, min, max).region(GaugeRegion::hex(upper, "#22C55E"))` (regions in
ascending order) with `.arc(ArcAngles::from_degrees(-135.0, 135.0))` and
`.radii(GaugeRadii::new(0.3, 0.45))` (normalized).

Maps (feature `map`, or `waterui-map` directly):

```rust
use waterui::map::{Annotation, Coordinate, Map, MapStyle, Region};

let center = Coordinate::from_degrees(37.33, -122.03).expect("valid coordinate");
let region: Binding<Region> = binding(Region::new(center, 0.05, 0.05));   // deltas are degree
// spans; the annotation is needed — `binding`'s `impl Into<T>` cannot infer `T` here

Map::new(region.clone())                     // impl IntoComputed<Region>
    .style(MapStyle::Standard)               // Standard | Satellite | Hybrid
    .annotations(pins.clone())               // impl IntoComputed<Vec<Annotation>>
    .optional_user_location(loc.clone())     // starts None while permission is pending
    .shows_compass(true).shows_scale(true)   // plain bools
```

`Region` has public `center` / `latitude_delta` / `longitude_delta` fields — zooming is a
handler that `.set()`s a region with scaled deltas. `Coordinate::from_degrees` returns a
`Result`; out-of-range coordinates are errors, not clamps.

On GPU/self-drawn backends the map has **no built-in tile source** — install one in
`app(env)` or the map draws nothing (`waterui-map-gpu` + `waterui-url` as direct deps):

```rust
use waterui_map_gpu::MapGpuOptions;
env.insert(MapGpuOptions::new(waterui_url::Url::new("https://tiles.openfreemap.org/styles/positron")));
```

Maps need the `internet` permission; showing the user's location needs the `location`
permission declared in `Water.toml` *and* requested at runtime — see
[project.md](project.md).
