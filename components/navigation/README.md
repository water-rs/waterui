# WaterUI Navigation

Navigation containers and stack-based routing for WaterUI applications.

## Overview

`waterui-navigation` provides the building blocks for hierarchical navigation patterns commonly found in mobile and desktop applications. It includes stack-based navigation with push/pop semantics, navigation bars with customizable titles and styling, programmatic navigation links, and a tab interface for switching between multiple root views.

The crate operates through a controller-based architecture where native backends implement the `CustomNavigationController` trait to handle platform-specific navigation rendering (iOS UINavigationController, Android Navigation Component, etc.), while the Rust side manages the navigation state and view hierarchy declaratively.

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
waterui-navigation = "0.1.0"
```

Or use the main `waterui` crate which re-exports these components:

```toml
[dependencies]
waterui = "0.1.0"
```

## Quick Start

```rust
use waterui::prelude::*;
use waterui_navigation::{NavigationView, NavigationStack};

pub fn main() -> impl View {
    NavigationStack::new(
        NavigationView::new("Home",
            vstack((
                text("Welcome to the app"),
                // Navigation content here
            ))
        )
    )
}
```

## Core Concepts

### NavigationView

A `NavigationView` combines a navigation bar (with title, color, visibility) and content. This is the fundamental unit of a navigation hierarchy.

```rust
use waterui_navigation::NavigationView;
use waterui_text::Text;

let view = NavigationView::new("Settings",
    vstack((
        text("App Settings"),
        toggle("Enable notifications"),
    ))
);
```

The navigation bar can be customized:

```rust
let mut view = NavigationView::new("Profile", profile_content);
view.bar.color = Computed::new(Color::blue());
view.bar.hidden = Computed::new(true);
```

### NavigationStack

A container that manages a stack of navigation views with push/pop semantics. The simplest form wraps a single root view:

```rust
use waterui_navigation::NavigationStack;

NavigationStack::new(
    NavigationView::new("Root", root_content)
)
```

For programmatic navigation, use a `NavigationPath` to track the stack state:

```rust
use waterui_navigation::{NavigationStack, NavigationPath};
use nami::binding;

#[derive(Clone)]
enum Route {
    Detail(i32),
    Settings,
}

impl View for Route {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Route::Detail(id) => text(format!("Detail {}", id)),
            Route::Settings => text("Settings"),
        }
    }
}

let path = binding(NavigationPath::new());

NavigationStack::with(path.clone(),
    NavigationView::new("Home", home_view)
)
.destination(|route: Route| {
    match route {
        Route::Detail(id) => NavigationView::new(
            format!("Item {}", id),
            detail_view(id)
        ),
        Route::Settings => NavigationView::new(
            "Settings",
            settings_view()
        ),
    }
})
```

Navigate programmatically:

```rust
// Push a new view
path.borrow_mut().push(Route::Detail(42));

// Pop back
path.borrow().pop();

// Pop multiple levels
path.borrow().pop_n(2);
```

### NavigationLink

A declarative link that pushes a new view when tapped. Internally implemented as a button with a navigation action.

```rust
use waterui_navigation::NavigationLink;

NavigationLink::new(
    text("Show Details"),
    || NavigationView::new("Details", detail_content())
)
```

The label can be any view:

```rust
NavigationLink::new(
    hstack((
        image("icon"),
        text("Settings"),
        spacer(),
        text(">"),
    )),
    || NavigationView::new("Settings", settings_view())
)
```

### NavigationController

The `NavigationController` is injected into the environment by the native backend and provides the runtime connection between Rust navigation commands and platform navigation APIs. Views can extract it to perform navigation actions programmatically:

```rust
use waterui_navigation::NavigationController;

button("Go Forward").action(|controller: NavigationController| {
    controller.push(NavigationView::new("Next", next_view()));
})
```

## Tab Interface

The tab system provides multiple independent navigation stacks with a bottom or top tab bar.

```rust
use waterui_navigation::tab::{Tab, Tabs, TabPosition};
use waterui_core::id::{TaggedView, Id};
use nami::binding;

let selection = binding(Id::from(0));

let tabs = Tabs {
    selection,
    position: TabPosition::Bottom,
    tabs: vec![
        Tab::new(
            TaggedView::new(
                Id::from(0),
                hstack((icon("house"), text("Home"))).anyview()
            ),
            || NavigationView::new("Home", home_view())
        ),
        Tab::new(
            TaggedView::new(
                Id::from(1),
                hstack((icon("gear"), text("Settings"))).anyview()
            ),
            || NavigationView::new("Settings", settings_view())
        ),
    ],
};
```

Tab switching is controlled by mutating the `selection` binding:

```rust
selection.set(Id::from(1)); // Switch to second tab
```

## Examples

### Master-Detail Navigation

```rust
use waterui::prelude::*;
use waterui_navigation::{NavigationView, NavigationLink};

struct Item {
    id: i32,
    name: String,
}

fn item_list(items: Vec<Item>) -> impl View {
    NavigationView::new("Items",
        vstack(
            items.into_iter().map(|item| {
                NavigationLink::new(
                    text(item.name.clone()),
                    move || item_detail(item.id)
                )
            }).collect::<Vec<_>>()
        )
    )
}

fn item_detail(id: i32) -> NavigationView {
    NavigationView::new(
        format!("Item {}", id),
        vstack((
            text(format!("Details for item {}", id)),
            button("Delete").action(|controller: NavigationController| {
                // Delete item, then pop back
                controller.pop();
            }),
        ))
    )
}
```

### Programmatic Navigation with State

```rust
use waterui::prelude::*;
use waterui_navigation::{NavigationStack, NavigationPath};

#[derive(Clone)]
enum AppRoute {
    Login,
    Home,
    Profile(String),
}

impl View for AppRoute {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            AppRoute::Login => text("Login Screen"),
            AppRoute::Home => text("Home Screen"),
            AppRoute::Profile(name) => text(format!("Profile: {}", name)),
        }
    }
}

fn app() -> impl View {
    let path = binding(NavigationPath::new());

    NavigationStack::with(path.clone(),
        NavigationView::new("Login",
            button("Log In").action({
                let path = path.clone();
                move || {
                    path.borrow_mut().push(AppRoute::Home);
                }
            })
        )
    )
    .destination(|route: AppRoute| {
        match route {
            AppRoute::Home => NavigationView::new("Home",
                button("View Profile").action({
                    let path = path.clone();
                    move || {
                        path.borrow_mut().push(AppRoute::Profile("User".into()));
                    }
                })
            ),
            AppRoute::Profile(name) => NavigationView::new(
                "Profile",
                vstack((
                    text(format!("Welcome, {}", name)),
                    button("Back to Home").action(move |_: ()| {
                        path.borrow().pop();
                    }),
                ))
            ),
            _ => NavigationView::new("", text("")),
        }
    })
}
```

### Tabbed Interface with Independent Stacks

```rust
use waterui::prelude::*;
use waterui_navigation::tab::{Tab, Tabs, TabPosition};

fn main_tabs() -> impl View {
    let selection = binding(Id::from(0));

    Tabs {
        selection: selection.clone(),
        position: TabPosition::Bottom,
        tabs: vec![
            Tab::new(
                TaggedView::new(Id::from(0), text("Feed").anyview()),
                || NavigationView::new("Feed", feed_view())
            ),
            Tab::new(
                TaggedView::new(Id::from(1), text("Search").anyview()),
                || NavigationView::new("Search", search_view())
            ),
            Tab::new(
                TaggedView::new(Id::from(2), text("Profile").anyview()),
                || NavigationView::new("Profile", profile_view())
            ),
        ],
    }
}
```

## API Overview

### Types

- `NavigationView` - A view with a navigation bar and content
- `NavigationStack<T, F>` - A stack-based navigation container
- `NavigationPath<T>` - Reactive stack state for programmatic navigation
- `NavigationLink<Label, Content>` - Declarative navigation link
- `NavigationController` - Runtime controller for push/pop actions
- `Bar` - Navigation bar configuration (title, color, visibility)

### Tab Types

- `Tabs` - Tab container with selection binding
- `Tab<T>` - Individual tab with label and content
- `TabPosition` - Tab bar position (Top/Bottom)

### Functions

- `navigation(title, view)` - Convenience function to create a `NavigationView`

### Extension Traits

`ViewExt::title(title)` from the main `waterui` crate creates a `NavigationView`:

```rust
use waterui::prelude::*;

content_view.title("Screen Title")
```

## Features

This crate has no optional features. All navigation components are included by default.

## Architecture Notes

Navigation rendering is backend-specific:

- **Apple**: Maps to SwiftUI's `NavigationStack` and `NavigationBar`
- **Android**: Maps to Jetpack Compose's `NavHost` and `TopAppBar`
- **Hydrolysis**: Self-drawn navigation (experimental)

The Rust side provides the declarative API and state management while native backends handle the actual rendering and platform-specific navigation gestures (swipe to go back, etc.).
