# Layout Components

WaterUI provides powerful layout components for arranging UI elements. This chapter covers the essential layout tools.

## Stack Layouts

### VStack - Vertical Arrangement

```rust,ignore
fn vertical_layout() -> impl View {
    vstack((
        "First item",
        "Second item",
        "Third item",
    ))
    .spacing(10.0)
    .padding(20.0)
}
```

### HStack - Horizontal Arrangement

```rust,ignore
fn navigation_bar() -> impl View {
    hstack((
        button("← Back"),
        spacer(),  // Pushes items apart
        "Title",
        spacer(),
        button("Menu"),
    ))
    .padding(15.0)
}
```

### ZStack - Overlay Arrangement

```rust,ignore
fn card_with_badge() -> impl View {
    zstack((
        // Background card
        card_background(),
        
        // Main content
        card_content(),
        
        // Badge overlay (top-right)
        badge("New")
            .alignment(.top_trailing),
    ))
}
```

## Grid Layout

```rust,ignore
fn photo_grid() -> impl View {
    grid([
        [photo("1.jpg"), photo("2.jpg"), photo("3.jpg")],
        [photo("4.jpg"), photo("5.jpg"), photo("6.jpg")],
        [photo("7.jpg"), photo("8.jpg"), photo("9.jpg")],
    ))
    .spacing(5.0)
}
```

## Scrolling

```rust,ignore
fn long_list() -> impl View {
    scroll(
        vstack(
            (0..1000).map(|i| text(format!("Item {}", i)))
        )
    )
    .max_height(400.0)
}
```

## Sizing and Constraints

```rust,ignore
fn responsive_layout() -> impl View {
    hstack((
        sidebar()
            .width(250.0)
            .max_width(300.0),
            
        content()
            .flex(1)  // Takes remaining space
            .min_width(400.0),
    ))
}
```

Next: [Text and Typography](09-text.md)
