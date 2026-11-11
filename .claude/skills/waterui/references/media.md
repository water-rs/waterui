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

`waterui::media` (`Photo`, `Image`, `MediaPicker`, and the image kind of `asset!`) is
behind the `media` feature, which is not on by default: add `features = ["media"]` to the
`waterui` dependency. Four features sit beside it, none implied by `media`:

- `video` — `Video`, `VideoPlayer`, `LivePhoto`, video picks, the video kind of `asset!`;
  also `waterui::video`. Enable `["media", "video"]` for photo and video together.
- `video-gpu` — WaterUI's own GPU player (implies `video` and `gpu`). Apple and the web
  bridge a native player; Hydrolysis platforms (Android, Windows, Linux) draw video only
  when the app's own `waterui` dependency declares `video-gpu`.
- `remote` — fetching from web URLs links an HTTP client. Without it a `Photo` pointed
  at `https://…` reports `Event::Error` naming the feature; bundled and local sources
  need nothing.
- `webview` — see Web content.

```rust
use waterui::media::photo::Event as PhotoEvent;   // the event type needs this alias import
use waterui::media::{Image, Photo, Url};

Photo::new("https://waterui.dev/logo.png")   // impl IntoComputed<Url>; Url: From<&'static str>
    .on_event(|event| match event {          // inherent on Photo — attach it BEFORE filters,
        PhotoEvent::Loaded => (),            // which wrap the view in a Filtered<..> type
        PhotoEvent::Error(_msg) => (),
        _ => (),                             // the event enum is #[non_exhaustive]
    })
    .blur(radius.clone())
    .saturation(sat.clone())

Image::new(rgba_pixels, 200, 150)        // in-memory RGBA8; asserts len == w * h * 4
```

`Photo` displays a URL (`Photo::from_path(path)` takes a local path); `Image` is the route
for decoded or generated pixel buffers — `Image::from_encoded(&bytes)` decodes PNG/JPEG/…
bytes to a `Result<Image, String>`. Both keep their natural size (only scaling down,
aspect preserved) until `.resizable()`; then they fill the offer, and
`.content_mode(ContentMode::Fit)` or `Fill` (`waterui::layout::ContentMode`) keeps the
aspect ratio — without it the picture stretches per axis. A runtime string is not a
`Url` — parse it, and report the failure rather than letting a bad address reach the
loader as a local path:

```rust
let Some(parsed) = Url::parse(entered.snapshot().as_str()) else { return };   // parse returns Option
photo_slot.set(Photo::new(parsed));
```

Media picking — the picker itself is the tappable control:

```rust
use waterui::media::media_picker::{MediaFilter, MediaPicker, Selected};

let selection: Binding<Option<Selected>> = Binding::default();
MediaPicker::new(&selection)
    .filter(MediaFilter::Image)          // Image | Video | LivePhoto (the last two need `video`); takes a signal
    .label(text("Pick a photo"))         // type-changing builder — call before storing

// In a handler: Selected::load() is synchronous and consumes the selection.
let media = selected.load();             // Media::Image(Url) | Video(Url) | LivePhoto(source)
```

`Selected::media()` borrows the payload without consuming it. With `video`,
`LivePhoto::new(LivePhotoSource::new(image_url, video_url))` (`waterui::media::live`)
displays the live-photo variant, `.on_event` reports `MotionStarted` / `MotionEnded` / `MotionFailed(msg)`.

Video (feature `video`) — `video_player(url)` is the one-item shorthand (a paused player
with controls); `video(url)` is the chromeless autoplaying, looping `Video` view. The
general form is a playlist session whose controller you grab *before* the session moves
into the player (a session mounts once):

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

`MediaItem::from(url)` builds a progressive item (`&'static str` and `Url` convert; a
runtime `String` must be parsed first); `MediaItem::new(url, Delivery::Hls | Dash)` names
the transport, and `.subtitle_track(..)`, `.metadata(..)`, `.drm(..)` refine it. Each item
carries a `Copy` id, `item.get_id()` → `MediaItemId` — collect the ids before the items
move into the playlist; they are what `controller.seek_to_item(id)` takes.
`PlayerController` exposes reactive readers (`position()`, `duration()`,
`current_item_index()`, `current_item_id()`, `phase()` → `Computed<T>`), writable bindings
via methods (`volume()` of `Volume::new(level)`, level in `0.0..=1.0` or it panics, `muted()`, `playback_rate()`,
`repeat_mode()`, `shuffle_enabled()`, `subtitle_selection()`), fallible commands (`seek`,
`seek_relative`, `next`, `previous`, `seek_to_item` → `Result<_, PlaybackError>`) and
infallible ones (`play`, `pause`, `stop`, `set_repeat`, `set_shuffle`, `add_item`,
`replace_playlist`).

Beyond the basics: `VideoPlayer::projection(VideoProjection::Equirectangular(
EquirectangularProjection::new(SphericalViewport::new(yaw, pitch, fov))))` plays 360° video
(pitch within ±90°, vertical fov 30–120°, else it panics; `Video` has `.projection` too).
`PlaybackSession::with_policy(playlist, PlaybackPolicy::live_default())` picks realtime over
the default `vod_default()` buffering. On the controller, `track_catalog()` is a
`Computed<TrackCatalog>` of audio, video and subtitle tracks, `live_window()` a
`Computed<Option<LiveWindow>>` (`None` for finite media) with `seek_to_live_edge()` →
`Result<(), PlaybackError>`, `step_forward()` / `step_backward()` pause and show the next or
previous frame, and `preserve_pitch()` is a `Binding<bool>`. `video::Event::PlaybackMetrics
{ metrics }` arrives periodically while playing.

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
        _ => (),                                        // the event enum is #[non_exhaustive]
    })
```

`.inject(key, script, time)` re-injects on every page load; injecting again under the
same key *replaces* the earlier script (`js_file!("scripts/setup.js")` reads a script
from a file at compile time). `.on_event` takes `Fn` — capture cloned bindings.

Bundled and controlled content:

- `include_web!(resources, "web")` (features `webview` + `assets`) embeds a web frontend:
  the first argument borrows the app's `ResourceContext`, the second is the web project
  root (relative to the crate's `Cargo.toml`) holding `package.json`; named options
  `out_dir` (`dist`), `entry` (`index.html`), `spa`, `csp`. It expands to a `WebViewOpen`,
  so `.serve(..)`, `.inject(..)`, `.on_event(..)` chain on it. The `water` CLI builds and
  stages the frontend; one per application; main-frame navigations that leave its origin
  open in the system browser. It installs a navigation policy, which compiles only with
  the Apple deployment targets raised to `27.0` ([project.md](project.md)).
- `WebView::open_assets(server, entry)` serves `Fn(&AssetRequest) -> AssetResponse`
  (GET/HEAD only) under the engine's local asset origin, a secure context on every
  engine; `DirectoryServer::new(dir).into_server_fn()` is the ready-made server over a
  directory.
- `.navigation_policy(|request| NavigationDecision::Allow | Cancel | OpenExternally)`
  decides navigations the *page* starts (links, `window.location`, forms, `window.open`);
  loads the app directs and server redirects (`.redirects_enabled`) are not consulted.
  `stay_on_origin` is the ready-made policy. Same OS 27 requirement on Apple.
- `.bridge_origins(..)` chooses which documents may reach the bridge below. The default,
  `BridgeOrigins::Initial`, is the opening origin's main frame only; widen with
  a list of origins (`.bridge_origins(["https://waterui.dev"])`), `LocalFiles`, or `Any`
  (every page the view ever loads can then call every handler — only for content you
  control end to end).

Driving a live page: `.with_proxy(|| controls)` consumes the builder, installs a
`WebViewProxy` into the closure's environment, and returns `vstack((controls, page))` —
controls above, page below. Inside any handler in that scope the proxy is a bare
parameter, and navigation is fire-and-forget:

```rust
open.with_proxy(move || hstack((
    button("Back").action(|proxy: WebViewProxy| proxy.go_back()),
    button("Go").action(|proxy: WebViewProxy, State(addr): State<Binding<Str>>| {
        // parse_user_input tolerates human input (missing scheme); returns Option.
        if let Some(url) = Url::parse_user_input(addr.snapshot().as_str()) {
            proxy.go_to(url);
        }
    }).state(&address),
)))
```

`proxy.run_javascript(expr)` is async (`Result<Str, Str>`, the JSON of the value) — await
it in `.action_async`. The proxy also has `go_forward`, `refresh`, `stop`, `set_cookie`,
`get_cookies().await` and `set_user_agent`. On a `WebView` handle (not the proxy) — the
value `WebViewController::open()` returns, the controller being an extractor —
`eval!(webview, "expr @{value}").await` decodes the result into a Rust type and
`exec!(webview, "program").await` runs for effect (`exec_file!(webview, "scripts/setup.js")`
is the fused `webview.exec(&js_file!(..))`, awaited the same way); `@{..}` holes are passed as bound
arguments, never spliced into the source. `with_proxy` always places the page below the
content; on such a handle `webview.proxy_scope(|| controls)` attaches the proxy and leaves
placement to you.

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
    // fn returning a signal  ->  mirrored state: after `await waterui.ready`, `waterui.state.address`, `waterui.watch("address", cb)`
    fn address(&self) -> Binding<Str> { self.address.clone() }        // Binding: JS writes flow back
    fn greetings(&self) -> Computed<u32> { self.greetings.clone().computed() }   // read-only mirror
}

WebView::open(url).serve(PageApi { address, greetings })
```

A page receives mirrored values with the reply to its own first request, never in the
script injected into it: `waterui.ready` resolves once they have arrived, and reading or
assigning `waterui.state.<key>` before that throws. Await it before the first read.

A typed `Json<T>` payload requires `T: Serialize`/`Deserialize`, so the app needs `serde`
(with the `derive` feature) as a direct dependency. An `async fn` returning `Result<_, E>`
rejects the page's promise on `Err`. `#[js_api(namespace = "app")]` prefixes every name
with `app.` — still on the one `waterui` object: the page calls
`await waterui.invoke("app.greet", { name })` and reads `waterui.state["app.address"]` —
and a method is left out with `#[js(skip)]` or renamed with `#[js(rename = "..")]`. A
synchronous method with arguments is a compile error.
`webview::typescript_declarations::<PageApi>()` renders a `.d.ts` for the surface. The
macro only writes what `.handler(name, closure)` and `.expose(name, signal)` express by
hand.

Which browser engine draws a `WebView` follows the app's own dependencies, not a project
setting. Link nothing and the platform's engine is used (`WKWebView`, Android `WebView`,
`WebKitGTK`). To pick one of WaterUI's bundled engines instead, add `waterui-browser-cef`
(with its `webview` feature) or `waterui-browser-wpe` as a dependency and call its
`install(&mut env)` in `app` before
`App::new` — the `WebView` code above is unchanged either way. Installing two engines
panics: one component, one engine. Linux outside GTK bridges no system engine, so a
`WebView` there needs one of these crates.

`waterui-chromium` is the separate, heavier component for a full Chromium surface,
headless pages, screenshots, and DevTools Protocol access. It is a direct crate
dependency (not a `waterui` feature), paired with `waterui-browser-cef`'s `chromium`
feature; calling `waterui_browser_cef::install_chromium(&mut env)` in `app` installs a
`ChromiumController` into the environment (`env.get::<ChromiumController>()`);
`controller.open(ChromiumConfiguration::new(url))` (`.profile(..)`, `.proxy(..)`,
`.language(..)`, `.user_agent(..)`) returns a value that is both a `View` and a handle (`.page()` — clone the page out *before* the
view moves into the layout); `page.watch(|event: ChromiumEvent| ..)` observes lifecycle
(this is an event subscription, not the reactive `watch`); `page.cdp()` executes raw or
typed CDP commands; `controller.headless(config).await` gives an off-screen page you must
`page.close().await` yourself.

## Graphics and codes

```rust
use waterui::canvas::{Canvas, Circle, StrokeStyle};   // facade feature `canvas`

Canvas::new(move |cx| {                  // runs once per mount; signals stay bound
    cx.fill(cx.size().map(Rect::from_size), Color::srgb(240, 240, 240));
    cx.stroke(radius.map(|r| Circle::new(Point::new(40.0, 40.0), r)), StrokeStyle::new(2.0), accent);
})

use waterui::barcode::Barcode;           // facade feature `barcode` (the crate is also usable directly)
Barcode::qr("https://waterui.dev").size(120.0, 120.0)
Barcode::code128("012345").size(160.0, 60.0)

use waterui::svg::Svg;
Svg::new(source)
```

A `Canvas` closure records once; it never reruns. Every operand takes a value or a signal
(`radius` above is a `Binding<f32>`, `accent` a `Color`), and a signal's change updates
that one operand in place, animated when the change carries an animation — so there is no
redraw call. Structure that depends on data re-records locally: `cx.each(items, key, body)`
keeps one sub-recording per key and hands the body a `Computed<T>` of its item, and
`cx.scope(signal, body)` re-records one region when the signal changes. `cx.size()` and
`cx.time()` are signals. State is scoped by closures — `cx.clip(shape, ..)`,
`cx.transform(affine, ..)`, `cx.layer(Layer::opacity(0.5), ..)` — never saved and restored.
`Drawing::record(size, |cx| ..)` records constants, on any thread, for `cx.draw(&drawing,
transform)` or `Canvas::from(drawing)`, a view of the drawing's size. Images are
`ImageSource`s the target registers itself; `image.status()` reports a refusal. Give a
canvas with a natural size `.intrinsic_size(..)`, and a name with `.offered_label(..)`.
Beyond `fill` and `stroke`: `cx.shadow(shape, Shadow::new(blur, color).offset(dx, dy))`
(`color` is a resolved `waterui::graphics::WorkingColor` such as `WorkingColor::BLACK`,
not a `Color`),
`cx.text(styled, TextPlacement::at(point))` (laid out by the same text stack as every
`Text`; the canvas never measures it) and `cx.glyphs(..)` for pre-shaped runs.

Barcodes: `Barcode::qr(content)` and `Barcode::code128(content)` take a signal, so a
rotating token re-encodes in place. The view *panics while rendering* when the content
exceeds QR capacity or holds characters Code128 cannot encode; validate runtime input
first with `BarcodeSource::qr(text)` / `BarcodeSource::code128(text)`, which return a
`Result`. `.dark_color(c)`, `.light_color(c)` and `.linear_gradient(..)` style it.
`Svg::new(markup)` takes SVG source text (`.tint(color)` recolours it,
`Svg::from_path(path_data, w, h)` draws one path); `waterui::svg` needs the `gpu`
feature, on by default.

Custom GPU content, from cheapest to fullest control:

```rust
use waterui::graphics::shader;
shader!("starfield.wgsl").size(400.0, 500.0)   // static fragment shader from the .wgsl file beside this source file, no build.rs, no wgpu dep
```

`shader!` reads the file next to the calling source file (`include_str!`) and expands to a
`ShaderPaintView` that fills the space it is given. It renders once per change of its
inputs; `.animated(signal)` (a `bool`, `.animated(true)` for constant) re-renders every
frame so `uniforms.time` advances, and `.uniforms(signal)` of a `Vec<f32>` (up to 64
floats) feeds `params`. The WGSL writes `@fragment fn main(@location(0) uv: vec2<f32>) ->
@location(0) vec4<f32>` (`uv` runs 0..1 from the top-left) against the prelude's
`uniforms.time`, `uniforms.resolution` and `params` (`array<vec4<f32>, 16>`), returning
premultiplied linear Display P3; `cherenkov_linear_srgb` converts an sRGB colour. For
full control, implement `GpuContent` (sync `setup(&mut self, gpu: &Context<'_>)` owns persistent GPU
resources; `render(&mut self, frame: &mut Frame<'_>)` draws — call `frame.request_redraw()` at the
end to keep animating) and wrap it: `GpuContentView::new(content).size(w, h)`. The content is `Send`
and lives on the render thread once installed — it must NOT hold `Binding`s or other UI-thread
state; instead read reactive state in a `.on_frame` hook (UI thread, once per frame) and post
plain values to the content through shared `Arc<Mutex<..>>` state. `waterui::graphics` re-exports
`bytemuck`. Verify GPU components with offscreen rendering, never by reasoning about the code.
Give the surface an accessibility name with `.labeled(..)` and a value with `.described(..)`.

Input for self-drawn views (a terminal, an editor, a game board) arrives through
`GpuContentView::on_input(|event: &SurfaceInputEvent| ..)` — a UI-thread `Fn`, so it may
capture cloned bindings — or, for a `SceneContent`, by returning `true` from
`wants_input_events` and handling `input(&mut self, event)`. Positions are logical and
local to the surface. What changes correctness:

```rust
use waterui::graphics::input::{Code, Key, Modifiers, NamedKey, SurfaceInputEvent};

GpuContentView::new(terminal)
    .on_input(move |event| match event {
        // IME and injected insertions arrive as text events, never as `Key`
        SurfaceInputEvent::TextInput(text) | SurfaceInputEvent::CompositionCommit(text) => {
            typed.set(text.clone())
        }
        // `key` is the logical key; `code` is the physical one, for layout-independent bindings
        SurfaceInputEvent::Key { pressed: true, code: Code::KeyC, modifiers, .. }
            if modifiers.contains(Modifiers::CONTROL) => interrupt.set(true),
        SurfaceInputEvent::Key { pressed: true, key: Key::Named(NamedKey::Enter), .. } => submit.set(true),
        // remember the pointer: under `.context_menu` the secondary press never reaches here
        SurfaceInputEvent::PointerMove { position } => pointer_x.set(position.x),
        _ => (),
    })
```

- Text from an input method (`CompositionStart` … `CompositionUpdate` … `CompositionCommit`)
  or an injected insertion (`TextInput`, `Paste`) is not a `Key`; a handler that must block
  input (a keyboard lock, a modal prompt) gates both families.
- `Key` carries the logical `key` (after layout and modifiers) and the physical `code`
  (position on the keyboard, W3C vocabulary from `keyboard-types`).
- Under an enclosing `.context_menu` with items, a secondary press focuses the surface but is
  not delivered as a `PointerButton`: the menu claims it. Build per-click menu contents from
  state the surface already tracks, such as the last pointer position.
- Move input-method candidate panels with `.on_ime_caret(|| Option<Rect>)`, a surface-local
  `waterui::graphics::gpu::Rect`.

Built-in filters are `FilterViewExt` methods on any view (prelude; feature `effects`, on by
default through `gpu`), each parameter an `impl IntoSignalF32` (a literal or a signal) unless
noted; `.filter(F)` applies a value from `waterui::graphics::filter_view::filters` directly.
Attach them after inherent builders like `Photo::on_event`. Auxiliary inputs — `FilterImage`
(`FilterImage::from_rgba8(w, h, bytes)` or `::from_encoded(&bytes)?`), `LutImage`,
`BlendMode`, `TransitionDirection` — are in `waterui::graphics::filter_view`; besides the last
row, `guided_smooth`, `depth_aware_blur`, `temporal_denoise` and `displacement_warp` take a
`FilterImage` too.

| Family | Methods |
|---|---|
| Color and tone | `brightness` `contrast` `saturation` `vibrance` `exposure` `gamma` `grayscale` `sepia` `hue_rotation` `invert()` `temperature_tint(t, tint)` `highlights_shadows(h, s)` `white_point(r, g, b)` `tone_curve(shadows, mids, highlights, gamma, amount)` `color_matrix([[f32; 4]; 3])` `lut_color_grade(LutImage, intensity)` |
| Blur, sharpen, denoise | `blur(radius)` `gaussian_blur(sigma)` `motion_blur(radius, angle)` `zoom_blur(amount, x, y)` `sharpen` `unsharp_mask(..)` `bloom(..)` `gloom(..)` `guided_smooth(..)` `depth_aware_blur(..)` `temporal_denoise(..)` `median3x3()` |
| Edges, kernels | `sobel()` `prewitt()` `edge_work(..)` `convolution3x3([p; 9])` `convolution5x5([p; 25])` (`p: Copy`, so literals) `morphology_min()` `morphology_max()` `morphology_gradient()` |
| Stylize | `pixellate` `crystallize` `vignette(radius, softness)` `dot_halftone(..)` `line_halftone(..)` `kaleidoscope(..)` `mirror_tile(..)` `photo_effect_{mono,noir,chrome,instant,fade,process,tonal,transfer}()` |
| Geometry | `bump_distortion(..)` `pinch_distortion(..)` `twirl_distortion(..)` `vortex_distortion(..)` `displacement_warp(..)` `perspective_transform([[f32; 2]; 4])` `perspective_correction([[f32; 2]; 4])` |
| Composite with an image (`FilterImage`) | `blend_with_image(image, amount, BlendMode)` `masked_blur(mask, ..)` `replace_background(matte, background, softness)` and the `{,swipe_,radial_,zoom_,displacement_}transition_to_image(..)` family |

`.effect(..)` post-processes a view's rendered output with a fragment shader the app writes —
the hook for CRT, scanline, or distortion passes over existing content (a terminal's custom
shader is the canonical case). The WGSL module defines `@fragment fn main(in: VertexOutput)`;
the prelude supplies `VertexOutput` (`position`, `uv`), `input_texture`/`input_sampler`,
`uniforms` (`resolution`, `input_resolution`, `time`, `time_delta`, `frame`, `params`), and
`effect_param(i)`:

```rust
use waterui::graphics::filter_view::ShaderEffect;

let crt = ShaderEffect::new(include_str!("crt.wgsl"))?; // invalid WGSL is an Err, never a blank view
terminal()
    .effect(crt.clone())            // a clone shares the validated module; constant params carry
    .param(strength.clone())        // a signal — the shader reads it as effect_param(0u)
    .animated(focused.clone())      // a bool signal — a frame after every frame while true
```

`ShaderEffect::new` parses and validates the module on the CPU, so a shader read from a config
file is a `Result` handled where it is read. Parameters index in call order — a constant
`.param(0.35)` on the effect counts too, and watched parameters appended by `.param(signal)`
follow it — at most `SHADER_EFFECT_MAX_PARAMS` (16). `.animated()` on the effect itself is the
constant form; the view-level `.animated(signal)` drives the flag reactively. `.effect(..)`
needs the `gpu` feature and a backend that runs GPU effects.

Frames that already live in GPU memory (a decoder's `CVPixelBuffer`, an `AHardwareBuffer`, a
dmabuf) are not drawn with `GpuContent`: implement `ExternalFrameSource` and wrap it in
`ExternalFrameView::new(source)`. `start(&mut self, output: FrameOutput)` runs on the UI thread
whenever the host builds the view's layer (again after a device loss); hand the `FrameOutput` to
the decoder thread, import each plane onto `output.device()`, and `output.present(frame)` a
`cherenkov_gpu::interop::ExternalFrame` carrying its `FrameColor`. The engine samples the planes
in place — no copy, no shader of your own — and the newest frame wins. Stop producing once
`present` returns `RetiredOutput`.

Particles (facade feature `particle`, imported as `use waterui::particle::ParticleSystem;`):

```rust
ParticleSystem::new(10_000)             // max particles
    .emit_from_rect(1.5, 0.1).at(0.5, -0.05)     // positions/sizes are normalized 0..1 view space
    .rate(2_500.0).life(0.6, 0.8).speed(2.5, 4.5)  // (min, max) randomization ranges
    .color(Color::from(Srgb::new(0.8, 0.9, 1.0)).with_opacity(0.4),
           Color::from(Srgb::new(0.85, 0.95, 1.0)).with_opacity(0.0))   // start -> end over lifetime
```

Every numeric builder parameter is a signal slot (`impl IntoSignalF32`; colors take
`impl IntoSignal<Color>`). Careful: on
`ParticleSystem`, `.size(min, max)` is the *particle* size range in normalized units —
it is not the frame modifier. Also available: `.emit_from_point()` / `.emit_from_circle(r)`,
`.angle(min, max)`, `.spin(min, max)`, `.shape(ParticleShape::Circle | Rect)`, `.additive()`,
`.softness(s)`, `.stretch_with_velocity()`, the forces `.gravity(x, y)`, `.wind(x, y)`,
`.turbulence(s)`, `.drag(s)`, and collisions (`.collide_with_viewport()`,
`.collide_with_rect(x, y, w, h)`, `.collide_with_circle_obstacle(x, y, r)`,
`.collide_with_particles(radius, strength)`, `.bounce(r)`, `.surface_friction(f)`).

Shapes are views that fill the space they are given; `.fill()` and `.clip()` are the two
ways to use them — the styling reference covers them, including the normalized-radius
rule for `RoundedRectangle`.

## Data: charts and maps

Charts (facade feature `chart`, `waterui::chart`; also usable as the `waterui-chart` crate).
Two facts govern the whole family:

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
`Candle::new(timestamp, open, high, low, close, volume)`;
`DepthData::new(bids, asks)` of `DepthLevel::new(price, cumulative_qty)`;
`HeatmapData::new(rows, cols, row_major_values)`; `ContourData::new(rows, cols, values,
level_count)`; `RadarData::new(axis_count).labels(..).series(RadarSeries::new(name, values))`
(every vec exactly `axis_count` long); `AreaData::new(x_values).series(..).stacked(true)`;
`BubblePoint::with_color(x, y, size, r, g, b, a)`;
`GaugeData::new(value, min, max).region(GaugeRegion::hex(upper, "#22C55E"))` (regions are
kept sorted by threshold), whose `GaugeChart` takes `.arc(ArcAngles::from_degrees(-135.0,
135.0))` and `.radii(GaugeRadii::new(0.3, 0.45))` (normalized).

Interaction and composition, on every chart: `.focused(&binding)` / `.selected(&binding)`
write the hit datum into a `Binding<Option<HitResult<T>>>` (`series`, `index`, `value`,
`anchor`); `.focused_tooltip(|hit| ..)` and `.selected_tooltip(|hit| ..)` draw a view
anchored at the datum — `Tooltip::new(TooltipContent::new().title(..).value(label, value))`
is the ready-made one; `.chart_overlay(|proxy| ..)` / `.chart_background(|proxy| ..)`
layer arbitrary views, the `ChartProxy` exposing `focused()`, `selected()` and the frame
signals. Cartesian charts also take `.chart_x_selection(&binding)` and friends; a
`Legend::new(vec![LegendItem::new(label, color)])` is a separate view. `LineChart::fill(opacity)`
shades under the line.

Maps (`waterui-map`):

```rust
use waterui_map::{Annotation, Coordinate, Map, MapStyle, Region};

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
`Result`; out-of-range coordinates are errors, not clamps. Also: `Map::centered_on(coordinate)`
and `Map::centered_on_location(location)` (signals; default zoom), `Annotation::new(coordinate,
title).subtitle(..)`, `.is_interactive(false)`, `.shows_user_location(true)`,
`.follows_location(signal)`, and `.status(&Binding<MapStatus>)`, which the map fills with
`Loading | Ready | Failed(reason)` so the app can draw its own spinner or error over it.

`waterui-map` is the semantic component; who *draws* it is the application's choice, made
in `app(env)`. Apple platforms bridge MapKit and need nothing. Every other platform draws
the map itself, from `waterui-map-gpu`, which the app installs and gives a tile source —
there is no built-in one, and without both lines the map draws nothing (`waterui-map-gpu` +
`waterui-url` as direct deps). `MapStyle::Satellite` and `Hybrid` render there only when the
options also carry `.satellite_style_url(url)` / `.hybrid_style_url(url)`; otherwise the map
reports `MapStatus::Failed` instead of drawing the standard style. Tile limits, cache size,
timeout and retry are further `MapGpuOptions` builders. `install` yields to a backend that
already registered a platform map, so the guard below only keeps the GPU map out of Apple
builds:

```rust
use waterui_map_gpu::MapGpuOptions;
env.insert(MapGpuOptions::new(waterui_url::Url::new("https://tiles.openfreemap.org/styles/positron")));
#[cfg(not(target_vendor = "apple"))] // Apple bridges MapKit; keeps the GPU map out of Apple builds
waterui_map_gpu::install(&mut env);
```

Maps need the `internet` permission; showing the user's location needs the `location`
permission declared in `Water.toml` *and* requested at runtime — see
[project.md](project.md).
