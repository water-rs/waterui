# Text and Typography

Text is fundamental to any UI framework. WaterUI provides powerful text rendering capabilities with reactive updates, styling options, and typography controls.

## Basic Text

### Static Text

```rust,ignore
fn static_text_demo() -> impl View {
    vstack((
        text("Simple text"),
        text("Multi-line text with\nline breaks"),
        text("Very long text that might need to wrap to multiple lines in a container with limited width"),
    ))
    .spacing(10.0)
}
```

### Reactive Text with text! Macro

```rust,ignore
fn reactive_text_demo() -> impl View {
    let count = binding(0);
    let name = binding("Alice".to_string());
    let temperature = binding(22.5);
    
    vstack((
        text!("Count: {}", count),
        text!("Hello, {}!", name),
        text!("Temperature: {:.1}°C", temperature),
        text!("Status: {}", s!(if count > 5 { "High" } else { "Low" })),
        
        hstack((
            button("Increment")
                .action({
                    let count = count.clone();
                    move |_| count.update(|c| c + 1)
                }),
            button("Reset")
                .action({
                    let count = count.clone();
                    move |_| count.set(0)
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(15.0)
}
```

## Text Styling

### Font Properties

```rust,ignore
fn font_styling_demo() -> impl View {
    vstack((
        text("Default text"),
        
        text("Large text")
            .font_size(24.0),
            
        text("Small text")
            .font_size(12.0),
            
        text("Bold text")
            .font_weight(FontWeight::Bold),
            
        text("Light text")
            .font_weight(FontWeight::Light),
            
        text("Italic text")
            .font_style(FontStyle::Italic),
            
        text("Monospace text")
            .font_family("monospace"),
            
        text("Custom font")
            .font_family("Arial, sans-serif"),
    ))
    .spacing(8.0)
}
```

### Text Colors

```rust,ignore
fn text_colors_demo() -> impl View {
    vstack((
        text("Default color text"),
        
        text("Primary colored text")
            .color(Color::primary()),
            
        text("Secondary colored text")
            .color(Color::secondary()),
            
        text("Success text")
            .color(Color::green()),
            
        text("Warning text")
            .color(Color::orange()),
            
        text("Error text")
            .color(Color::red()),
            
        text("Custom color text")
            .color(Color::rgb(0.2, 0.6, 0.9)),
    ))
    .spacing(8.0)
}
```

### Text Decorations

```rust,ignore
fn text_decorations_demo() -> impl View {
    let completed = binding(true);
    
    vstack((
        text("Normal text"),
        
        text("Underlined text")
            .underline(true),
            
        text("Strikethrough text")
            .strikethrough(true),
            
        text!("Reactive strikethrough: {}", s!(if completed { "Completed task" } else { "Pending task" }))
            .strikethrough(completed.get()),
            
        toggle(completed.clone())
            .label("Mark as completed"),
    ))
    .spacing(10.0)
}
```

## Text Layout and Alignment

### Text Alignment

```rust,ignore
fn text_alignment_demo() -> impl View {
    vstack((
        text("Left aligned text")
            .alignment(TextAlignment::Leading)
            .width(300.0)
            .background(Color::light_gray()),
            
        text("Center aligned text")
            .alignment(TextAlignment::Center)
            .width(300.0)
            .background(Color::light_gray()),
            
        text("Right aligned text")
            .alignment(TextAlignment::Trailing)
            .width(300.0)
            .background(Color::light_gray()),
            
        text("Justified text with enough content to demonstrate how justification works across multiple lines")
            .alignment(TextAlignment::Justified)
            .width(300.0)
            .background(Color::light_gray()),
    ))
    .spacing(10.0)
}
```

### Line Height and Spacing

```rust,ignore
fn text_spacing_demo() -> impl View {
    vstack((
        text("Normal line height\nSecond line\nThird line"),
        
        text("Tight line height\nSecond line\nThird line")
            .line_height(1.0),
            
        text("Loose line height\nSecond line\nThird line")
            .line_height(2.0),
            
        text("Custom spacing")
            .letter_spacing(2.0),
    ))
    .spacing(20.0)
}
```

## Advanced Text Features

### Text Truncation and Wrapping

```rust,ignore
fn text_wrapping_demo() -> impl View {
    let long_text = "This is a very long piece of text that demonstrates various text wrapping and truncation behaviors in WaterUI applications.";
    
    vstack((
        text("Text Wrapping Examples").font_size(18.0),
        
        text(long_text)
            .width(200.0)
            .wrap(TextWrap::Word),
            
        text(long_text)
            .width(200.0)
            .wrap(TextWrap::Character),
            
        text(long_text)
            .width(200.0)
            .truncation(.tail)
            .max_lines(2),
            
        text(long_text)
            .width(200.0)
            .truncation(.middle)
            .max_lines(1),
    ))
    .spacing(15.0)
}
```

### Rich Text with Spans

```rust,ignore
fn rich_text_demo() -> impl View {
    vstack((
        rich_text([
            text_span("Regular text, ")
                .color(Color::primary()),
            text_span("bold text, ")
                .font_weight(FontWeight::Bold)
                .color(Color::blue()),
            text_span("italic text, ")
                .font_style(FontStyle::Italic)
                .color(Color::green()),
            text_span("and underlined text.")
                .underline(true)
                .color(Color::red()),
        )),
        
        rich_text([
            text_span("Temperature: "),
            text_span("25°C")
                .font_weight(FontWeight::Bold)
                .color(Color::orange()),
            text_span(" ("),
            text_span("Normal")
                .color(Color::green()),
            text_span(")"),
        )),
    ))
    .spacing(15.0)
}
```

### Interactive Text

```rust,ignore
fn interactive_text_demo() -> impl View {
    let hover_state = binding(false);
    let click_count = binding(0);
    
    vstack((
        text!("Hover over me! (Hovered: {})", hover_state)
            .color(s!(if hover_state { Color::blue() } else { Color::primary() }))
            .on_hover({
                let hover_state = hover_state.clone();
                move |is_hovering| hover_state.set(is_hovering)
            }),
            
        text!("Click me! (Clicked {} times)", click_count)
            .color(Color::blue())
            .underline(true)
            .cursor(CursorStyle::Pointer)
            .on_tap({
                let click_count = click_count.clone();
                move |_| click_count.update(|c| c + 1)
            }),
            
        text("Selectable text content that users can select and copy")
            .selectable(true)
            .background(Color::light_gray())
            .padding(10.0),
    ))
    .spacing(15.0)
}
```

## Text Input Fields

### Basic Text Fields

```rust,ignore
fn text_input_demo() -> impl View {
    let single_line = binding(String::new());
    let multiline = binding(String::new());
    let password = binding(String::new());
    
    vstack((
        text_field(single_line.clone())
            .placeholder("Enter single line text")
            .on_change({
                let single_line = single_line.clone();
                move |new_value| {
                    println!("Single line changed: {}", new_value);
                    single_line.set(new_value);
                }
            }),
            
        text_area(multiline.clone())
            .placeholder("Enter multiple lines of text here...")
            .min_height(100.0)
            .on_change({
                let multiline = multiline.clone();
                move |new_value| multiline.set(new_value)
            }),
            
        secure_field(password.clone())
            .placeholder("Enter password")
            .on_change({
                let password = password.clone();
                move |new_value| password.set(new_value)
            }),
            
        text!("Single line: '{}'", single_line),
        text!("Multiline length: {} characters", s!(multiline.len())),
        text!("Password length: {} characters", s!(password.len())),
    ))
    .spacing(10.0)
}
```

### Formatted Text Fields

```rust,ignore
fn formatted_input_demo() -> impl View {
    let number = binding(0.0);
    let currency = binding(0.0);
    let phone = binding(String::new());
    
    vstack((
        number_field(number.clone())
            .placeholder("Enter a number")
            .format(NumberFormat::Decimal(2))
            .on_change({
                let number = number.clone();
                move |new_value| number.set(new_value)
            }),
            
        currency_field(currency.clone())
            .placeholder("Enter amount")
            .currency_code("USD")
            .on_change({
                let currency = currency.clone();
                move |new_value| currency.set(new_value)
            }),
            
        text_field(phone.clone())
            .placeholder("(555) 123-4567")
            .input_formatter(PhoneFormatter::new())
            .on_change({
                let phone = phone.clone();
                move |new_value| phone.set(new_value)
            }),
            
        text!("Number: {:.2}", number),
        text!("Currency: ${:.2}", currency),
        text!("Phone: {}", phone),
    ))
    .spacing(10.0)
}
```

## Typography System

### Text Styles

```rust,ignore
fn typography_system_demo() -> impl View {
    vstack((
        text("Display Large")
            .style(TextStyle::DisplayLarge),
            
        text("Display Medium")
            .style(TextStyle::DisplayMedium),
            
        text("Display Small")
            .style(TextStyle::DisplaySmall),
            
        text("Headline Large")
            .style(TextStyle::HeadlineLarge),
            
        text("Headline Medium")
            .style(TextStyle::HeadlineMedium),
            
        text("Headline Small")
            .style(TextStyle::HeadlineSmall),
            
        text("Title Large")
            .style(TextStyle::TitleLarge),
            
        text("Title Medium")
            .style(TextStyle::TitleMedium),
            
        text("Title Small")
            .style(TextStyle::TitleSmall),
            
        text("Body Large - This is the primary text style for longer passages of text.")
            .style(TextStyle::BodyLarge),
            
        text("Body Medium - This is a versatile text style for most UI text content.")
            .style(TextStyle::BodyMedium),
            
        text("Body Small - This style is used for captions and supporting text.")
            .style(TextStyle::BodySmall),
            
        text("Label Large")
            .style(TextStyle::LabelLarge),
            
        text("Label Medium")
            .style(TextStyle::LabelMedium),
            
        text("Label Small")
            .style(TextStyle::LabelSmall),
    ))
    .spacing(5.0)
}
```

### Custom Typography Theme

```rust,ignore
fn custom_typography_demo() -> impl View {
    vstack((
        text("App Title")
            .font_size(32.0)
            .font_weight(FontWeight::Bold)
            .color(Color::primary()),
            
        text("Section Header")
            .font_size(24.0)
            .font_weight(FontWeight::SemiBold)
            .color(Color::primary())
            .margin_top(20.0),
            
        text("This is body text with custom styling applied. It uses a readable font size and appropriate line height for comfortable reading.")
            .font_size(16.0)
            .line_height(1.5)
            .color(Color::text()),
            
        text("Caption or metadata text")
            .font_size(12.0)
            .color(Color::secondary())
            .font_style(FontStyle::Italic),
    ))
    .spacing(8.0)
    .padding(20.0)
}
```

## Text Performance and Best Practices

### Efficient Text Updates

```rust,ignore
fn efficient_text_demo() -> impl View {
    let counter = binding(0);
    let expensive_computation = binding(false);
    
    // Good: Use s! macro for reactive computations
    let status = s!(match counter {
        0..=10 => "Low",
        11..=50 => "Medium", 
        _ => "High"
    });
    
    // Good: Memoize expensive computations
    let computed_value = s!({
        if expensive_computation {
            // Simulate expensive calculation
            (0..1000).sum::<i32>()
        } else {
            0
        }
    });
    
    vstack((
        text!("Counter: {} ({})", counter, status),
        text!("Computed: {}", computed_value),
        
        hstack((
            button("Increment")
                .action({
                    let counter = counter.clone();
                    move |_| counter.update(|c| c + 1)
                }),
            button("Toggle Computation")
                .action({
                    let expensive_computation = expensive_computation.clone();
                    move |_| expensive_computation.update(|c| !c)
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(10.0)
}
```

### Text Accessibility

```rust,ignore
fn accessible_text_demo() -> impl View {
    vstack((
        text("Accessible Text Example")
            .accessibility_label("Main heading for text accessibility demonstration")
            .accessibility_role(AccessibilityRole::Heading)
            .accessibility_level(1),
            
        text("This text provides important information to users")
            .accessibility_hint("Additional context about this information")
            .min_contrast_ratio(4.5), // WCAG AA compliance
            
        text("Error: Invalid input")
            .color(Color::red())
            .accessibility_role(AccessibilityRole::Alert)
            .accessibility_live_region(LiveRegion::Assertive),
            
        text("Optional field")
            .color(Color::secondary())
            .accessibility_label("Optional field description"),
    ))
    .spacing(10.0)
}
```

## Testing Text Components

```rust,ignore
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_reactive_text_updates() {
        let count = binding(0);
        let text_content = s!(format!("Count: {}", count));
        
        assert_eq!(text_content.get(), "Count: 0");
        
        count.set(5);
        assert_eq!(text_content.get(), "Count: 5");
        
        count.update(|c| c * 2);
        assert_eq!(text_content.get(), "Count: 10");
    }
    
    #[test]
    fn test_text_formatting() {
        let temperature = binding(23.456);
        let formatted = s!(format!("Temperature: {:.1}°C", temperature));
        
        assert_eq!(formatted.get(), "Temperature: 23.5°C");
        
        temperature.set(-5.0);
        assert_eq!(formatted.get(), "Temperature: -5.0°C");
    }
}
```

## Summary

WaterUI's text system provides:

- **Reactive Text**: Use `text!` macro for dynamic content
- **Rich Styling**: Comprehensive font, color, and decoration options
- **Typography System**: Predefined styles for consistent design
- **Text Input**: Various input field types with formatting
- **Performance**: Efficient reactive updates with `s!` macro
- **Accessibility**: Built-in support for screen readers and WCAG compliance

Key best practices:
- Use `text!` macro for reactive text content
- Leverage `s!` macro for computed text values
- Apply consistent typography styles
- Consider accessibility from the start
- Test text components thoroughly

Next: [Media Components](10-media.md)
