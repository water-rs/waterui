# waterui-controls

The interactive controls every WaterUI app is built from: buttons, toggles, sliders, steppers, text fields, and menus.

## Overview

`waterui-controls` supplies WaterUI's foundational control set. Each control is bound
to reactive state — a `Binding<T>` the control reads and writes, or an
`impl IntoComputed<T>` for the values that only flow inward — so a control and the
state behind it never drift apart and never need a manual refresh.

Two rules shape the whole API surface. **Every control takes a semantic label at
construction**, because a control with no name is unreachable for screen readers,
voice control, and command palettes; when the label should not be drawn, hide it
with `.hide_label()` and it stays in the accessibility tree. And **style is an
attribute, not a type**: a checkbox is `Toggle::new(&flag).checkbox()`, not a
separate `Checkbox` component, so switching the presentation never changes what
the control means.

The crate is `no_std` (it needs only `alloc`) and holds no rendering code of its
own. Every control lowers to a configuration struct that a backend projects onto
a real platform widget — UIKit/AppKit on Apple, Android Views on Android — or
draws itself in the Hydrolysis and Dew renderers.

## Installation

Most applications get these controls through the umbrella crate, which re-exports
them from its prelude — every example below assumes `use waterui::prelude::*;`.

```toml
[dependencies]
waterui = "0.2"
```

Depend on the crate directly only when building a component library that must not
pull in the full framework:

```toml
[dependencies]
waterui-controls = "0.2"
```

There are no Cargo features; the whole control set is always available.

## Quick Start

```rust,no_run
use waterui::prelude::*;

fn settings_form() -> impl View {
    let name = Binding::container(Str::from(""));
    let quantity = Binding::i32(1);
    let notify = Binding::bool(true);
    let volume = Binding::f64(0.5);
    let saved = Binding::bool(false);

    vstack((
        field("Name", &name),
        stepper("Quantity", &quantity).range(1..=10),
        toggle("Notifications", &notify),
        slider("Volume", &volume).range(0.0..=1.0),
        button("Save")
            .bordered_prominent()
            .action(|State(saved): State<Binding<bool>>| saved.set(true))
            .state(&saved),
    ))
    .spacing(12.0)
}
```

## Core Concepts

### Labels are mandatory, visibility is optional

Every constructor takes a label. `IntoLabel` accepts a string literal, a `Text`,
a `StyledStr`, a `Binding`, a `Computed`, or a fully built `Label` — literals go
through WaterUI's i18n-aware text pipeline, so a label is a translation key as
much as it is a string.

When the surrounding layout already explains the control, hide the label's chrome
rather than omitting it:

```rust,no_run
use waterui::prelude::*;

fn brightness_row(brightness: &Binding<f64>) -> impl View {
    hstack((
        text("*"),
        // Drawn as a bare track; still announced as "Brightness".
        slider("Brightness", brightness).hide_label(),
    ))
}
```

`.label_style(...)` takes the full `LabelDisplayMode` range — `TitleAndIcon`,
`TitleOnly`, `IconOnly`, `Hidden` — and `Automatic` defers to whatever the
surrounding scope installed. Installing a mode on a subtree adapts a whole strip
of chrome at once:

```rust,no_run
use waterui::icon::system_icon;
use waterui::prelude::*;

fn toolbar() -> impl View {
    hstack((
        button(label("Search").system_icon(system_icon::search())).action(|| {}),
        button(label("Settings").system_icon(system_icon::settings())).action(|| {}),
    ))
    // Both buttons render icon-only; both still announce their titles.
    .install(LabelDisplayMode::IconOnly)
}
```

`system_icon` renders SF Symbols on Apple platforms and is deliberately
unsupported elsewhere. For portable icons pass any view to `Label::icon` — the
`waterui-icons-lucide`, `waterui-icons-material-icon`, and
`waterui-icons-fontawesome7` packs are built for this.

When the visible label is an arbitrary composition rather than text, `Label::new`
keeps the spoken text separate from what is drawn:

```rust,no_run
use waterui::prelude::*;

fn verified_button() -> impl View {
    button(Label::new("Verified account", || {
        hstack((text("Account"), text("[v]")))
    }))
    .action(|| {})
}
```

### Two constructors per control

`Type::new(...)` is the general constructor and takes the most general input the
control can render — `Slider::new` and `Stepper::new` take a built `Label`,
`Toggle::new` and `TextField::new` take only the binding and let you attach the
label afterwards. The free functions `button`, `toggle`, `slider`, `stepper`,
`field`, and `label` are the ergonomic entry points: they accept `impl IntoLabel`
so a string literal is enough.

```rust,no_run
use waterui::prelude::*;

fn two_ways(wifi: &Binding<bool>, sync: &Binding<bool>) -> impl View {
    vstack((
        Toggle::new(wifi).label("Wi-Fi").switch(),
        toggle("Sync", sync).checkbox(),
    ))
}
```

### Actions carry state through the environment

`Button::action` takes a handler whose parameters are extractors, resolved from
the environment at click time. `.state(&value)` injects a cloneable value for a
`State<T>` parameter to pick up; repeated calls bind in argument order. Nothing
needs to be cloned into the closure.

```rust,no_run
use waterui::prelude::*;

fn counter(count: &Binding<i32>) -> impl View {
    hstack((
        button("Decrement")
            .bordered()
            .action(|State(count): State<Binding<i32>>| *count.get_mut() -= 1)
            .state(count),
        text!("{count}"),
        button("Increment")
            .bordered_prominent()
            .action(|State(count): State<Binding<i32>>| *count.get_mut() += 1)
            .state(count),
    ))
    .spacing(8.0)
}
```

`action_async` takes the same handler shape and returns a future, spawned on the
local executor:

```rust,no_run
use waterui::prelude::*;

fn refresh(busy: &Binding<bool>) -> impl View {
    button("Refresh")
        .action_async(|State(busy): State<Binding<bool>>| async move {
            busy.set(true);
            // ... await the work ...
            busy.set(false);
        })
        .state(busy)
        .disabled(busy.clone())
}
```

### Disabling is a scope, not a field

No control has a `disabled` field. `.disabled(signal)` installs a scoped
environment attribute, and every control inside reads the state in force at its
own position — so one call disables an entire form, reactively.

```rust,no_run
use waterui::prelude::*;

fn locked_form(locked: &Binding<bool>, notify: &Binding<bool>) -> impl View {
    vstack((toggle("Notifications", notify),)).disabled(locked.clone())
}
```

### Layout behavior

`Toggle`, `Slider`, `Stepper`, and `TextField` stretch horizontally to fill the
width they are offered, at a fixed intrinsic height; `Toggle` and `Stepper` put
the label at the leading edge and the control at the trailing edge. `Button` and
`Menu` are content-sized and never stretch.

## Examples

### Slider with end-of-track labels

```rust,no_run
use waterui::prelude::*;

fn brightness(value: &Binding<f64>) -> impl View {
    slider("Brightness", value)
        .range(0.0..=100.0)
        .min_value_label("Dark")
        .max_value_label("Bright")
}
```

The default range is the normalized `0.0..=1.0`.

### Stepper with a formatted value

```rust,no_run
use waterui::prelude::*;

fn quantity(value: &Binding<i32>) -> impl View {
    stepper("Quantity", value)
        .range(0..=99)
        .step(5)
        .value_formatter(|n| format!("{n} items"))
}
```

`range` accepts any `RangeBounds<i32>`, and `step` accepts any
`impl IntoComputed<i32>`, so the step size can itself follow app state. The
formatter styles the inline value only — the semantic label is untouched.

### Text fields

```rust,no_run
use core::num::NonZeroUsize;
use text_field::KeyboardType;
use waterui::prelude::*;

fn email(value: &Binding<Str>) -> impl View {
    TextField::new(value)
        .label("Email")
        .prompt("Enter your email")
        .keyboard(KeyboardType::Email)
}

fn notes(value: &Binding<Str>) -> impl View {
    // Fields are single-line by default; the limit is a NonZeroUsize, so
    // "zero lines" cannot be expressed. `disable_line_limit()` removes it.
    field("Notes", value).line_limit(NonZeroUsize::new(4).expect("four lines"))
}

fn body(value: &Binding<styled::StyledStr>) -> impl View {
    TextField::styled(value).label("Body").disable_line_limit()
}
```

`TextField::new` takes a plain `Binding<Str>` and rejects styled write-back; use
`TextField::styled` for rich text. `keyboard(...)` is a hint that platforms
without a software keyboard ignore.

### Menus and commands

Menu content is anything implementing `MenuView`: a `Command`, a nested `Menu`, a
`Divider`, an ordinary `Button`, or tuples, arrays, `Vec`s, and `Option`s of
those. `CommandExt` lets any label-like value start a command directly.

```rust,no_run
use waterui::prelude::*;

fn actions(log: &Binding<Str>) -> impl View {
    Menu::new(
        label("Actions").system_icon(waterui::icon::system_icon::plus()),
        (
            "Refresh"
                .action(|State(log): State<Binding<Str>>| log.set(Str::from("refreshed")))
                .state(log),
            Divider,
            Menu::new("Advanced", (button("Archive").action(|| {}),)),
        ),
    )
}
```

Commands carry the metadata system menus need — a keyboard shortcut, a reactive
disabled state, a reactive checked state. Pass the resulting menus to
`App::menu_bar` to populate a platform menu bar.

```rust,no_run
use waterui::prelude::*;

fn file_menu(dirty: &Binding<bool>) -> Menu {
    Menu::new(
        "File",
        (
            Command::builder("Save")
                .action(|| {})
                .shortcut(Shortcut::new("s").command())
                .disabled(dirty.clone().not()),
            Divider,
            Command::builder("Close")
                .action(|| {})
                .shortcut(Shortcut::new("w").command().shift()),
        ),
    )
}
```

The same vocabulary customizes a text field's selection menu:

```rust,no_run
use waterui::prelude::*;

fn note(value: &Binding<Str>) -> impl View {
    TextField::new(value)
        .label("Note")
        .selection_menu(("Translate".action(|| {}), "Define".action(|| {})))
}
```

## API Overview

### Controls

| Type | Constructors | Notes |
| --- | --- | --- |
| `Button<Action>` | `Button::new(Label)`, `button(impl IntoLabel)` | `action` / `action_async`, `style`, `size`; content-sized |
| `Toggle` | `Toggle::new(&Binding<bool>)`, `toggle(label, &binding)` | `switch()` / `checkbox()` presentation |
| `Slider` | `Slider::new(Label, &Binding<f64>)`, `slider(label, &binding)` | `range`, `min_value_label`, `max_value_label` |
| `Stepper` | `Stepper::new(Label, &Binding<i32>)`, `stepper(label, &binding)` | `range`, `step`, `value_formatter` |
| `TextField` | `TextField::new(&Binding<Str>)`, `TextField::styled(&Binding<StyledStr>)`, `field(label, &binding)` | `prompt`, `keyboard`, `line_limit`, `selection_menu` |
| `Menu` | `Menu::new(label, impl MenuView)` | Renders as a popup trigger, or nests inside another menu |

### Labels

- `Label` — semantic label carrying text, an optional icon, a display mode, and
  an optional accessibility override.
- `label(impl IntoText)` — ergonomic constructor for a text label.
- `Label::new(semantic_text, content_builder)` — arbitrary visual content with
  separate spoken text.
- `IntoLabel` — implemented for `&'static str`, `String`, `Str`, `Text`,
  `StyledStr`, `Binding<T>`, `Computed<T>`, and `Label`.
- `LabelDisplayMode`, `IconPosition` — presentation attributes; `LabelDisplayMode`
  is also a `Plugin`, installable on a subtree.

### Menus

- `Command`, `CommandBuilder`, `CommandExt` — a reusable action with a label, a
  shortcut, and reactive `disabled` / `selected` state.
- `MenuItem` — a command, a divider, or a nested menu.
- `Shortcut`, `ShortcutModifiers` — key equivalent plus command / shift / option /
  control.
- `MenuView`, `MenuBarView` — conversions from menu content and from top-level
  menu-bar content.

### Style attributes

- `ButtonStyle` — `Automatic`, `Plain`, `Link`, `Borderless`, `Bordered`,
  `BorderedProminent`. Also a `Plugin`: install it on a subtree to set the default
  for buttons that did not pick one.
- `button::ButtonSize` — `ExtraSmall`, `Small`, `Medium`, `Large`, `ExtraLarge`;
  scales height, padding, icon size, and corner shape together.
- `ToggleStyle` — `Automatic`, `Switch`, `Checkbox`.
- `text_field::KeyboardType` — `Text`, `Email`, `URL`, `Number`, `PhoneNumber`.

### Backend-facing configuration

`ButtonConfig`, `ToggleConfig`, `SliderConfig`, `StepperConfig`, and
`TextFieldConfig` are the payloads backends consume. Application code rarely
names them; theme hooks that restyle a control receive one and hand back a view.

## Related Crates

- [`waterui`](https://crates.io/crates/waterui) — the umbrella crate; its prelude
  re-exports everything documented here.
- [`waterui-core`](https://crates.io/crates/waterui-core) — the `View` trait,
  `Environment`, extractors, and the handler machinery behind `action`.
- [`waterui-text`](https://crates.io/crates/waterui-text) — `Text`, `StyledStr`,
  and the i18n-aware pipeline every label flows through.
- [`waterui-icon`](https://crates.io/crates/waterui-icon) — `SystemIcon` and the
  `system_icon` catalog used by labels.
- [`waterui-form`](https://crates.io/crates/waterui-form) — form building and
  validation layered on these controls, plus `Picker` and friends.
