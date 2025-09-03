# Component Composition

Component composition is fundamental to building maintainable UIs in WaterUI. This chapter teaches you how to build complex interfaces by combining simple, reusable components.

## Composition Principles

### Small, Focused Components

Build components that do one thing well:

```rust,ignore
use waterui::*;
use nami::s;

// Single responsibility: Display a metric
fn metric_card(label: &str, value: impl Signal<Output = String>) -> impl View {
    vstack((
        text(label)
            .size(12.0)
            .color(Color::secondary()),
        text(value)
            .size(24.0)
            .weight(.bold),
    ))
    .padding(15.0)
    .background(Color::white())
    .corner_radius(8.0)
    .shadow(1.0)
}

// Compose multiple metrics into a dashboard
fn dashboard() -> impl View {
    let users = binding(1234);
    let revenue = binding(98765.43);
    let growth = binding(12.5);
    
    hstack((
        metric_card("Users", s!(users.to_string())),
        metric_card("Revenue", s!(format!("${:.2}", revenue))),
        metric_card("Growth", s!(format!("{:.1}%", growth))),
    ))
    .spacing(20.0)
}
```

## Summary

Component composition enables building complex UIs from simple, reusable parts using functions and Environment for cross-cutting concerns.

Next: [Event Handling](07-events.md)
