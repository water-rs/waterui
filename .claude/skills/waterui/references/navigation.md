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

The worked, compiling example for everything here is `examples/navigation/src/lib.rs` in
the WaterUI repository; windows are `examples/multi_window`.

## The governing idea: chrome is declared, not drawn

Bars, tab strips, sidebars, and search fields are never views you compose out of stacks.
You declare *what* they are, and each backend projects that into its own platform object:
`UITabBarController` on Apple, bottom navigation or a navigation rail on Android, a
notebook on GTK, retained bar chrome in the self-drawn renderers.

The same declaration therefore looks different — correctly — on each platform, and it
adapts within a platform too: `tab_style::automatic()` is a tab bar on a phone and a
sidebar on a desktop-class window; `.large_title()` collapses on scroll on iOS and becomes
a large app bar on Android. Do not try to control that from app code.

The corollary: if a bar looks wrong on one platform, that is a backend defect. Do not
compensate for it with an `hstack` that imitates a bar.

## Tabs

`Tabs` is keyed by your own tab type — you never name an `Id`.

```rust
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Pane { Inbox, Library, Settings }

let pane = binding(Pane::Inbox);

Tabs::new(&pane, vec![
    Tab::container(Pane::Inbox, label("Inbox").icon(mdi::inbox()), inbox_stack)
        .badge(unread_count),
    Tab::container(Pane::Library, label("Library").icon(mdi::image_album()), library_split),
    Tab::container(Pane::Settings, label("Settings").icon(mdi::cog()), settings_stack),
])
.style(tab_style::automatic())
```

**Use `Tab::container`, not a plain tab, whenever a tab has pushable content.** The
container is what gives that tab its own navigation stack, and therefore what makes it
remember where you left it: push twice in Inbox, visit Settings, come back, and both
pushed pages are still there. Without it, switching tabs discards the stack.

## Navigation stacks and routes

A stack is a typed route enum plus a `destination` mapping. Pushing is a value, not a view.

```rust
#[derive(Clone, PartialEq)]
enum MailRoute { Message(u64), Compose }

fn inbox_stack(mail: Mail) -> impl View {
    NavigationStack::with_path(NavigationPath::<MailRoute>::new(), inbox_root(mail.clone()))
        .destination(move |route| match route {
            MailRoute::Message(id) => message_detail(mail.clone(), id),
            MailRoute::Compose => compose_page(mail.clone()),
        })
}
```

The destination mapping produces **`NavigationView`s** — give every destination page a
`.title(..)` (which performs that conversion) or build it with
`NavigationView::new(title, content)`; a helper returning plain `impl View` will not
satisfy the closure.

Two ways to travel:

```rust
NavigationLink::value("Appearance", SettingsRoute::Appearance)   // a tappable row/tile

// …or imperatively, from any handler, via the Navigator extractor:
|navigator: Navigator<MailRoute>| navigator.push(MailRoute::Compose)
```

`Navigator<R>` is installed by the stack itself — no `.state()` call supplies it — and
both extractor spellings work interchangeably in a handler: the bare
`navigator: Navigator<MailRoute>` or `State(navigator): State<Navigator<MailRoute>>`.

A link's first argument is `impl IntoLabel`, so `NavigationLink::value(Label::new(text!(
"{sender}, {subject}"), move || row_content(..)), route)` gives a rich row that still
reads as one accessibility node. Whether a link row shows a disclosure chevron is the
platform's convention, not yours — iOS draws one, macOS does not.

## Going back, and destination lifecycle

```rust
fn send_draft(State(mail): State<Mail>, navigator: Navigator<MailRoute>) {
    mail.send_draft();
    let _ = navigator.pop();      // returns Option<T> and is #[must_use] — bind it
}
```

`on_navigation_appear` / `on_navigation_disappear` are destination-level hooks that fire
when the push/pop *transition completes* — the right place for "opening this page marks
it read", which does not belong in the link's tap handler. They take full extractor
handlers, like `.action`:

```rust
message_detail(mail.clone(), id)
    .title(text!("{subject}"))
    .on_navigation_appear({
        let mail = mail.clone();
        move || mail.mark_read(id)
    })
```

(The generic `.on_appear` / `.on_disappear` view modifiers fire on any mount/unmount;
these two are specifically about the navigation transition.)

## Bar chrome on a destination

Chrome is a set of modifiers on the destination view, and applying `.title(..)` is what
converts a plain view into the concrete **`NavigationView`** — which is therefore the
return type of a helper that builds a destination:

```rust
fn message_list_page(mail: Mail) -> NavigationView {
    message_list(mail)
        .title("Inbox")                             // or .title(text!("{subject}"))
        .large_title()                              // or .inline_title()
        .navigation_subtitle(text!("{unread} unread"))
        .searchable(&query, "Search mail")          // a field inside the bar, not above the content
        .navigation_pop_enabled(can_leave)          // refuse a back gesture reactively
        .on_navigation_pop_attempted(|State(m): State<SnackbarManager>| {
            m.show(Snackbar::new("Finish the draft first"));
        })
}
```

`NavigationView::new(title, content)` is the direct constructor for the same thing — and
the clean way around one sharp edge: `Text` has its own `.title()` (the semantic font
size), which **shadows** the navigation `.title(impl IntoText)`, so a navigation title
applied to a bare `text(..)` fails with "this method takes 0 arguments". Put the title on
the container, where it belongs anyway.

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

`NavigationToolbar::new(vec![items…])` is the vector-constructor alternative. Chain order
matters: `.title(..)` first (it produces the `NavigationView`), then `.searchable(..)`,
then `.navigation_toolbar(..)`.

The complete placement set: `Principal` (title-area content), `PrimaryAction`,
`SecondaryAction`, `Confirmation`, `Cancellation`, `BottomBar`, `Status`,
`TopBarLeading`, `TopBarTrailing`. Pick by meaning — a detail page's non-primary action
belongs in `SecondaryAction`, not hardcoded into `TopBarTrailing`; iOS puts confirmation
on the right and Android puts it in the app bar, and that difference is the point.

## Transitions

How a destination *arrives* is a modifier on the destination:

```rust
.transition(navigation_transition::zoom(id))
.transition(navigation_transition::fade())
.transition(navigation_transition::none())
```

Put a transition on the stack only when every push should move the same way. A **matched
zoom belongs on the destination**, because the pair it names differs per destination —
tile 3 and the page it opens share an identity that tile 4 does not. Mark both halves:

```rust
tile.navigation_transition_source(photo_transition(index))
hero.navigation_transition_destination(photo_transition(index))
```

The shared identity is an `Id` — one of the few places app code touches one. `Id` is
**non-zero and fallible**: `use waterui::id::Id;` then
`Id::try_from(index_i32 + 1).expect("non-zero")` — there is no `Id::new`, and `0` is
rejected, which is why a 0-based index gets `+ 1`.

A destination that declares nothing inherits the stack's transition. A pair whose halves
are not both on screen falls back to the platform default rather than failing.

## Split views

```rust
use waterui::navigation::NavigationSplitView;

let selection: Binding<Option<Album>> = binding(Some(Album::default()));
let sidebar_selection = selection.clone();

NavigationSplitView::new(&selection, move || album_sidebar(sidebar_selection.clone()), album_detail)
    .sidebar_width(ColumnWidth::new(220.0, 280.0, 360.0))   // min, ideal, max
    .style(split_style::prominent_detail())
    .placeholder(|| text("No album selected"))
```

The selection binding is **`Binding<Option<T>>`**, the detail closure receives the
*unwrapped* `T`, and `.placeholder` covers the `None` case — that split is what makes the
type-check work. Like `Tabs`, it is generic over your own selection type; the `Id`
erasure happens below the authoring layer. It adapts to a sliding pane on a phone and
side-by-side columns on a large window. Both closures are re-invoked on rebuild, so state
they read must be owned outside them.

## Windows

A single-window app needs nothing beyond `App::new(view, env)`. For window chrome,
multiple windows, or a window-level toolbar, describe the windows explicitly:

```rust
use waterui::window::{Window, WindowState, WindowStyle};

App::new_with_windows(
    [Window::new("WaterUI Menu Examples", binding(WindowState::Normal), move || scene())
        .toolbar(window_toolbar(&status))],
    env,
)
```

The `Window` builder, precisely:

- `Window::new(title, state, content)` — the title is `impl IntoComputed<Str>` (a
  reactive window title is free), `state` is a `Binding<WindowState>` **by value**, and
  `content` is any `Fn() -> impl View`, so a bare function item works.
- `.style(WindowStyle)` — exactly three variants: `Titled` (default), `Borderless`,
  `FullSizeContentView` (content extends under the title bar). "Frosted" and
  "transparent" are **not** styles — they are backgrounds:
- `.background(..)` accepts a `Color` (a translucent one gives a transparent window) or a
  `Material` (frosted glass; applied to the window's content, best-effort per backend).
- `.resizable(bool)` — plain bool, default `true`. `.min_size(..)`/`.max_size(..)` each
  take one `impl IntoComputed<Size>` (a `Size` or a signal of one, not two floats);
  without a min, the backend derives one by measuring content at a zero proposal.
- `.toolbar(..)` — window-level chrome; it installs `LabelDisplayMode::IconOnly` for its
  items automatically.

`WindowState` variants: `Normal`, `Closed` (**the `Default`**), `Minimized`,
`Fullscreen`. `WindowState` is held in a binding, so opening, closing, minimizing, and
restoring are ordinary reactive state changes:

```rust
button("Open Window")
    .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Normal))
    .state(&window_state)
```

Note the inference: `binding::<WindowState>(WindowState::default())` needs the turbofish
(nothing downstream pins `T`), and `binding(WindowState::Normal)` means the window is
open from the first frame — start from `default()` for a window that opens on demand.

## Windows that open and close

A window created on demand needs two pieces, created **together, at the same level as the
state** (never inside a rebuilt subtree, or the open-once guard resets):

```rust
use waterui::window::{Window, WindowPresentation, WindowState, WindowStyle, conditional_window};

let state = binding::<WindowState>(WindowState::default());
let presentation = WindowPresentation::new(&state);    // guards against duplicate opens

// An INVISIBLE view — it must still be placed in the tree (e.g. a later zstack child).
conditional_window(&presentation, |state| {
    Window::new("Inspector", state, inspector_content)
        .style(WindowStyle::Titled)
        .resizable(true)
})
```

`conditional_window(&presentation, creator)` materializes the native window when the
state leaves `Closed` and tears it down when it returns there; `Normal → Minimized` does
not spawn a duplicate, and it re-arms after closing. It is the one sanctioned use of
subtree-watching in window code — do not reimplement it with `watch`. A window-filling
overlay layer inside a window must use `absolute(..)`, not a content-sized `zstack`, or
edge-anchored children mis-anchor as soon as the window is larger than the content.
