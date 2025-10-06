# Buttons

Buttons turn user intent into actions. WaterUI’s `button` helper mirrors the ergonomics of SwiftUI
while keeping the full power of Rust’s closures. This chapter explains how to build buttons, capture
state, coordinate with the environment, and structure handlers for complex flows.

## Anatomy of a Button

`button(label)` returns a `Button` view. The `label` can be any view—string literal, `Text`, or a
fully custom composition. Attach behaviour with `.action` or `.action_with`.

```rust,ignore
use waterui::prelude::*;

fn simple_button() -> impl View {
    button("Click Me").action(|| {
        println!("Button was clicked!");
    })
}
```

Behind the scenes, WaterUI converts the closure into a `HandlerFn`. Handlers can access the
`Environment` or receive state via `.action_with`.

## Working with State

Buttons often mutate reactive state. Use `action_with` to borrow a binding without cloning it
manually.

```rust,ignore
use waterui::prelude::*;
use waterui::reactive::binding;

fn counter_button() -> impl View {
    let count = binding(0);

    vstack((
        text!("Count: {count}"),
        button("Increment").action_with(&count, |binding| binding.increment(1)),
    ))
}
```

`.action_with(&binding, handler)` clones the binding for you (bindings are cheap handles). Inside
the handler you can call any of the convenience methods exposed by nami (`.increment`, `.toggle`,
`.push`, `.update`, …).

## Passing Data into Handlers

Handlers can receive additional state or values from the environment in any order. Compose them with
other extractors using tuples:

```rust,ignore
use waterui::prelude::*;
use waterui::core::extract::{Use, UseEnv};

#[derive(Clone)]
struct Analytics;

fn delete_button(item_id: Binding<Option<u64>>) -> impl View {
    button("Delete")
        .action_with(&item_id, |id, (Use(analytics), UseEnv(env)): (Use<Analytics>, UseEnv<Environment>)| {
            if let Some(id) = id.get() {
                analytics.track_delete(id);
                env.log("Item deleted");
            }
        })
}
```

> **Tip**: Extractors live in `waterui::core::extract`. They let you pull services (analytics,
> database pools, etc.) from the environment at the moment the handler runs.

## Custom Labels and Composition

Because labels are just views, you can craft rich buttons with icons, nested stacks, or dynamic
content.

```rust,ignore
use waterui::prelude::*;
use waterui::component::layout::{padding::EdgeInsets, stack::hstack};

fn hero_button() -> impl View {
    button(
        hstack((
            text("🚀"),
            text("Launch")
                .size(18.0)
                .padding_with(EdgeInsets::new(0.0, 0.0, 0.0, 8.0)),
        ))
        .padding()
    )
    .action(|| println!("Initiating launch"))
}
```

You can nest buttons inside stacks, grids, navigation views, or conditionals—WaterUI treats them
like any other view.

## Guarding Actions

WaterUI does not currently ship a built-in `.disabled` modifier. Instead, guard inside the handler or
wrap the button in a conditional.

```rust,ignore
use waterui::widget::condition::when;

fn guarded_submit(can_submit: Computed<bool>) -> impl View {
    when(can_submit.clone(), || {
        button("Submit").action(|| println!("Submitted"))
    })
    .or(|| text("Complete all fields to submit"))
}
```

For idempotent operations, simply return early:

```rust,ignore
button("Pay")
    .action_with(&payment_state, |state| {
        if state.is_processing() {
            return;
        }
        state.begin_processing();
    });
```

## Asynchronous Workflows

Handlers run on the UI thread. When you need async work, hand it off to a task:

```rust,ignore
use waterui::prelude::*;
use waterui::task::task;

fn refresh_button() -> impl View {
    button("Refresh").action(|| {
        task(async {
            let data = fetch_from_api().await;
            update_store(data);
        });
    })
}
```

`task` spawns onto the executor configured for your app (see the `task` chapter). Keep the handler
lightweight—schedule work and return.

## Best Practices

- **Keep handlers pure** – Avoid blocking IO or heavy computation directly in the closure.
- **Prefer `action_with`** – It guarantees the binding lives long enough and stays reactive.
- **Think environment-first** – Use extractors when a button needs shared services.
- **Make feedback visible** – Toggle UI state with bindings (loading spinners, success banners) so
  the user sees progress.

Buttons may look small, but they orchestrate the majority of user journeys. Combine them with the
layout and state tools covered elsewhere in this book to build polished, responsive workflows.
