# Conditional Rendering

Conditional rendering is a fundamental technique for creating dynamic user interfaces that respond to changing application state. WaterUI provides powerful and ergonomic components for conditional rendering through the `when` function and related components.

## The `when` Function

The `when` function is the primary tool for conditional rendering in WaterUI. It takes a reactive boolean condition and a closure that returns a view to display when the condition is true.

```rust,ignore
use waterui::widget::condition::when;
use waterui_text::text;
use waterui::reactive::{binding, s};

let is_logged_in = binding(false);

when(&is_logged_in, || {
    "Welcome back!"
})
```

## Basic Conditional Rendering

Here's a simple example showing how to conditionally display content:

```rust,ignore
use waterui::widget::condition::when;
use waterui_text::text;
use waterui::component::{button::button, layout::stack::vstack};
use waterui::reactive::binding;

pub fn login_view() -> impl View {
	#[derive(Debug,Clone,Default)]
	pub struct Authenticated(Binding<Bool>)
    
    vstack((
        use_env(|Authenticated(auth):Authenticated|{
	        when(&auth, || {
		        text!("You are logged in!")
	        }),
        
	        when(&!is_authenticated, |Authenticated(auth):Authenticated| {
	            button("Login").action(move || auth.set(true)})
	        })
        })
    )).with(Authenticated::default())
}
```

## Reactive Negation with `!`

WaterUI's `Binding` type implements the `Not` trait, which means you can use `!` directly on bindings without wrapping them in `s!()`. This maintains full reactivity:

```rust,ignore
use waterui::{when, text};
use nami::binding;

let is_visible = binding(true);

when(!is_visible, || "Hidden");
```

## Complete Conditional Rendering with `or`

For situations where you need to display one of two views, use the `.or()` method:

```rust,ignore
use waterui::widget::condition::when;
use waterui::component::button::button;
use waterui::reactive::binding;

let has_data = binding(false);

when(&has_data, || {
    "Data loaded successfully!"
}).or(|| {
    "Loading..."
})
```
