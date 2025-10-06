# Conditional Rendering

Declarative UI is all about letting data drive what appears on screen. WaterUI’s conditional
widgets allow you to branch on reactive `Binding`/`Signal` values without leaving the view tree or
breaking reactivity. This chapter covers the `when` helper and its siblings, demonstrates practical
patterns, and highlights best practices drawn from real-world apps.

## Choosing the Right Tool

| Scenario | Recommended API | Notes |
| -------- | --------------- | ----- |
| Show a block only when a boolean is `true` | `when(condition, || view)` | Condition can be any `Signal<bool>` (e.g. `Binding<bool>`, `Computed<bool>`). |
| Provide an `else` branch | `.or(|| fallback)` | Chains onto `when`, returns a `WhenOr` that renders one of two views. |
| Toggle based on an `Option<T>` | `when(option.map(|opt| opt.is_some()), || …)` | Keeps logic declarative, avoids `.get()`. |
| Show a loading indicator while work happens | `when(is_ready.clone(), || content).or(|| loading())` | Compose with asynchronous bindings or suspense. |

## Basic Usage

```rust,ignore
use waterui::prelude::*;
use waterui::widget::condition::when;
use waterui::reactive::binding;

pub fn status_card() -> impl View {
    let is_online = binding(true);

    when(is_online.clone(), || {
        text("All systems operational")
            .foreground(Color::srgb(68, 207, 95))
    })
    .or(|| {
        text("Offline".to_string())
            .foreground(Color::srgb(220, 76, 70))
    })
}
```

`when` evaluates the condition reactively. Whenever `is_online` flips, WaterUI rebuilds only the
branch that needs to change.

### Negation and Derived Conditions

`Binding<bool>` implements `Not`, so you can negate without extra helpers:

```rust,ignore
let show_help = binding(false);
when(!show_help.clone(), || text("Need help?"));
```

For complex logic, derive a computed boolean with `SignalExt`:

```rust,ignore
use nami::SignalExt;

let cart_items = binding::<Vec<CartItem>>(Vec::new());
let has_items = cart_items.map(|items| !items.is_empty());

when(has_items, || checkout_button())
    .or(|| text("Your cart is empty"));
```

The key guideline is **never call `.get()` inside the view tree**; doing so breaks reactivity. Always
produce another `Signal<bool>`.

### Option-Based Rendering

Options are ubiquitous. Transform them into booleans with `map` or unwrap them inline using
`option.then_some` convenience methods:

```rust,ignore
let selected_user = binding::<Option<User>>(None);

when(selected_user.map(|user| user.is_some()), || {
    // Safe to unwrap because the branch only runs when a user exists.
    let profile = selected_user.unwrap_or_else(User::placeholder);
    profile_view(profile)
})
.or(|| placeholder_tile())
```

`Binding<Option<T>>::unwrap_or_else` (from nami) returns a new binding that always contains a value
and wraps writes in `Some(_)`, which can simplify nested UI.

### Conditional Actions

Conditional widgets are themselves views, so you can embed them anywhere a normal child would
appear:

```rust,ignore
pub fn dashboard() -> impl View {
    let has_error = binding(false);

    vstack((
        header(),
        when(has_error.clone(), || error_banner()),
        content(),
    ))
}
```

Combine `when` with `button` actions for toggles:

```rust,ignore
let expanded = binding(false);

vstack((
    button("Details").action_with(&expanded, |state| state.toggle()),
    when(expanded, || detailed_view()),
))
```

### Avoid Side-Effects Inside Closures

The closures you pass to `when` should be pure view builders. Mutating external state or launching
async work from inside introduces hard-to-debug behaviour. Instead, trigger those effects from
button handlers or tasks, then let the binding drive the conditional view.

## Advanced Patterns

- **Multiple Conditions** – Nest `when` calls or build a `match`-style dispatcher using `match` on
  an enum and return different views for each variant.
- **Animations & Transitions** – Wrap the conditional content in your animation view or attach a
  custom environment hook. WaterUI will destroy and recreate the branch when toggled, so animations
  should capture their state in external bindings if you want continuity.
- **Layouts with Placeholders** – Sometimes you want the layout to remain stable even when the
  branch is hidden. Instead of removing the view entirely, render a transparent placeholder using
  `when(condition, || view).or(|| spacer())` or a `Frame` with a fixed size.

## Troubleshooting

- **Blinking Content** – If you see flashing during rapid toggles, ensure the heavy computation
  lives outside the closure (e.g. precompute data in a `Computed` binding).
- **Impossible Branch** – When you *know* only one branch should appear, log unexpected states in
  the `or` closure so you catch logic issues early.
- **Backend Differences** – On some targets (notably Web) changing the DOM tree may reset native
  controls. Preserve user input by keeping the control alive and toggling visibility instead of
  removing it entirely.

Conditional views are a small API surface, but mastering them keeps your UI declarative and predictable.
Use them liberally to express application logic directly alongside the view structure.
