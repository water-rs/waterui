# Animation and Transitions

WaterUI provides a powerful animation system that allows you to create smooth, performant animations and transitions with minimal code. The animation system is built on top of the reactive state management system and integrates seamlessly with the view hierarchy.

## Understanding WaterUI Animations

WaterUI animations are declarative and state-driven. Instead of imperatively controlling animation timing, you define the desired end state and let WaterUI handle the interpolation.

### Basic Animation Concepts

```rust,ignore
use waterui::*;
use nami::{s, Binding};
use std::time::Duration;

// Basic animated property
fn animated_button() -> impl View {
    let is_pressed = s!(false);
    let scale = s!(1.0);
    
    button("Press Me")
        .scale(scale.get())
        .animation(Animation::spring())
        .on_press(move || {
            is_pressed.set(!is_pressed.get());
            scale.set(if is_pressed.get() { 0.95 } else { 1.0 });
        })
}
```

## Animation Types

WaterUI supports several types of animations, each optimized for different use cases.

### Linear Animations

Linear animations provide constant-speed transitions:

```rust,ignore
use std::time::Duration;

fn linear_fade() -> impl View {
    let opacity = s!(1.0);
    
    vstack((
        text("Fading Text")
            .opacity(opacity.get())
            .animation(Animation::linear(Duration::from_millis(500))),
        
        button("Toggle")
            .on_press(move || {
                opacity.set(if opacity.get() > 0.5 { 0.0 } else { 1.0 });
            }),
    ))
}
```

### Ease Animations

Ease animations provide natural acceleration and deceleration:

```rust,ignore
fn ease_animations() -> impl View {
    let position_x = s!(0.0);
    
    vstack((
        // Ease in - slow start, fast end
        circle()
            .size(50.0)
            .offset_x(position_x.get())
            .animation(Animation::ease_in(Duration::from_millis(800))),
        
        // Ease out - fast start, slow end  
        circle()
            .size(50.0)
            .offset_x(position_x.get())
            .animation(Animation::ease_out(Duration::from_millis(800))),
        
        // Ease in-out - slow start and end
        circle()
            .size(50.0)
            .offset_x(position_x.get())
            .animation(Animation::ease_in_out(Duration::from_millis(800))),
        
        button("Move")
            .on_press(move || {
                position_x.set(if position_x.get() > 100.0 { 0.0 } else { 200.0 });
            }),
    ))
    .spacing(20.0)
}
```

### Spring Animations

Spring animations provide bouncy, natural-feeling motion:

```rust,ignore
fn spring_example() -> impl View {
    let scale = s!(1.0);
    let rotation = s!(0.0);
    
    vstack((
        rectangle()
            .size(100.0)
            .scale(scale.get())
            .rotation(rotation.get())
            .animation(Animation::spring(SpringConfig {
                stiffness: 300.0,
                damping: 20.0,
                mass: 1.0,
                initial_velocity: 0.0,
            }))
            .fill(Color::blue()),
        
        hstack((
            button("Scale")
                .on_press({
                    let scale = scale.clone();
                    move || scale.set(if scale.get() > 1.0 { 1.0 } else { 1.5 })
                }),
            
            button("Rotate")
                .on_press(move || {
                    rotation.set(rotation.get() + 45.0);
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(30.0)
}
```

## Animatable Properties

WaterUI can animate most visual properties smoothly:

### Transform Properties

```rust,ignore
fn transform_animations() -> impl View {
    let offset = s!(Offset::zero());
    let scale = s!(1.0);
    let rotation = s!(0.0);
    
    let animated_view = rectangle()
        .size(100.0)
        .offset(offset.get())
        .scale(scale.get())
        .rotation(rotation.get())
        .animation(Animation::spring())
        .fill(Color::red());
    
    vstack((
        animated_view,
        
        hstack((
            button("Move")
                .on_press({
                    let offset = offset.clone();
                    move || offset.set(Offset::new(100.0, 50.0))
                }),
            
            button("Scale")
                .on_press({
                    let scale = scale.clone();
                    move || scale.set(2.0)
                }),
            
            button("Rotate")
                .on_press(move || rotation.set(180.0)),
        ))
        .spacing(10.0),
        
        button("Reset")
            .on_press({
                let offset = offset.clone();
                let scale = scale.clone();
                let rotation = rotation.clone();
                move || {
                    offset.set(Offset::zero());
                    scale.set(1.0);
                    rotation.set(0.0);
                }
            }),
    ))
    .spacing(20.0)
}
```

### Color Animations

```rust,ignore
fn color_animations() -> impl View {
    let background_color = s!(Color::blue());
    let text_color = s!(Color::white());
    
    vstack((
        text("Animated Colors")
            .color(text_color.get())
            .padding(20.0)
            .background(background_color.get())
            .corner_radius(10.0)
            .animation(Animation::ease_in_out(Duration::from_millis(600))),
        
        hstack((
            button("Theme 1")
                .on_press({
                    let bg = background_color.clone();
                    let text = text_color.clone();
                    move || {
                        bg.set(Color::blue());
                        text.set(Color::white());
                    }
                }),
            
            button("Theme 2")
                .on_press({
                    let bg = background_color.clone();
                    let text = text_color.clone();
                    move || {
                        bg.set(Color::red());
                        text.set(Color::yellow());
                    }
                }),
            
            button("Theme 3")
                .on_press(move || {
                    background_color.set(Color::green());
                    text_color.set(Color::black());
                }),
        ))
        .spacing(10.0),
    ))
    .spacing(20.0)
}
```

### Size and Layout Animations

```rust,ignore
fn size_animations() -> impl View {
    let width = s!(100.0);
    let height = s!(100.0);
    let expanded = s!(false);
    
    vstack((
        rectangle()
            .width(width.get())
            .height(height.get())
            .animation(Animation::spring())
            .fill(Color::purple()),
        
        button("Toggle Size")
            .on_press(move || {
                expanded.set(!expanded.get());
                if expanded.get() {
                    width.set(200.0);
                    height.set(150.0);
                } else {
                    width.set(100.0);
                    height.set(100.0);
                }
            }),
    ))
    .spacing(20.0)
}
```

## Complex Animation Sequences

For more complex animations, you can chain and combine multiple animations:

### Sequential Animations

```rust,ignore
use std::time::Duration;
use tokio::time::sleep;

fn sequential_animation() -> impl View {
    let position = s!(Offset::zero());
    let scale = s!(1.0);
    let opacity = s!(1.0);
    
    let animated_circle = circle()
        .size(50.0)
        .offset(position.get())
        .scale(scale.get())
        .opacity(opacity.get())
        .animation(Animation::ease_in_out(Duration::from_millis(500)))
        .fill(Color::orange());
    
    vstack((
        animated_circle,
        
        button("Animate Sequence")
            .on_press({
                let position = position.clone();
                let scale = scale.clone();
                let opacity = opacity.clone();
                move || {
                    let pos = position.clone();
                    let sc = scale.clone();
                    let op = opacity.clone();
                    
                    tokio::spawn(async move {
                        // Step 1: Move right
                        pos.set(Offset::new(100.0, 0.0));
                        sleep(Duration::from_millis(500)).await;
                        
                        // Step 2: Scale up
                        sc.set(1.5);
                        sleep(Duration::from_millis(500)).await;
                        
                        // Step 3: Fade out
                        op.set(0.3);
                        sleep(Duration::from_millis(500)).await;
                        
                        // Step 4: Return to original state
                        pos.set(Offset::zero());
                        sc.set(1.0);
                        op.set(1.0);
                    });
                }
            }),
    ))
    .spacing(30.0)
}
```

### Parallel Animations

```rust,ignore
fn parallel_animations() -> impl View {
    let transform = s!(Transform::identity());
    let color = s!(Color::blue());
    
    rectangle()
        .size(100.0)
        .transform(transform.get())
        .fill(color.get())
        .animation(Animation::spring())
        .on_tap(move || {
            // Both animations happen simultaneously
            transform.set(Transform::identity()
                .scaled(1.5)
                .rotated(45.0)
                .translated(50.0, 25.0));
            color.set(Color::red());
        })
}
```

## Gesture-Driven Animations

WaterUI animations work seamlessly with gesture recognizers:

### Drag Animations

```rust,ignore
fn draggable_animation() -> impl View {
    let position = s!(Offset::zero());
    let is_dragging = s!(false);
    
    circle()
        .size(80.0)
        .offset(position.get())
        .scale(if is_dragging.get() { 1.1 } else { 1.0 })
        .animation(Animation::spring())
        .fill(Color::green())
        .gesture(
            DragGesture::new()
                .on_started(move |_| is_dragging.set(true))
                .on_changed({
                    let position = position.clone();
                    move |delta| position.set(position.get() + delta.translation)
                })
                .on_ended(move |_| is_dragging.set(false))
        )
}
```

### Swipe Animations

```rust,ignore
fn swipe_cards() -> impl View {
    let cards = s!(vec!["Card 1", "Card 2", "Card 3", "Card 4"));
    let current_offset = s!(0.0);
    
    zstack(
        cards.get().iter().enumerate().map(|(index, card)| {
            let offset = current_offset.get() + (index as f32 * 300.0);
            
            rectangle()
                .width(250.0)
                .height(150.0)
                .offset_x(offset)
                .corner_radius(10.0)
                .fill(Color::blue())
                .animation(Animation::spring())
                .overlay(
                    text(card)
                        .color(Color::white())
                        .font_size(18.0)
                )
                .gesture(
                    DragGesture::new()
                        .on_ended({
                            let current_offset = current_offset.clone();
                            move |details| {
                                if details.velocity.x.abs() > 500.0 {
                                    // Snap to next card
                                    let new_offset = if details.velocity.x < 0.0 {
                                        current_offset.get() - 300.0
                                    } else {
                                        current_offset.get() + 300.0
                                    };
                                    current_offset.set(new_offset.max(-600.0).min(0.0));
                                }
                            }
                        })
                )
        }).collect::<Vec<_>>()
    )
}
```

## Performance Optimization

### Animation Performance Tips

1. **Use Transform Properties**: Transform properties (translate, scale, rotate) are GPU-accelerated:

```rust,ignore
// ✅ Efficient - uses transforms
fn efficient_animation() -> impl View {
    let scale = s!(1.0);
    
    circle()
        .size(100.0)
        .scale(scale.get())  // GPU-accelerated
        .animation(Animation::spring())
}

// ❌ Less efficient - changes layout
fn less_efficient_animation() -> impl View {
    let size = s!(100.0);
    
    circle()
        .size(size.get())  // May trigger layout recalculation
        .animation(Animation::spring())
}
```

2. **Batch Animation Updates**: Update multiple properties simultaneously when possible:

```rust,ignore
fn batched_updates() -> impl View {
    let transform = s!(Transform::identity());
    
    rectangle()
        .size(100.0)
        .transform(transform.get())
        .animation(Animation::spring())
        .on_tap(move || {
            // Single state update triggers one animation
            transform.set(Transform::identity()
                .scaled(1.2)
                .rotated(90.0)
                .translated(50.0, 0.0));
        })
}
```

3. **Use Appropriate Animation Curves**: Choose the right animation type for your use case:

```rust,ignore
fn optimized_animations() -> impl View {
    vstack((
        // For UI feedback - use spring for natural feel
        button("Spring Animation")
            .animation(Animation::spring()),
        
        // For loading indicators - use linear for consistency  
        progress_bar()
            .animation(Animation::linear(Duration::from_millis(1000))),
        
        // For page transitions - use ease curves
        page_transition()
            .animation(Animation::ease_in_out(Duration::from_millis(300))),
    ))
}
```

## Custom Animation Curves

You can create custom animation curves for unique effects:

```rust,ignore
fn custom_curve_animation() -> impl View {
    let progress = s!(0.0);
    
    // Custom bounce curve
    let bounce_curve = |t: f32| -> f32 {
        if t < 1.0 / 2.75 {
            7.5625 * t * t
        } else if t < 2.0 / 2.75 {
            let t = t - 1.5 / 2.75;
            7.5625 * t * t + 0.75
        } else if t < 2.5 / 2.75 {
            let t = t - 2.25 / 2.75;
            7.5625 * t * t + 0.9375
        } else {
            let t = t - 2.625 / 2.75;
            7.5625 * t * t + 0.984375
        }
    };
    
    circle()
        .size(60.0)
        .offset_y(progress.get() * 200.0)
        .animation(Animation::custom(
            Duration::from_millis(1000),
            bounce_curve
        ))
        .fill(Color::red())
        .on_tap(move || {
            progress.set(if progress.get() > 0.5 { 0.0 } else { 1.0 });
        })
}
```

## Animation State Management

### Animation Controllers

For complex animation sequences, use animation controllers:

```rust,ignore
struct AnimationController {
    is_playing: Binding<bool>,
    progress: Binding<f32>,
    direction: Binding<i32>, // 1 for forward, -1 for reverse
}

impl AnimationController {
    fn new() -> Self {
        Self {
            is_playing: s!(false),
            progress: s!(0.0),
            direction: s!(1),
        }
    }
    
    fn play(&self) {
        self.is_playing.set(true);
        self.animate_to_end();
    }
    
    fn reverse(&self) {
        self.direction.set(-1);
        self.is_playing.set(true);
        self.animate_to_start();
    }
    
    fn animate_to_end(&self) {
        let progress = self.progress.clone();
        let is_playing = self.is_playing.clone();
        
        tokio::spawn(async move {
            while progress.get() < 1.0 {
                progress.set((progress.get() + 0.02).min(1.0));
                sleep(Duration::from_millis(16)).await; // ~60 FPS
            }
            is_playing.set(false);
        });
    }
    
    fn animate_to_start(&self) {
        let progress = self.progress.clone();
        let is_playing = self.is_playing.clone();
        
        tokio::spawn(async move {
            while progress.get() > 0.0 {
                progress.set((progress.get() - 0.02).max(0.0));
                sleep(Duration::from_millis(16)).await;
            }
            is_playing.set(false);
        });
    }
}

fn controlled_animation() -> impl View {
    let controller = AnimationController::new();
    let scale = controller.progress.map(|p| 1.0 + p * 0.5);
    let rotation = controller.progress.map(|p| p * 360.0);
    
    vstack((
        rectangle()
            .size(100.0)
            .scale(scale.get())
            .rotation(rotation.get())
            .fill(Color::blue()),
        
        hstack((
            button("Play")
                .on_press({
                    let controller = controller.clone();
                    move || controller.play()
                }),
            
            button("Reverse")
                .on_press(move || controller.reverse()),
        ))
        .spacing(10.0),
    ))
    .spacing(20.0)
}
```

## Real-World Animation Examples

### Loading Animations

```rust,ignore
fn loading_animations() -> impl View {
    let rotation = s!(0.0);
    let pulse_scale = s!(1.0);
    
    // Start continuous animations
    let rotation_clone = rotation.clone();
    tokio::spawn(async move {
        loop {
            rotation_clone.set(rotation_clone.get() + 2.0);
            sleep(Duration::from_millis(16)).await;
        }
    });
    
    let pulse_clone = pulse_scale.clone();
    tokio::spawn(async move {
        let mut growing = true;
        loop {
            let current = pulse_clone.get();
            if growing {
                pulse_clone.set(current + 0.01);
                if current >= 1.2 { growing = false; }
            } else {
                pulse_clone.set(current - 0.01);
                if current <= 0.8 { growing = true; }
            }
            sleep(Duration::from_millis(16)).await;
        }
    });
    
    vstack((
        // Spinning loader
        circle()
            .size(40.0)
            .stroke(Color::blue(), 4.0)
            .stroke_dash([10.0, 5.0))
            .rotation(rotation.get())
            .animation(Animation::linear(Duration::from_millis(16))),
        
        // Pulsing dot
        circle()
            .size(20.0)
            .scale(pulse_scale.get())
            .animation(Animation::ease_in_out(Duration::from_millis(16)))
            .fill(Color::green()),
        
        text("Loading...")
            .color(Color::gray()),
    ))
    .spacing(20.0)
}
```

### Page Transitions

```rust,ignore
fn page_transition_example() -> impl View {
    let current_page = s!(0);
    let transition_offset = s!(0.0);
    
    let pages = vec!["Home", "Profile", "Settings"];
    
    vstack((
        // Page content with slide transition
        zstack(
            pages.iter().enumerate().map(|(index, page)| {
                let offset = (index as f32 - current_page.get() as f32) * 300.0 + transition_offset.get();
                
                rectangle()
                    .width(300.0)
                    .height(200.0)
                    .offset_x(offset)
                    .fill(Color::white())
                    .border(Color::gray(), 1.0)
                    .animation(Animation::ease_out(Duration::from_millis(300)))
                    .overlay(
                        text(page)
                            .font_size(24.0)
                            .color(Color::black())
                    )
            }).collect::<Vec<_>>()
        ),
        
        // Navigation
        hstack(
            pages.iter().enumerate().map(|(index, page)| {
                button(page)
                    .on_press({
                        let current_page = current_page.clone();
                        move || current_page.set(index)
                    })
                    .background(if current_page.get() == index { 
                        Color::blue() 
                    } else { 
                        Color::gray() 
                    })
            }).collect::<Vec<_>>()
        )
        .spacing(10.0),
    ))
    .spacing(20.0)
}
```

## Animation Testing and Debugging

### Animation Inspector

```rust,ignore
fn animation_debug_view(animated_view: impl View) -> impl View {
    let show_debug = s!(false);
    
    vstack((
        animated_view,
        
        if show_debug.get() {
            vstack((
                text("Animation Debug Info"),
                text("Frame Rate: 60 FPS"),
                text("GPU Accelerated: Yes"),
                text("Active Animations: 2"),
            ))
            .background(Color::black().opacity(0.8))
            .color(Color::white())
            .padding(10.0)
        } else {
            empty()
        },
        
        button("Toggle Debug")
            .on_press(move || show_debug.set(!show_debug.get())),
    ))
    .spacing(10.0)
}
```

## Best Practices

1. **Keep Animations Purposeful**: Every animation should serve a purpose - providing feedback, guiding attention, or enhancing understanding.

2. **Follow Platform Conventions**: Respect platform-specific animation durations and curves.

3. **Test Performance**: Profile your animations on target devices to ensure smooth performance.

4. **Provide Accessibility Options**: Allow users to disable animations if needed for accessibility.

5. **Use Appropriate Durations**: 
   - Micro-interactions: 100-200ms
   - Page transitions: 300-500ms
   - Loading animations: Continuous
   - Attention-seeking: 500-800ms

6. **Optimize for Battery Life**: Avoid unnecessary continuous animations on mobile devices.

By following these patterns and best practices, you can create smooth, performant animations that enhance your WaterUI applications' user experience while maintaining good performance across all target platforms.