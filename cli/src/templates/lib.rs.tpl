use waterui::prelude::*;
#[cfg(not(target_arch = "wasm32"))]
use waterui::hot_reload::Hotreload;

pub fn init() -> Environment {
    Environment::new()
}

// Demo form data structure
#[form]
struct UserProfile {
    name: String,
    email: String,
    age: i32,
    notifications: bool,
    theme_brightness: f64,
}

pub fn main() -> impl View {
    // Reactive state
    let profile = UserProfile::binding();
    let counter = Binding::int(0);
    let progress_value = Binding::container(0.3);

    let view = scroll(
        vstack((
            // App header
            vstack((
                text("WaterUI Demo").size(24),
                "Cross-platform Reactive UI Framework",
                Divider,
            )),
            spacer(),
            // Counter demo with reactive updates
            vstack((
                text("Interactive Counter").size(18),
                hstack((
                    "Count: ",
                    waterui::text!("{}", counter),
                    spacer(),
                    stepper(&counter),
                )),
                progress(counter.map(|count| count as f64 / 10.0)),
            )),
            spacer(),
            // User profile form
            vstack((
                text("User Profile").size(18.0f32),
                form(&profile),
                hstack((
                    "Name: ",
                    waterui::text!("{}", profile.project().name).bold(),
                )),
                hstack(("Email: ", waterui::text!("{}", profile.project().email))),
            )),
            spacer(),
            // Interactive controls
            vstack((
                text("Controls").size(18.0f32),
                Slider::new(0.0..=1.0, &progress_value),
                progress(progress_value),
                loading(),
            )),
            spacer(),
            Divider,
            "Built with WaterUI - Cross-platform Reactive UI Framework",
        ))
        .padding_with(EdgeInsets::all(100.0)),
    );

    view
}

waterui_ffi::export!();
