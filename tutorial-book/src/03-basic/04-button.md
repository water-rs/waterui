# Button

The `button` is a fundamental interactive component that allows users to trigger actions. In WaterUI, buttons are versatile and can be composed with any view to create rich, interactive experiences.

## Basic Usage

To create a button, you use the `button` function, providing a label and an action to be performed on click.

```rust,ignore
use waterui::prelude::*;

fn my_button() -> impl View {
    button("Click Me", || {
        println!("Button was clicked!");
    })
}
```

In this example, we create a button with the label "Click Me". When the button is pressed, the closure `|| { println!("Button was clicked!"); }` is executed.

## State Management with Bindings

Buttons are most powerful when they interact with the application's state. You can use a `Binding` to modify your app's state from within a button's action.

Let's create a simple counter that increments each time a button is clicked.

```rust,ignore
use waterui::prelude::*;

fn counter_button() -> impl View {
    let count = binding(0);

    vstack((
        text(s!("Count: {count}")),
        button("Increment", move || {
            count.update(|c| *c += 1);
        }),
    ))
}
```

Here's what's happening:
1.  We create a mutable `Binding` called `count` initialized to `0`.
2.  The `text` component displays the current value of `count`. Because we use the `s!` macro, the text will automatically update whenever `count` changes.
3.  The `button`'s action closure captures the `count` binding.
4.  Inside the action, we call `count.update(|c| *c += 1)`. This is the idiomatic way to modify a value within a binding. It gives us a mutable reference to the value (`c`), we increment it, and `nami` ensures that any UI component subscribed to `count` is re-rendered.

## Custom Labels

A button's label doesn't have to be just text. You can pass any `View` to the `button` function.

```rust,ignore
use waterui::prelude::*;

fn custom_label_button() -> impl View {
    button(
        hstack((
            // In a real app, you might use an `icon` component here
            text("🚀"), 
            text("Launch").padding(5.0)
        )),
        || {
            println!("Launching rocket!");
        }
    )
}
```

This allows you to create buttons with icons, images, or complex layouts as their labels, giving you full design flexibility.

## Styling

WaterUI provides a flexible styling system to customize the appearance of your buttons. You can change colors, borders, padding, and more. Styling is covered in detail in a later chapter. (TODO: Link to styling chapter)
