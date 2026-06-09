//! Animation Example - Demonstrates WaterUI's animation system
//!
//! This example showcases visual animations:
//! - Scale/rotation/translation animations
//! - Progress bar animations with smooth value transitions
//! - Text transitions with cross-fade effects
//! - Toggle state animations
//! - Different animation curves (linear, ease-in, ease-out, spring)
//! - Framework-driven GPU morph animation (not expressible as a simple native property transform)
//!
//! Animations in WaterUI are reactive - they automatically apply
//! when reactive values change, using the `.animated()` or
//! `.with(Animation::...)` modifiers.

use core::time::Duration;
use waterui::animation::Animation;
use waterui::app::App;
use waterui::prelude::slider::slider;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::Binding;
use waterui::shape::{Capsule, Circle, Rectangle, RoundedRectangle, ShapeExt};

const SCALE_BOX_SIDE: f32 = 80.0;
const SCALE_STAGE_SIDE: f32 = SCALE_BOX_SIDE * 2.25;
const ROTATION_BOX_SIDE: f32 = 60.0;
const ROTATION_STAGE_SIDE: f32 = ROTATION_BOX_SIDE * 1.8;
const TRANSLATION_BOX_SIDE: f32 = 50.0;
const TRANSLATION_STAGE_SIDE: f32 = TRANSLATION_BOX_SIDE * 3.0;
const COMBINED_BOX_SIDE: f32 = 60.0;
const COMBINED_STAGE_SIDE: f32 = COMBINED_BOX_SIDE * 3.0;

fn transform_stage(content: impl View, side: f32) -> impl View {
    content.size(side, side)
}

fn set_f32_button(label: &'static str, value: f32, binding: &Binding<f32>) -> impl View {
    button(label)
        .action(move |State(current): State<Binding<f32>>| current.set(value))
        .state(binding)
}

fn set_f64_button(label: &'static str, value: f64, binding: &Binding<f64>) -> impl View {
    button(label)
        .action(move |State(current): State<Binding<f64>>| current.set(value))
        .state(binding)
}

/// Demo: Scale animation - visual transform on colored boxes
fn scale_animation_section(scale: &Binding<f32>) -> impl View {
    let animated_scale = scale.with(Animation::spring(300.0, 15.0));

    vstack((
        text("Scale Animation").headline(),
        text("Click buttons to scale the box with spring physics").body(),
        transform_stage(
            Blue.size(SCALE_BOX_SIDE, SCALE_BOX_SIDE)
                .scale(animated_scale.clone(), animated_scale.clone()),
            SCALE_STAGE_SIDE,
        ),
        hstack((
            set_f32_button("0.5x", 0.5, scale),
            set_f32_button("1x", 1.0, scale),
            set_f32_button("1.5x", 1.5, scale),
            set_f32_button("2x", 2.0, scale),
        )),
    ))
    .padding()
}

/// Demo: Rotation animation - spinning box
fn rotation_animation_section(rotation: &Binding<f32>) -> impl View {
    let animated_rotation = rotation.with(Animation::ease_in_out(Duration::from_millis(500)));

    vstack((
        text("Rotation Animation").headline(),
        text("Rotate the box smoothly").body(),
        transform_stage(
            Green
                .size(ROTATION_BOX_SIDE, ROTATION_BOX_SIDE)
                .rotation(animated_rotation),
            ROTATION_STAGE_SIDE,
        ),
        vstack((
            hstack((
                button("-90°")
                    .action(|State(r): State<Binding<f32>>| r.set(r.get() - 90.0))
                    .state(rotation),
                button("-45°")
                    .action(|State(r): State<Binding<f32>>| r.set(r.get() - 45.0))
                    .state(rotation),
                set_f32_button("Reset", 0.0, rotation),
            )),
            hstack((
                button("+45°")
                    .action(|State(r): State<Binding<f32>>| r.set(r.get() + 45.0))
                    .state(rotation),
                button("+90°")
                    .action(|State(r): State<Binding<f32>>| r.set(r.get() + 90.0))
                    .state(rotation),
            )),
        )),
    ))
    .padding()
}

/// Demo: Translation animation - moving box
fn translation_animation_section(offset_x: &Binding<f32>, offset_y: &Binding<f32>) -> impl View {
    let animated_x = offset_x.with(Animation::spring(200.0, 20.0));
    let animated_y = offset_y.with(Animation::spring(200.0, 20.0));
    let center_x = offset_x.clone();
    let center_y = offset_y.clone();

    vstack((
        text("Translation Animation").headline(),
        text("Move the box with spring physics").body(),
        transform_stage(
            Purple
                .size(TRANSLATION_BOX_SIDE, TRANSLATION_BOX_SIDE)
                .offset(animated_x, animated_y),
            TRANSLATION_STAGE_SIDE,
        ),
        vstack((
            hstack((
                button("Center").action(move || {
                    center_x.set(0.0);
                    center_y.set(0.0);
                }),
                set_f32_button("Left", -50.0, offset_x),
                set_f32_button("Right", 50.0, offset_x),
            )),
            hstack((
                set_f32_button("Up", -30.0, offset_y),
                set_f32_button("Down", 30.0, offset_y),
            )),
        )),
    ))
    .padding()
}

/// Demo: Combined transform animation
fn combined_transform_section(
    combined_scale: &Binding<f32>,
    combined_rotation: &Binding<f32>,
) -> impl View {
    let animated_scale = combined_scale.with(Animation::spring(250.0, 18.0));
    let animated_rotation =
        combined_rotation.with(Animation::ease_in_out(Duration::from_millis(400)));
    let reset_scale = combined_scale.clone();
    let reset_rotation = combined_rotation.clone();
    let grow_scale = combined_scale.clone();
    let grow_rotation = combined_rotation.clone();

    vstack((
        text("Combined Transforms").headline(),
        text("Scale and rotation together").body(),
        transform_stage(
            Orange
                .size(COMBINED_BOX_SIDE, COMBINED_BOX_SIDE)
                .scale(animated_scale.clone(), animated_scale.clone())
                .rotation(animated_rotation),
            COMBINED_STAGE_SIDE,
        ),
        hstack((
            button("Reset").action(move || {
                reset_scale.set(1.0);
                reset_rotation.set(0.0);
            }),
            button("Grow + Spin").action(move || {
                grow_scale.set(1.8);
                grow_rotation.set(grow_rotation.get() + 180.0);
            }),
            button("Pulse")
                .action(|State(s): State<Binding<f32>>| {
                    if s.get() > 1.2 {
                        s.set(0.8);
                    } else {
                        s.set(1.5);
                    }
                })
                .state(combined_scale),
        )),
    ))
    .padding()
}

/// Demo: Animated progress bar - the most visually impressive animation
fn progress_animation_section(progress_value: &Binding<f64>) -> impl View {
    let animated_progress = progress_value.with(Animation::ease_in_out(Duration::from_millis(800)));

    vstack((
        text("Progress Bar Animation").headline(),
        text("Watch the bar smoothly transition between values").body(),
        progress(animated_progress),
        vstack((
            hstack((
                set_f64_button("0%", 0.0, progress_value),
                set_f64_button("25%", 0.25, progress_value),
                set_f64_button("50%", 0.5, progress_value),
            )),
            hstack((
                set_f64_button("75%", 0.75, progress_value),
                set_f64_button("100%", 1.0, progress_value),
            )),
        )),
    ))
    .padding()
}

/// Demo: Progress with spring physics - bouncy feel
fn spring_progress_section(spring_value: &Binding<f64>) -> impl View {
    let animated_progress = spring_value.with(Animation::spring(200.0, 12.0));
    let status = spring_value.gt(0.5).select("High", "Low").animated();

    vstack((
        text("Spring Physics Animation").headline(),
        text("Notice the bouncy overshoot effect").body(),
        progress(animated_progress),
        hstack((
            text!("{status}")
                .padding()
                .background(Purple.with_opacity(0.3)),
            spacer(),
            button("Toggle")
                .action(|State(sv): State<Binding<f64>>| {
                    if sv.get() > 0.5 {
                        sv.set(0.1);
                    } else {
                        sv.set(0.9);
                    }
                })
                .state(spring_value),
        )),
    ))
    .padding()
}

/// Demo: Animation curves comparison using visual bars
fn animation_curves_section(bar_scale: &Binding<f32>) -> impl View {
    // Same value animated with different curves - visually compare timing
    let linear_scale = bar_scale.with(Animation::linear(Duration::from_millis(1000)));
    let ease_in_scale = bar_scale.with(Animation::ease_in(Duration::from_millis(1000)));
    let ease_out_scale = bar_scale.with(Animation::ease_out(Duration::from_millis(1000)));
    let spring_scale = bar_scale.with(Animation::spring(150.0, 12.0));

    vstack((
        text("Animation Curves Comparison").headline(),
        "Watch how different curves animate at different speeds",
        vstack((
            // Linear - constant speed
            hstack((
                text("Linear").min_width(80.0),
                Cyan.size(200.0, 24.0)
                    .scale(linear_scale.clone(), 1.0)
                    .min_width(220.0),
            )),
            // Ease-in - starts slow, ends fast
            hstack((
                text("Ease-In").min_width(80.0),
                Green
                    .size(200.0, 24.0)
                    .scale(ease_in_scale.clone(), 1.0)
                    .min_width(220.0),
            )),
            // Ease-out - starts fast, ends slow
            hstack((
                text("Ease-Out").min_width(80.0),
                Orange
                    .size(200.0, 24.0)
                    .scale(ease_out_scale.clone(), 1.0)
                    .min_width(220.0),
            )),
            // Spring - bouncy overshoot
            hstack((
                text("Spring").min_width(80.0),
                Red.size(200.0, 24.0)
                    .scale(spring_scale.clone(), 1.0)
                    .min_width(220.0),
            )),
        )),
        hstack((
            set_f32_button("Small (0.3)", 0.3, bar_scale),
            set_f32_button("Medium (0.7)", 0.7, bar_scale),
            set_f32_button("Large (1.0)", 1.0, bar_scale),
        )),
    ))
    .padding()
}

/// Demo: Toggle with animated indicator
fn toggle_animation_section(toggle_state: &Binding<bool>) -> impl View {
    // Animated scale for the indicator
    let indicator_scale = toggle_state
        .select(1.0, 0.3)
        .with(Animation::spring(300.0, 15.0));

    // Animated rotation
    let indicator_rotation = toggle_state
        .select(0.0, 180.0)
        .with(Animation::ease_in_out(Duration::from_millis(400)));

    vstack((
        text("Toggle Animation").headline(),
        text("Watch the indicator scale and rotate with toggle").body(),
        hstack((
            Toggle::new(toggle_state).label("Power"),
            spacer(),
            // Visual indicator that animates
            Green
                .size(60.0, 60.0)
                .scale(indicator_scale.clone(), indicator_scale.clone())
                .rotation(indicator_rotation)
                .min_width(80.0)
                .min_height(80.0),
        )),
    ))
    .padding()
}

/// Demo: Staggered animations with bars
fn staggered_section(expanded: &Binding<bool>) -> impl View {
    // Each bar has different spring stiffness, creating a cascading effect
    let bar1_scale = expanded
        .select(1.0, 0.2)
        .with(Animation::spring(200.0, 15.0));
    let bar2_scale = expanded
        .select(1.0, 0.2)
        .with(Animation::spring(150.0, 15.0));
    let bar3_scale = expanded
        .select(1.0, 0.2)
        .with(Animation::spring(100.0, 15.0));
    let bar4_scale = expanded
        .select(1.0, 0.2)
        .with(Animation::spring(80.0, 12.0));

    vstack((
        text("Staggered Animations").headline(),
        text("Different spring stiffness creates cascading effect").body(),
        hstack((
            Green
                .size(50.0, 80.0)
                .scale(1.0, bar1_scale.clone())
                .min_height(100.0)
                .min_width(60.0),
            Blue.size(50.0, 80.0)
                .scale(1.0, bar2_scale.clone())
                .min_height(100.0)
                .min_width(60.0),
            Purple
                .size(50.0, 80.0)
                .scale(1.0, bar3_scale.clone())
                .min_height(100.0)
                .min_width(60.0),
            Orange
                .size(50.0, 80.0)
                .scale(1.0, bar4_scale.clone())
                .min_height(100.0)
                .min_width(60.0),
        )),
        button("Toggle Bars")
            .action(|State(e): State<Binding<bool>>| e.set(!e.get()))
            .state(expanded),
    ))
    .padding()
}

/// Demo: Animated size indicator - visual elements respond to value
fn size_indicator_section(size_value: &Binding<f64>) -> impl View {
    // Animated scale based on value (0-100 maps to 0.1-1.0 scale)
    let animated_scale = size_value
        .clone()
        .map(|s| s / 100.0 + 0.1)
        .with(Animation::spring(200.0, 15.0));

    let animated_rotation = size_value
        .clone()
        .map(|s| s * 3.6) // 0-100 maps to 0-360 degrees
        .with(Animation::ease_in_out(Duration::from_millis(600)));

    let animated_y_scale = size_value
        .clone()
        .map(|s| s / 100.0)
        .with(Animation::spring(180.0, 14.0));

    vstack((
        text("Size Indicator").headline(),
        "All elements respond to slider with different animations",
        hstack((
            // Vertical bar that scales vertically
            Blue.size(30.0, 100.0)
                .scale(1.0, animated_y_scale.clone())
                .min_height(120.0)
                .min_width(50.0),
            spacer(),
            // Rotating square
            Red.size(50.0, 50.0)
                .rotation(animated_rotation)
                .min_height(80.0)
                .min_width(80.0),
            spacer(),
            // Scaling square
            Green
                .size(60.0, 60.0)
                .scale(animated_scale.clone(), animated_scale.clone())
                .min_height(100.0)
                .min_width(100.0),
        )),
        slider("Animation size", size_value)
            .range(0.0..=100.0)
            .hide_label(),
        vstack((
            hstack((
                set_f64_button("0", 0.0, size_value),
                set_f64_button("25", 25.0, size_value),
                set_f64_button("50", 50.0, size_value),
            )),
            hstack((
                set_f64_button("75", 75.0, size_value),
                set_f64_button("100", 100.0, size_value),
            )),
        )),
    ))
    .padding()
}

/// Demo: Framework-side custom animation rendered by WaterUI GPU pipeline.
///
/// This is intentionally not a simple native transform animation (scale/rotation/offset).
/// Geometry morphing is produced by WaterUI's renderer-side interpolation pipeline.
fn custom_gpu_animation_section() -> impl View {
    vstack((
        text("Framework GPU Animation").headline(),
        text("Shape geometry morphing (renderer-side, not plain Core Animation transform)").body(),
        hstack((
            Circle
                .morph_to(RoundedRectangle::new(0.22), Color::srgb_hex("#3B82F6"))
                .duration(Duration::from_millis(1100))
                .size(90.0, 90.0),
            Rectangle
                .morph_to(Capsule, Color::srgb_hex("#10B981"))
                .duration(Duration::from_millis(900))
                .autoreverse(true)
                .size(128.0, 72.0),
        ))
        .spacing(16.0),
    ))
    .padding()
}

#[preview]
fn main() -> impl View {
    // State for transform sections
    let scale = Binding::f32(1.0);
    let rotation = Binding::f32(0.0);
    let offset_x = Binding::f32(0.0);
    let offset_y = Binding::f32(0.0);
    let combined_scale = Binding::f32(1.0);
    let combined_rotation = Binding::f32(0.0);

    // State for other demo sections
    let progress_value = Binding::f64(0.3);
    let spring_value = Binding::f64(0.2);
    let bar_scale = Binding::f32(0.7);
    let toggle_state = Binding::bool(false);
    let staggered_expanded = Binding::bool(true);
    let size_value = Binding::f64(50.0);

    scroll(
        vstack((
            // Header
            text("WaterUI Animation Examples").title(),
            "Visual demonstrations of the animation system",
            Divider,
            // Scale/rotation/translation animations - the most visual demos
            vstack((
                scale_animation_section(&scale),
                Divider,
                rotation_animation_section(&rotation),
                Divider,
                translation_animation_section(&offset_x, &offset_y),
                Divider,
                combined_transform_section(&combined_scale, &combined_rotation),
                Divider,
                custom_gpu_animation_section(),
            )),
            // Progress and value animations
            vstack((
                Divider,
                progress_animation_section(&progress_value),
                Divider,
                spring_progress_section(&spring_value),
                Divider,
                size_indicator_section(&size_value),
            )),
            // Animation curves and staggered demos
            vstack((
                Divider,
                animation_curves_section(&bar_scale),
                Divider,
                toggle_animation_section(&toggle_state),
                Divider,
                staggered_section(&staggered_expanded),
            )),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
