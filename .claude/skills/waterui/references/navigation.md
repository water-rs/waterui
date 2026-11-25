# Navigation and windows

## Contents

- The governing idea: chrome is declared, not drawn
- Tabs
- Navigation stacks and routes
- Going back, and destination lifecycle
- Bar chrome on a destination
- Toolbars
- Transitions
- Split views
- Windows
- Windows that open and close
- Resident apps and windows on demand

The worked, compiling example for everything here is `examples/navigation/src/lib.rs` in
the WaterUI repository; windows are `examples/multi_window`.

## The governing idea: chrome is declared, not drawn

Bars, tab strips, sidebars, and search fields are never views you compose out of stacks.
You declare *what* they are, and each backend projects that into its own platform object:
`UITabBarController` on Apple, bottom navigation or a navigation rail on Android, a
notebook on GTK, retained bar chrome in the self-drawn renderers.

The same declaration therefore looks different — correctly — on each platform, and it
adapts within a platform too: `TabStyle::Automatic` is a tab bar on a phone and a
sidebar on a desktop-class window; `.large_title()` collapses on scroll on iOS and becomes
a large app bar on Android. Do not try to control that from app code.

The corollary: if a bar looks wrong on one platform, that is a backend defect. Do not
compensate for it with an `hstack` that imitates a bar.

## Tabs

`Tabs::new(&selection, tabs)` takes a tab set, and each tab keeps the type of the
builder that makes its root, so a tab reaches the backend without an `AnyView`:

- untagged tabs (`(Tab::new(..), Tab::new(..))`, `Vec<Tab>`, arrays, or `ForEach`) fall
  back to positional `Binding<usize>` (or `ForEach` item) identity;
- chaining `.tag(value)` keys any tuple, `Vec`, array, or `ForEach` by your own `T`
  (such as a `Pane` enum) without `SelfId` or `OneOfN`:

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pane { Inbox, Library, Settings, Search }

let pane = binding(Pane::Inbox);

Tabs::new(
    &pane,
    (
        Tab::new(label("Inbox").icon(mdi::inbox()), inbox_stack)
            .tag(Pane::Inbox)
            .badge(unread_count.clone()),
        Tab::new(label("Library").icon(mdi::image_album()), library_split).tag(Pane::Library),
        Tab::new(label("Settings").icon(mdi::cog()), settings_stack).tag(Pane::Settings),
        Tab::new(label("Search").icon(mdi::magnify()), search_stack)
            .tag(Pane::Search)
            .role(TabRole::Search), // iOS: the trailing system search tab; elsewhere a regular tab
    ),
)
.style(TabStyle::Automatic)
.minimize_behavior(TabBarMinimizeBehavior::OnScrollDown)   // iOS 26 collapses the bar while scrolling; ignored elsewhere
.bottom_accessory(now_playing_bar())        // iOS 26 glass bar above the tab bar (mini-player slot); not shown elsewhere
```

A `Tab` is `Tab::new(label, content)` (`label` is `impl IntoLabel`, `content` any
`Fn() -> impl View`) with `.badge(value)` (`u32`, `BadgeValue::{Hidden, Dot, Count, Text}`, or signal; `0` hides) and `.enabled(bool)` signals and `.role(..)`.
Other styles: `TabStyle::TabBar` and `TabStyle::Sidebar` force one presentation;
`TabBarMinimizeBehavior` is `Automatic | Never | OnScrollDown | OnScrollUp`.

**Give each tab its own navigation container when it pushes or splits** — a
`NavigationStack` or a `NavigationSplitView`, like `inbox_stack` above. Each tab owning
its own container is what makes it remember where you left it: push twice in Inbox, visit
Settings, come back, and both pushed pages are still there. A single-page tab can return
a `NavigationView` or any plain `impl View` directly.

## Navigation stacks and routes

A stack is a typed route enum plus a `destination` mapping. Pushing is a value, not a view.

```rust
#[derive(Clone, PartialEq)]
enum MailRoute { Message(u64), Compose }

fn inbox_stack(mail: Mail) -> impl View {
    NavigationStack::with_path(NavigationPath::<MailRoute>::new(), inbox_root(mail.clone()))
        .destination(move |route| match route {
            MailRoute::Message(id) => message_detail(mail.clone(), id).map_content(OneOf2::A),
            MailRoute::Compose => compose_page(mail.clone()).map_content(OneOf2::B),
        })
}
```

The stack's root and every destination are **`NavigationView`s** — give each page a
`.navigation_title(..)` (which performs that conversion) or build it with
`NavigationView::new(title, content)`; a helper returning plain `impl View` satisfies
neither the root parameter nor the destination closure. A `NavigationView<V, T = ()>` keeps its
content and toolbar types, so a helper returns `NavigationView<impl View>` (or
`NavigationView<impl View, impl NavigationToolbarItems>` when it attaches `.navigation_toolbar(..)`),
and destinations whose
pages differ return the variants of one `OneOfN`:
`message_detail(..).map_content(OneOf2::A).map_toolbar(OneOf2::A)` and
`compose_page(..).map_content(OneOf2::B).map_toolbar(OneOf2::B)`.
The stack builds each pushed page for its backend from the route, so no page is erased.

Two ways to travel:

```rust
NavigationLink::value("Appearance", SettingsRoute::Appearance)   // a tappable row/tile

// …or imperatively, from any handler, via the Navigator extractor:
|navigator: Navigator<MailRoute>| navigator.push(MailRoute::Compose)
```

`Navigator<R>` is itself an extractor — the stack publishes it through the `.state()`
channel — so a handler takes it bare: `navigator: Navigator<MailRoute>`. The wrapped
spelling `State(navigator): State<Navigator<MailRoute>>` resolves to the same value.

Beyond `push` and `pop`, a `Navigator<R>` has `pop_n(n)` (panics when `n` exceeds the
path's length), `clear()` / `pop_to_root()` and `replace(routes)` (the whole path in one
transaction). `NavigationPath` itself has `push`, `pop`, `pop_n`, `clear` and `replace`,
plus `len`, `is_empty`, `snapshot`, `truncate`, and `NavigationPath::from(vec![..])` for a
stack that starts deep. A route-free link, for a stack with no route type, is
`NavigationLink::new(label, || destination_view)` inside `NavigationStack::new(root)`; the
closure returns a `NavigationView`.

A link's label is `impl IntoLabel`, so `NavigationLink::value(Label::new(text!(
"{sender}, {subject}"), move || row_content(..)), route)` gives a rich row that still
reads as one accessibility node. Whether a link row shows a disclosure chevron is the
platform's convention, not yours — iOS draws one, macOS does not.

A deep link is a `NavigationRouter` over the stack's path: each `.route(..)` resolver maps a
`Url` to the complete route sequence it owns (or `None`), and `open` replaces the path in
one transaction. It returns `false` when no resolver claims the URL and panics when two do.

```rust
use waterui::Url;

let path = NavigationPath::<MailRoute>::new();
let router = NavigationRouter::new(path.clone())
    .route(|url| (url.path() == "/compose").then(|| vec![MailRoute::Compose]));

let opened = router.open(&Url::new("waterui://app/compose"));
```

With the `navigation-restoration` feature a path is also serde-serializable (its routes
must be `Serialize + DeserializeOwned`), for restoring a stack across launches.

## Going back, and destination lifecycle

```rust
fn send_draft(mail: Mail, navigator: Navigator<MailRoute>) {
    mail.send_draft();
    navigator.pop();              // returns the popped route as Option<T>, `None` at the root
}
```

`Mail` is the app's own `#[state]`-marked model, so it arrives bare alongside the
navigator rather than wrapped in `State<..>`.

`on_navigation_appear` / `on_navigation_disappear` are destination-level hooks that fire
when the push/pop *transition completes* — the right place for "opening this page marks
it read", which does not belong in the link's tap handler. They take full extractor
handlers, like `.action`:

```rust
message_body(mail.clone(), id)
    .navigation_title(text!("{subject}"))
    .on_navigation_appear({
        let mail = mail.clone();
        move || mail.mark_read(id)
    })
```

(The generic `.on_appear` / `.on_disappear` view modifiers fire on any mount/unmount;
these two are specifically about the navigation transition.)

## Bar chrome on a destination

Chrome is a set of modifiers on the destination view, and applying `.navigation_title(..)` is what
converts a plain view into the concrete **`NavigationView`** — which is therefore the
return type of a helper that builds a destination:

```rust
fn message_list_page(mail: Mail) -> NavigationView<impl View> {
    message_list(mail)
        .navigation_title("Inbox")                  // or .navigation_title(text!("{subject}"))
        .large_title()                              // or .inline_title()
        .navigation_subtitle(text!("{unread} unread"))
        .searchable(&query)
        .search_prompt("Search mail")               // a field inside the bar, not above the content
        .search_active(&searching)                  // two-way Binding<bool>
        .on_search_submit(run_query)                // fires when the user commits the query
        .navigation_pop_enabled(can_leave)          // refuse a back gesture reactively
        .on_navigation_pop_attempted(|m: SnackbarManager| {
            m.show(Snackbar::new("Finish the draft first"));
        })
}
```

`SnackbarManager` is `#[state]`-marked — attach `.snackbar_host()` to an ancestor view —
so the handler takes it bare rather than wrapped in `State<..>`.

The rest of the destination's chrome: `.navigation_title_display_mode(NavigationTitleDisplayMode::Medium)`
(the mode between `.inline_title()` and `.large_title()`; the default is `Automatic`),
`.navigation_bar_visibility(visible)` (a bool signal; pages inside a wrapper hide their own bar with `false`), `.navigation_bar_color(color)` (a `Color`
signal that overrides the bar surface, which otherwise follows the `Surface` token) and
`.on_navigation_pop(handler)` (fires only after a completed pop removed this destination).

`NavigationView::new(title, content)` — also the free function `navigation(title, content)` — is the direct constructor for the same thing. (The view-extension modifier is `navigation_title` so `Text`'s zero-argument `.title()` typography preset stays unambiguous.)

`.searchable` binds the query directly, so filtering is an ordinary derived signal over
that binding — no event plumbing.

## Toolbars

Toolbar items carry a *semantic* placement. Each platform decides where that lands.

```rust
.navigation_toolbar(
    NavigationToolbar::default()
        .item(NavigationToolbarItem::new(
            NavigationToolbarPlacement::TopBarLeading,
            button(text!("{edit_title}")).style(ButtonStyle::Plain).action(toggle_editing),
        ))
        .item(NavigationToolbarItem::action(
            NavigationToolbarPlacement::PrimaryAction,
            label("Compose").icon(mdi::pencil()),
            |navigator: Navigator<MailRoute>| navigator.push(MailRoute::Compose),
        )),
)
```

`NavigationToolbar::new(vec![items…])` is the vector-constructor alternative.
`.navigation_toolbar(..)`, like `.searchable(..)`, is a `NavigationView` method, so
`.navigation_title(..)` comes first. `NavigationToolbarItem::action(placement, label, handler)` is
the semantic form — its name and icon reach each platform's native item — while
`NavigationToolbarItem::new(placement, view)` places an arbitrary view as given.

The complete placement set: `Principal` (title-area content), `PrimaryAction`,
`SecondaryAction`, `Confirmation`, `Cancellation`, `BottomBar`, `Status`,
`TopBarLeading`, `TopBarTrailing`. Pick by meaning — a detail page's non-primary action
belongs in `SecondaryAction`, not hardcoded into `TopBarTrailing`; iOS puts confirmation
on the right and Android puts it in the app bar, and that difference is the point.

## Transitions

How a destination *arrives* and leaves is `.page_transition(..)` on the destination (named separately so it never shadows `ViewExt::transition(..)`). It accepts both built-in platform hints and any core `Transition` or `TransitionSpec`:

```rust
.page_transition(navigation_transition::zoom(PhotoRoute(index)))
.page_transition(navigation_transition::fade())
.page_transition(navigation_transition::none())
.page_transition(PropertyTransition::opacity())
```

`navigation_transition::automatic()` is the platform default. Put a transition on the stack
(`NavigationStack::page_transition`) only when every push should move the same way. A **matched
zoom belongs on the destination**, because the pair it names differs per destination —
tile 3 and the page it opens share an identity that tile 4 does not. Mark both halves with
the application's own hashable key type (`K: Hash + Eq + 'static`, such as your route value):

```rust
tile.navigation_transition_source(PhotoRoute(index))
hero.navigation_transition_destination(PhotoRoute(index))
```

Each `NavigationStack` / `NavigationSplitView` resolves matching `(TypeId::of::<K>(), key)`
pairs into internal non-zero `Id`s for the backend, so application code never constructs raw `Id`s.

A destination that declares nothing inherits the stack's transition. `.page_transition(..)`
on a `NavigationView` takes precedence over a `.transition(..)` on its content root without
conflicting. A pair whose halves are not both on screen falls back to the platform default
rather than failing.

## Split views

```rust
use waterui::navigation::{NavigationSplitStyle, NavigationSplitView};

let selection: Binding<Option<Album>> = binding(Some(Album::default()));
let sidebar_selection = selection.clone();

NavigationSplitView::new(&selection, move || album_sidebar(sidebar_selection.clone()), album_detail)
    .sidebar_width(ColumnWidth::new(220.0, 280.0, 360.0))   // min, ideal, max
    .style(NavigationSplitStyle::ProminentDetail)
    .placeholder(|| text("No album selected"))
```

The selection binding is **`Binding<Option<T>>`** (`T: Clone + Ord`), the detail closure
receives the *unwrapped* `T` and returns a `NavigationView`, and `.placeholder` (any
`View + Clone`) covers the `None` case — that split is what makes the type-check work.
Like `Tabs`, it is generic over your own selection type; the `Id` erasure happens below
the authoring layer. It adapts to a sliding pane on a phone and side-by-side columns on a
large window. The sidebar and detail are re-invoked on rebuild, so state they read must
be owned outside them.

`NavigationSplitView::three_column(&sidebar_selection, &content_selection, sidebar,
content, detail)` adds a middle column with its own `Binding<Option<C>>`; the `content`
closure maps the sidebar selection to a `NavigationView`. `.column_visibility(..)` takes a
`NavigationSplitColumnVisibility` (`Automatic | All | DoubleColumn | DetailOnly`) or a
signal of one. Styles: `NavigationSplitStyle::{Automatic, Balanced, ProminentDetail}`.
`ColumnWidth::new` panics unless `0 < min <= ideal <= max`.

At compact widths a two-column split shows exactly one column, chosen by the
selection binding: `Some` shows the detail and `None` shows the sidebar. The
platform back control, including a completed swipe back, writes `None`. The
binding is the single source of truth on every backend: start with `None` if the
app should open on the sidebar. At wider widths the columns remain side by side.

## Windows

Declare an application's windows directly on `App::new(env)` with `.window(...)`:

```rust
use waterui::window::{Window, WindowStyle};

App::new(env).window(
    Window::new("WaterUI Menu Examples", move || scene())
        .toolbar(window_toolbar(&status)),
)
```

The windows are a window set, and `fn app(env: Environment) -> App<impl Windows>` (with
`waterui::app::{App, Windows}`) returns it unnamed: one `Window`, a tuple of windows whose
contents each keep their own type, an array or `Vec` of windows with one content type, or
`()` for none. `App::new(env)` starts empty and `.window(w)` appends each window or
`WindowChannel`.
Each window's content and toolbar keep their types up to the backend, so nothing in the tree is erased.

The `Window` builder, precisely:

- `Window::new(title, content)` — the title is `impl IntoText` (literals, `text!(...)`,
  and reactive strings localize through the environment's `TranslationCatalog`), and
  `content` is any `Fn() -> impl View`, so a bare
  function item works. The window owns an internal `WindowState::Normal` binding by
  default; chain `.state(&window_state)` when external code reads or drives its
  `Binding<WindowState>`. An empty title is shown as the application's own name;
  `.title(..)` sets it later.
  Windows mount only their own content; attach `.snackbar_host()` to a root view
  when the window uses snackbars.
- `.style(..)` — a `WindowStyle` or a `Binding<WindowStyle>`; exactly three variants:
  `Titled` (default), `Borderless`, `FullSizeContentView` (content extends under the
  title bar). The style is reactive: setting the binding (or
  `window.handle().set_style(..)`) re-applies it to the open window, e.g. to toggle
  decorations. "Frosted" and "transparent" are **not** styles — they are backgrounds:
- `.background(..)` accepts a `Color` (a translucent one gives a transparent window), a
  `Material` (the window's material), a `WindowBackground` (`Opaque` is the theme
  background; `Color(..)` and `Material(..)` are the other two) or a
  `Binding<WindowBackground>`. A material can be realized within the window or behind
  it, according to its level: `Regular`, `Thick` and `UltraThick` frost the window's own opaque background behind
  its content; `UltraThin` and `Thin` make the window translucent so
  the desktop shows through — blurred on macOS on the Apple and Hydrolysis
  backends, through the DWM's acrylic system backdrop on Windows 11 22H2 and
  later, under an X11 window manager honouring `_KDE_NET_WM_BLUR_BEHIND_REGION`
  (KWin), and under a Wayland compositor advertising
  `ext-background-effect-v1` and applying its blur by its own policy; older
  Windows, other X11 window managers and Wayland compositors not advertising
  the global leave the window tinted with the level's colour but unblurred.
  Hydrolysis Android realizes within-window levels; behind-window levels render
  as an opaque window until window transparency is supported. The background is reactive:
  setting the binding (or `window.handle().set_background(..)`) re-applies it to the
  open window, e.g. to switch between opaque, a translucent colour and a material.
- `.icon(..)` — a `WindowIcon` (`WindowIcon::new(width, height, rgba)`, straight-alpha
  RGBA8 pixels; it returns a `Result`, an error for an empty icon or a buffer that is not
  `width * height * 4` bytes), an `Option<WindowIcon>` or a `Binding<Option<WindowIcon>>`; `None`, the default, keeps the
  application icon. Reactive like the style: `window.handle().set_icon(..)` swaps it on
  the open window. Only platforms with a per-window icon show it (X11 and Windows);
  Wayland, macOS, iOS and Android always show the application icon.
- `.resizable(bool)` — plain bool, default `true`. `.min_size(..)`/`.max_size(..)` each
  take one `impl IntoComputed<Size>` (a `Size` or a signal of one, not two floats);
  without a min, the backend derives one by measuring content at a zero proposal.
- `.toolbar(..)` — window-level chrome; it installs `LabelDisplayMode::IconOnly` for its
  items automatically.
- `.level(..)` — one `impl IntoComputed<WindowLevel>`; `WindowLevel::Normal` (default)
  or `AlwaysOnTop` to float above other applications' windows.
- `.resize_increments(..)` — one `impl IntoComputed<Size>`; the window's content size
  then snaps to those steps while the user resizes.
- `.placement(MonitorSelector, |monitor: &Monitor| -> Rect)` — each time the window is
  shown the backend resolves the monitor (`Primary` default, `Pointer`, `Focused`), calls
  the closure with its geometry (`frame()`, `visible_frame()` minus desktop chrome,
  `scale_factor()`, `name()`) and uses the returned `Rect` as the window frame.
  Hydrolysis desktop and GTK only; Wayland honours just the size.
- `.activation(Activation)` — `OnShow` (default: showing takes focus), `OnClick` (showing
  does not, a click does) or `Never`, for a window that must not steal what the user is
  typing, such as a drop-down terminal.
- `.app_id(..)` / `.instance_name(..)` — the identity the desktop groups the window under
  (X11 `WM_CLASS` class and instance, Wayland `app_id`). `app_id` defaults to the
  application's identifier and `instance_name` to the resolved `app_id`; Wayland carries
  only `app_id`. Ignored on macOS, iOS, Android and Windows.
- `.closable(bool)` — plain bool, default `true`: when `false`, the close button is
  disabled on macOS and Windows and close requests are ignored on Linux.
- `.default_size(Size)` — initial content size (defaults to `800×600`); `.frame(&binding)`
  binds the window's `Binding<Rect>` when external code reads or drives its geometry.
- `.attention(&binding)` — binds `Binding<Option<UserAttention>>`: setting it asks the
  platform to draw the user to the window (taskbar flash, Dock bounce) and the backend
  resets it to `None` when the window gains focus; `WindowHandle::request_attention(..)`
  writes the same binding.

`WindowState` variants: `Normal`, `Closed` (**the `Default`**), `Minimized`,
`Maximized`, `Fullscreen`. `WindowState` is held in a binding, so opening, closing,
minimizing, maximizing, and restoring are ordinary reactive state changes.

Every backend installs a window's `WindowHandle` into the environment of its content,
toolbar, and `on_close_request` handler, so handlers inside the window extract
`|window: WindowHandle|` directly without calling `window.handle()` beforehand.
`Window::handle()` still returns a `WindowHandle` for callers outside the window's tree.
`WindowHandle` provides: `close()`, `request_close()`, `minimize()`, `maximize()`,
`fullscreen()`, `restore()`, `focus()` (bring to front and give it the keyboard — "show
the existing settings window"; panics before the window is realized and on headless
runners), `request_attention(UserAttention::Informational | UserAttention::Critical)`
(the backend clears it when the window gains focus), `cancel_attention()`, `set_frame(Rect)`,
`set_style(WindowStyle)`, `set_background(..)`, and `set_icon(..)`.

```rust
button("Open Window")
    .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Normal))
    .state(&window_state)
```

Note the inference: `binding::<WindowState>(WindowState::default())` needs the turbofish
(nothing downstream pins `T`), and `binding(WindowState::Normal)` means the window is
open from the first frame — start from `default()` for a window that opens on demand.

A window that must confirm or veto a close — a terminal with a running shell, an
editor with unsaved buffers — gates it with `.on_close_request(..)`, which mirrors
`App::on_quit_request`: the title-bar close button, a window-manager close, the Close
Window menu command and `handle().request_close()` all ask the handler, which takes
extractors like any action handler and answers a `CloseReply` from its future.
`Close` closes the window; `Cancel` leaves it open. One question is open per window
at a time, and further requests are dropped until it is answered. `handle().close()`
and writing `WindowState::Closed` to the binding close without asking — that is how
the window closes itself after the user confirms — and cancel an open question.
Quitting the application asks `on_quit_request`, not the windows. The handler runs on
Hydrolysis desktop and AppKit; Android, UIKit and web have no window close request,
so it never runs there and `request_close()` panics.

```rust
use waterui::window::CloseReply;

let unsaved = binding(true);
// While set, the editor shows a "Discard changes?" prompt whose Discard button
// writes `WindowState::Closed` — a close that does not ask again.
let confirm_discard = binding(false);
let window_state = binding(WindowState::Normal);

Window::new("Editor", editor_content)
    .state(&window_state)
    .on_close_request(move || {
    let ask = unsaved.snapshot();
    confirm_discard.set(ask);
    async move {
        if ask {
            CloseReply::Cancel
        } else {
            CloseReply::Close
        }
    }
})
```

## Windows that open and close

A single-instance secondary window (Settings, Inspector, Preferences) is declared directly on
`App` and driven by its `Binding<WindowState>`:

```rust
use waterui::app::{App, Windows};
use waterui::window::{Window, WindowState, WindowStyle};

fn app(env: Environment) -> App<impl Windows> {
    let inspector_state = binding::<WindowState>(WindowState::default()); // starts Closed
    let main_state = inspector_state.clone();
    App::new(env)
        .window(Window::new("Main", move || {
            button("Inspector")
                .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Normal))
                .state(&main_state)
        }))
        .window(
            Window::new("Inspector", inspector_content)
                .state(&inspector_state)
                .style(WindowStyle::Titled)
                .resizable(true),
        )
}
```

Writing `WindowState::Normal` while `Closed` creates the platform window; writing
`WindowState::Normal` while it is already open restores and focuses it without creating a
duplicate; writing `WindowState::Closed` (or closing the window via its title bar) tears down
the platform window while keeping the declaration armed to reopen on the next `Normal` write.
A window-filling overlay layer inside a window must use `absolute(..)`, not a content-sized
`zstack`, or edge-anchored children mis-anchor as soon as the window is larger than the
content.

## Resident apps and windows on demand

`App::new(env)` starts with no windows, and `.window(..)` adds any number of windows or
channels. What the app does
once it has no open window is `App::on_last_window_closed(LastWindowPolicy)` from
`waterui::app`: `Quit` (the default, on every platform, and immediately for an app that
declares no open window) or `StayResident` (macOS Dock convention, a tray or status-item app,
an app that opens windows on demand). iOS and Android have no windowless foreground state:
their runners reject an app that declares no open window whatever its policy.

For unbounded multi-instance windows (new document or scratch windows where every click opens
a fresh window), `WindowManager::new()` returns a `(WindowManager<C>, WindowChannel<C>)`
pair: pass `channel` to `App::window(channel)` so the runner monomorphizes `C` directly
into its root window type, and call
`window.show(&windows)` from any handler — including `App::menu_bar` commands, or nested
handlers when `.state(&windows)` is attached:

```rust
use waterui::app::{App, AppMenuBar, LastWindowPolicy, QuitReply, Windows};
use waterui::window::{Window, WindowManager};

fn app(env: Environment) -> App<impl Windows, impl AppMenuBar> {
    let (windows, channel) = WindowManager::new();
    App::new(env)
        .window(channel)
        .on_last_window_closed(LastWindowPolicy::StayResident)
        .on_quit_request(|| async { QuitReply::Quit })
        .on_terminate(|| async {})
        .menu_bar(Menu::new(
            "App",
            "New Window".action(move || {
                Window::new("Scratch", scratch_content).show(&windows);
            }),
        ))
}
```

`Window::new` starts in `WindowState::Normal`, so `show` opens it immediately. Showing a
window after its `WindowChannel` was dropped without being handed to a runner is a contract
violation (a debug panic, a logged error in release).

Quitting: `App::on_quit_request(handler)` is asked on a *cancellable* quit (menu Quit,
the platform quit gesture, `MenuItem::Quit`, `Quit::request`) and its future must return
`waterui::app::QuitReply` — `Quit` lets termination proceed, `Cancel` vetoes it. A
*required* termination (termination signal, last window closing under
`LastWindowPolicy::Quit`, a Windows session ending) never asks.
`App::on_terminate(handler)` runs exactly once — after `on_quit_request` answers `Quit`,
or straight away for a required termination — and the runner waits for its future (which
returns `()`) before tearing down, so it is the place to persist state and release
resources. Both handlers take extractors; iOS, Android and web
kill the process and call neither. Any menu or action handler can quit the app by
extracting the `waterui::app::Quit` handle and calling `quit.request()` (it files a
cancellable request, so `on_quit_request` still answers; extraction fails on iOS, Android,
web and headless hosts).

`waterui::app::application_name()` and `application_identifier()` return a `Str`, empty
when nothing told the app. The name is the project's `Water.toml` package name, read at
launch from the environment the `water` CLI sets when it runs the app, so a bundle opened
directly or a test harness reports it empty. The identifier is the reverse-DNS
`bundle_identifier`, compiled in from `WATERUI_APP_ID` at build time, so it is empty for a
build that did not set that variable. Treat empty as "ask the platform".
