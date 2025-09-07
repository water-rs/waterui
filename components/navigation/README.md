# WaterUI Navigation Components

Lightweight navigation primitives.

## NavigationView

```rust,ignore
use waterui_navigation::{NavigationView, Bar};
use waterui_text::Text;
use waterui_core::{Color, reactive::constant};

let view = NavigationView {
    bar: Bar {
        title: Text::new("My App"),
        color: constant(Color::default()),
        hidden: constant(false),
    },
    content: waterui_core::AnyView::new(/* your content view */),
};

// Convenience
let view2 = waterui_navigation::navigation("Title", /* content */);
```

## NavigationLink

```rust,ignore
use waterui_navigation::NavigationLink;
use waterui_text::text;

let link = NavigationLink::new(
    text("Go to details"),
    || waterui_navigation::navigation("Details", /* content */)
);
```

Notes:
- A minimal Tabs API exists under `waterui_navigation::tab`, but its usage is still evolving and not yet documented here.
