# Navigation

WaterUI provides a simple yet powerful navigation system that allows you to create hierarchical user interfaces. The two core components of this system are `NavigationView` and `NavigationLink`.

## `NavigationView`

A `NavigationView` is a container that displays a navigation bar with a title and a content view. It's the foundation of any screen that is part of a navigation stack.

```rust,ignore
use waterui::prelude::*;
use waterui::components::navigation::navigation_view;

fn my_screen() -> impl View {
    navigation_view("My Screen",
        "Hello, World!"
    )
}
```

This code creates a screen with a navigation bar at the top that displays the title "My Screen". The content of the screen is a simple text view.

## `NavigationLink`

A `NavigationLink` is a button-like view that, when tapped, navigates to a new `NavigationView`. It takes a label for the link and a closure that returns the destination view.

```rust,ignore
use waterui::prelude::*;
use waterui::components::navigation::{navigation_view, navigation_link};

fn master_view() -> impl View {
    navigation_view("Master",
        navigation_link("Go to Detail", || {
            detail_view()
        })
    )
}

fn detail_view() -> impl View {
    navigation_view("Detail",
        "This is the detail view."
    )
}
```

In this example, the `master_view` displays a `NavigationLink` with the label "Go to Detail". When the user taps on this link, the app will navigate to the `detail_view`, which is a new `NavigationView` with the title "Detail". The navigation bar will automatically include a back button to return to the `master_view`.

## Managing the Navigation Stack

WaterUI automatically manages the navigation stack for you. When you use `NavigationLink`, the new view is pushed onto the stack. When the user taps the back button, the current view is popped off the stack, and the previous view is displayed.

You can also programmatically control the navigation stack by using a `Binding` with a `Vec<NavigationView>` as the navigation path.

```rust,ignore
use waterui::prelude::*;
use waterui::components::navigation::{navigation_view, navigation_link};

fn complex_navigation() -> impl View {
    let path = binding(vec![]);

    navigation_view("Root",
        vstack((
            navigation_link("Go to A", move || {
                path.update(|p| p.push(view_a()));
            }),
            navigation_link("Go to B", move || {
                path.update(|p| p.push(view_b()));
            }),
        ))
    )
    .path(path)
}

fn view_a() -> impl View {
    navigation_view("View A", "This is View A")
}

fn view_b() -> impl View {
    navigation_view("View B", "This is View B")
}
```

## Toolbar

The toolbar is a flexible space in the navigation bar where you can add buttons and other controls. This feature is currently under development.

[WIP]
