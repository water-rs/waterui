# Navigation and windows

## Contents

- The governing idea: chrome is declared, not drawn
- Tabs
- Navigation stacks and routes
- Bar chrome on a destination
- Toolbars
- Transitions
- Split views
- Windows

The worked, compiling example for everything here is `examples/navigation/src/lib.rs` in
the WaterUI repository.

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

Two ways to travel:

```rust
NavigationLink::value("Appearance", SettingsRoute::Appearance)   // a tappable row/tile

// …or imperatively, from any handler, via the Navigator extractor:
|navigator: Navigator<MailRoute>| navigator.push(MailRoute::Compose)
```

Whether a link row shows a disclosure chevron is the platform's convention, not yours —
iOS draws one, macOS does not. The link decides; the application does not ask.

## Bar chrome on a destination

Chrome is a set of modifiers on the destination view, and the destination's type is
`NavigationView`.

One sharp edge: `Text` has its own `.title()` (the semantic font size), which **shadows**
the navigation `.title(impl IntoText)`. Applying a navigation title directly to
`text("…")` therefore fails to compile with "this method takes 0 arguments". Put the title
on the container that holds the content, which is where it belongs anyway.

```rust
message_list(mail)
    .title("Inbox")                             // or .title(text!("{subject}"))
    .large_title()                              // or .inline_title()
    .navigation_subtitle(text!("{unread} unread"))
    .searchable(&query, "Search mail")          // a field inside the bar, not above the content
    .navigation_pop_enabled(can_leave)          // refuse a back gesture reactively
    .on_navigation_pop_attempted(|State(m): State<SnackbarManager>| {
        m.show(Snackbar::new("Finish the draft first"));
    })
```

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
        ))
        .item(NavigationToolbarItem::new(
            NavigationToolbarPlacement::BottomBar,
            button("Mark All Read").style(ButtonStyle::Plain).action(mark_all_read),
        )),
)
```

Placements: `PrimaryAction`, `TopBarLeading`, `TopBarTrailing`, `BottomBar`,
`Confirmation`, `Cancellation`, `Status`. Pick by meaning; iOS puts confirmation on the
right and Android puts it in the app bar, and that difference is the point.

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

A destination that declares nothing inherits the stack's transition. A pair whose halves
are not both on screen falls back to the platform default rather than failing.

## Split views

```rust
use waterui::navigation::NavigationSplitView;

NavigationSplitView::new(&selection, || album_sidebar(sel.clone()), album_detail)
    .sidebar_width(ColumnWidth::new(220.0, 280.0, 360.0))   // min, ideal, max
    .style(split_style::prominent_detail())
    .placeholder(|| text("No album selected"))
```

Like `Tabs`, it is generic over your own selection type; the `Id` erasure happens below
the authoring layer. It adapts to a sliding pane on a phone and side-by-side columns on a
large window, so `.placeholder` is what the detail column shows before a first selection.

## Windows

A single-window app needs nothing beyond `App::new(view, env)`. For window chrome,
multiple windows, or a window-level toolbar, describe the windows explicitly:

```rust
use waterui::window::{Window, WindowPresentation, WindowState, WindowStyle};

App::new_with_windows(
    [Window::new("WaterUI Menu Examples", binding(WindowState::Normal), move || scene())
        .toolbar(window_toolbar(&status))],
    env,
)
```

`WindowState` is a binding, so opening, closing, minimizing, and restoring are ordinary
reactive state changes — a button that opens a window just sets it:

```rust
button("Open Window")
    .action(|State(s): State<Binding<WindowState>>| s.set(WindowState::Normal))
    .state(&window_state)
```

`WindowPresentation` and `WindowStyle` control decoration and materials (borderless,
frosted, transparent). A window-filling overlay layer inside a window must use
`AbsoluteLayout`, not a content-sized `zstack`, or edge-anchored children mis-anchor as
soon as the window is larger than the content.
