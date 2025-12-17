//! Animation Example - Demonstrates WaterUI's animation system
//!
//! This example showcases visual animations:
//! - Transform animations (scale, rotation, translation)
//! - Progress bar animations with smooth value transitions
//! - Text transitions with cross-fade effects
//! - Toggle state animations
//! - Different animation curves (linear, ease-in, ease-out, spring)
//!
//! Animations in WaterUI are reactive - they automatically apply
//! when reactive values change, using the `.animated()` or
//! `.with(Animation::...)` modifiers.

use core::time::Duration;
use waterui::animation::Animation;
use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::Binding;
use waterui::style::Transform;

/// Demo: Scale animation - visual transform on colored boxes
fn scale_animation_section(scale: Binding<f32>) -> impl View {
    let animated_scale = scale
        .clone()
        .with(Animation::spring(300.0, 15.0));

    vstack((
        text("Scale Animation").size(20.0),
        "Click buttons to scale the box with spring physics",
        zstack((
            Color::srgb_hex("#3B82F6")
                .size(80.0, 80.0)
                .transform(Transform::scale(animated_scale)),
        ))
        .min_height(120.0),
        hstack((
            {
                let s = scale.clone();
                button("0.5x").action(move || s.set(0.5))
            },
            {
                let s = scale.clone();
                button("1x").action(move || s.set(1.0))
            },
            {
                let s = scale.clone();
                button("1.5x").action(move || s.set(1.5))
            },
            {
                let s = scale.clone();
                button("2x").action(move || s.set(2.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Rotation animation - spinning box
fn rotation_animation_section(rotation: Binding<f32>) -> impl View {
    let animated_rotation = rotation
        .clone()
        .with(Animation::ease_in_out(Duration::from_millis(500)));

    vstack((
        text("Rotation Animation").size(20.0),
        "Rotate the box smoothly",
        zstack((
            Color::srgb_hex("#22C55E")
                .size(60.0, 60.0)
                .transform(Transform::rotation(animated_rotation)),
        ))
        .min_height(100.0),
        hstack((
            {
                let r = rotation.clone();
                button("-90°").action(move || r.set(r.get() - 90.0))
            },
            {
                let r = rotation.clone();
                button("-45°").action(move || r.set(r.get() - 45.0))
            },
            {
                let r = rotation.clone();
                button("Reset").action(move || r.set(0.0))
            },
            {
                let r = rotation.clone();
                button("+45°").action(move || r.set(r.get() + 45.0))
            },
            {
                let r = rotation.clone();
                button("+90°").action(move || r.set(r.get() + 90.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Translation animation - moving box
fn translation_animation_section(offset_x: Binding<f32>, offset_y: Binding<f32>) -> impl View {
    let animated_x = offset_x
        .clone()
        .with(Animation::spring(200.0, 20.0));
    let animated_y = offset_y
        .clone()
        .with(Animation::spring(200.0, 20.0));

    vstack((
        text("Translation Animation").size(20.0),
        "Move the box with spring physics",
        zstack((
            Color::srgb_hex("#A855F7")
                .size(50.0, 50.0)
                .transform(Transform::translation(animated_x, animated_y)),
        ))
        .min_height(150.0),
        hstack((
            {
                let x = offset_x.clone();
                let y = offset_y.clone();
                button("Center").action(move || {
                    x.set(0.0);
                    y.set(0.0);
                })
            },
            {
                let x = offset_x.clone();
                button("Left").action(move || x.set(-50.0))
            },
            {
                let x = offset_x.clone();
                button("Right").action(move || x.set(50.0))
            },
            {
                let y = offset_y.clone();
                button("Up").action(move || y.set(-30.0))
            },
            {
                let y = offset_y.clone();
                button("Down").action(move || y.set(30.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Combined transform animation
fn combined_transform_section(
    combined_scale: Binding<f32>,
    combined_rotation: Binding<f32>,
) -> impl View {
    let animated_scale = combined_scale
        .clone()
        .with(Animation::spring(250.0, 18.0));
    let animated_rotation = combined_rotation
        .clone()
        .with(Animation::ease_in_out(Duration::from_millis(400)));

    vstack((
        text("Combined Transforms").size(20.0),
        "Scale and rotation together",
        zstack((
            Color::srgb_hex("#F59E0B")
                .size(60.0, 60.0)
                .transform(
                    Transform::scale(animated_scale)
                        .with_rotation(animated_rotation),
                ),
        ))
        .min_height(150.0),
        hstack((
            {
                let s = combined_scale.clone();
                let r = combined_rotation.clone();
                button("Reset").action(move || {
                    s.set(1.0);
                    r.set(0.0);
                })
            },
            {
                let s = combined_scale.clone();
                let r = combined_rotation.clone();
                button("Grow + Spin").action(move || {
                    s.set(1.8);
                    r.set(r.get() + 180.0);
                })
            },
            {
                let s = combined_scale.clone();
                button("Pulse").action(move || {
                    if s.get() > 1.2 {
                        s.set(0.8)
                    } else {
                        s.set(1.5)
                    }
                })
            },
        )),
    ))
    .padding()
}

/// Demo: Animated progress bar - the most visually impressive animation
fn progress_animation_section(progress_value: Binding<f64>) -> impl View {
    let animated_progress = progress_value
        .clone()
        .with(Animation::ease_in_out(Duration::from_millis(800)));

    vstack((
        text("Progress Bar Animation").size(20.0),
        "Watch the bar smoothly transition between values",
        progress(animated_progress),
        hstack((
            {
                let pv = progress_value.clone();
                button("0%").action(move || pv.set(0.0))
            },
            {
                let pv = progress_value.clone();
                button("25%").action(move || pv.set(0.25))
            },
            {
                let pv = progress_value.clone();
                button("50%").action(move || pv.set(0.5))
            },
            {
                let pv = progress_value.clone();
                button("75%").action(move || pv.set(0.75))
            },
            {
                let pv = progress_value.clone();
                button("100%").action(move || pv.set(1.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Progress with spring physics - bouncy feel
fn spring_progress_section(spring_value: Binding<f64>) -> impl View {
    let animated_progress = spring_value
        .clone()
        .with(Animation::spring(200.0, 12.0));

    let status = spring_value
        .clone()
        .map(|v| if v > 0.5 { "High" } else { "Low" })
        .animated();

    vstack((
        text("Spring Physics Animation").size(20.0),
        "Notice the bouncy overshoot effect",
        progress(animated_progress),
        hstack((
            text(status)
                .padding()
                .background(Color::srgb_hex("#8B5CF6").with_opacity(0.3)),
            spacer(),
            {
                let sv = spring_value.clone();
                button("Toggle").action(move || {
                    if sv.get() > 0.5 {
                        sv.set(0.1)
                    } else {
                        sv.set(0.9)
                    }
                })
            },
        )),
    ))
    .padding()
}

/// Demo: Animation curves comparison using visual bars
fn animation_curves_section(bar_scale: Binding<f32>) -> impl View {
    // Same value animated with different curves - visually compare timing
    let linear_scale = bar_scale
        .clone()
        .with(Animation::linear(Duration::from_millis(1000)));
    let ease_in_scale = bar_scale
        .clone()
        .with(Animation::ease_in(Duration::from_millis(1000)));
    let ease_out_scale = bar_scale
        .clone()
        .with(Animation::ease_out(Duration::from_millis(1000)));
    let spring_scale = bar_scale
        .clone()
        .with(Animation::spring(150.0, 12.0));

    vstack((
        text("Animation Curves Comparison").size(20.0),
        "Watch how different curves animate at different speeds",
        vstack((
            // Linear - constant speed
            hstack((
                text("Linear").min_width(80.0),
                zstack((
                    Color::srgb_hex("#06B6D4")
                        .size(200.0, 24.0)
                        .transform(Transform::scale_xy(linear_scale.clone(), 1.0_f32)),
                ))
                .min_width(220.0),
            )),
            // Ease-in - starts slow, ends fast
            hstack((
                text("Ease-In").min_width(80.0),
                zstack((
                    Color::srgb_hex("#22C55E")
                        .size(200.0, 24.0)
                        .transform(Transform::scale_xy(ease_in_scale.clone(), 1.0_f32)),
                ))
                .min_width(220.0),
            )),
            // Ease-out - starts fast, ends slow
            hstack((
                text("Ease-Out").min_width(80.0),
                zstack((
                    Color::srgb_hex("#F59E0B")
                        .size(200.0, 24.0)
                        .transform(Transform::scale_xy(ease_out_scale.clone(), 1.0_f32)),
                ))
                .min_width(220.0),
            )),
            // Spring - bouncy overshoot
            hstack((
                text("Spring").min_width(80.0),
                zstack((
                    Color::srgb_hex("#EF4444")
                        .size(200.0, 24.0)
                        .transform(Transform::scale_xy(spring_scale.clone(), 1.0_f32)),
                ))
                .min_width(220.0),
            )),
        )),
        hstack((
            {
                let w = bar_scale.clone();
                button("Small (0.3)").action(move || w.set(0.3))
            },
            {
                let w = bar_scale.clone();
                button("Medium (0.7)").action(move || w.set(0.7))
            },
            {
                let w = bar_scale.clone();
                button("Large (1.0)").action(move || w.set(1.0))
            },
        )),
    ))
    .padding()
}

/// Demo: Toggle with animated indicator
fn toggle_animation_section(toggle_state: Binding<bool>) -> impl View {
    // Animated scale for the indicator
    let indicator_scale = toggle_state
        .clone()
        .map(|on| if on { 1.0_f32 } else { 0.3 })
        .with(Animation::spring(300.0, 15.0));

    // Animated rotation
    let indicator_rotation = toggle_state
        .clone()
        .map(|on| if on { 0.0_f32 } else { 180.0 })
        .with(Animation::ease_in_out(Duration::from_millis(400)));

    vstack((
        text("Toggle Animation").size(20.0),
        "Watch the indicator scale and rotate with toggle",
        hstack((
            Toggle::new(&toggle_state).label(text("Power")),
            spacer(),
            // Visual indicator that animates
            zstack((
                Color::srgb_hex("#22C55E")
                    .size(60.0, 60.0)
                    .transform(
                        Transform::scale(indicator_scale).with_rotation(indicator_rotation),
                    ),
            ))
            .min_width(80.0)
            .min_height(80.0),
        )),
    ))
    .padding()
}

/// Demo: Staggered animations with bars
fn staggered_section(expanded: Binding<bool>) -> impl View {
    // Each bar has a different animation duration, creating a cascading effect
    let bar1_scale = expanded
        .clone()
        .map(|e| if e { 1.0_f32 } else { 0.2 })
        .with(Animation::spring(200.0, 15.0)); // Fastest

    let bar2_scale = expanded
        .clone()
        .map(|e| if e { 1.0_f32 } else { 0.2 })
        .with(Animation::spring(150.0, 15.0)); // Medium

    let bar3_scale = expanded
        .clone()
        .map(|e| if e { 1.0_f32 } else { 0.2 })
        .with(Animation::spring(100.0, 15.0)); // Slowest

    let bar4_scale = expanded
        .clone()
        .map(|e| if e { 1.0_f32 } else { 0.2 })
        .with(Animation::spring(80.0, 12.0)); // Even slower

    vstack((
        text("Staggered Animations").size(20.0),
        "Different spring stiffness creates cascading effect",
        hstack((
            zstack((
                Color::srgb_hex("#22C55E")
                    .size(50.0, 80.0)
                    .transform(Transform::scale_xy(1.0_f32, bar1_scale)),
            ))
            .min_height(100.0)
            .min_width(60.0),
            zstack((
                Color::srgb_hex("#3B82F6")
                    .size(50.0, 80.0)
                    .transform(Transform::scale_xy(1.0_f32, bar2_scale)),
            ))
            .min_height(100.0)
            .min_width(60.0),
            zstack((
                Color::srgb_hex("#A855F7")
                    .size(50.0, 80.0)
                    .transform(Transform::scale_xy(1.0_f32, bar3_scale)),
            ))
            .min_height(100.0)
            .min_width(60.0),
            zstack((
                Color::srgb_hex("#F59E0B")
                    .size(50.0, 80.0)
                    .transform(Transform::scale_xy(1.0_f32, bar4_scale)),
            ))
            .min_height(100.0)
            .min_width(60.0),
        )),
        {
            let e = expanded.clone();
            button("Toggle Bars").action(move || e.set(!e.get()))
        },
    ))
    .padding()
}

/// Demo: Animated size indicator - visual elements respond to value
fn size_indicator_section(size_value: Binding<f64>) -> impl View {
    // Animated scale based on value (0-100 maps to 0.1-1.0 scale)
    let animated_scale = size_value
        .clone()
        .map(|s| (s / 100.0 + 0.1) as f32)
        .with(Animation::spring(200.0, 15.0));

    let animated_rotation = size_value
        .clone()
        .map(|s| (s * 3.6) as f32) // 0-100 maps to 0-360 degrees
        .with(Animation::ease_in_out(Duration::from_millis(600)));

    let animated_y_scale = size_value
        .clone()
        .map(|s| (s / 100.0) as f32)
        .with(Animation::spring(180.0, 14.0));

    vstack((
        text("Size Indicator").size(20.0),
        "All elements respond to slider with different animations",
        hstack((
            // Vertical bar that scales vertically
            zstack((
                Color::srgb_hex("#3B82F6")
                    .size(30.0, 100.0)
                    .transform(Transform::scale_xy(1.0_f32, animated_y_scale)),
            ))
            .min_height(120.0)
            .min_width(50.0),
            spacer(),
            // Rotating square
            zstack((
                Color::srgb_hex("#EF4444")
                    .size(50.0, 50.0)
                    .transform(Transform::rotation(animated_rotation)),
            ))
            .min_height(80.0)
            .min_width(80.0),
            spacer(),
            // Scaling square
            zstack((
                Color::srgb_hex("#22C55E")
                    .size(60.0, 60.0)
                    .transform(Transform::scale(animated_scale)),
            ))
            .min_height(100.0)
            .min_width(100.0),
        )),
        Slider::new(0.0..=100.0, &size_value),
        hstack((
            {
                let v = size_value.clone();
                button("0").action(move || v.set(0.0))
            },
            {
                let v = size_value.clone();
                button("25").action(move || v.set(25.0))
            },
            {
                let v = size_value.clone();
                button("50").action(move || v.set(50.0))
            },
            {
                let v = size_value.clone();
                button("75").action(move || v.set(75.0))
            },
            {
                let v = size_value.clone();
                button("100").action(move || v.set(100.0))
            },
        )),
    ))
    .padding()
}

#[hot_reload]
fn main() -> impl View {
    // State for transform sections
    let scale = Binding::container(1.0_f32);
    let rotation = Binding::container(0.0_f32);
    let offset_x = Binding::container(0.0_f32);
    let offset_y = Binding::container(0.0_f32);
    let combined_scale = Binding::container(1.0_f32);
    let combined_rotation = Binding::container(0.0_f32);

    // State for other demo sections
    let progress_value = Binding::container(0.3_f64);
    let spring_value = Binding::container(0.2_f64);
    let bar_scale = Binding::container(0.7_f32);
    let toggle_state = Binding::bool(false);
    let staggered_expanded = Binding::bool(true);
    let size_value = Binding::container(50.0_f64);

    scroll(
        vstack((
            // Header
            text("WaterUI Animation Examples").size(28.0),
            "Visual demonstrations of the animation system",
            Divider,
            // Transform animations - the most visual demos
            vstack((
                scale_animation_section(scale),
                Divider,
                rotation_animation_section(rotation),
                Divider,
                translation_animation_section(offset_x, offset_y),
                Divider,
                combined_transform_section(combined_scale, combined_rotation),
            )),
            // Progress and value animations
            vstack((
                Divider,
                progress_animation_section(progress_value),
                Divider,
                spring_progress_section(spring_value),
                Divider,
                size_indicator_section(size_value),
            )),
            // Animation curves and staggered demos
            vstack((
                Divider,
                animation_curves_section(bar_scale),
                Divider,
                toggle_animation_section(toggle_state),
                Divider,
                staggered_section(staggered_expanded),
            )),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
