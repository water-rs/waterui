use waterui::{
    Binding, Environment, View,
    component::{
        divder::Divider,
        form::{FormBuilder, Slider, form, stepper},
        layout::{
            spacer,
            stack::{hstack, vstack},
        },
        progress::{loading, progress},
        text::text,
    },
    reactive::Project,
};

pub fn init() -> Environment {
    Environment::new()
}

// Demo form data structure
#[derive(Default, Clone, Debug, FormBuilder, Project)]
struct UserProfile {
    name: String,
    email: String,
    age: i32,
    notifications: bool,
    theme_brightness: f32,
}

pub fn main() -> impl View {
    // Reactive state
    let profile = UserProfile::binding();
    let counter = Binding::int(0);
    let progress_value = Binding::container(0.3);

    vstack((
        // App header
        vstack((
            text("WaterUI Demo").size(24.0),
            "Cross-platform Reactive UI Framework",
            Divider,
        )),
        spacer(),
        // Counter demo with reactive updates
        vstack((
            text("Interactive Counter").size(18.0),
            hstack((
                "Count: ",
                waterui::text!("{}", counter),
                spacer(),
                stepper(&counter),
            )),
            progress(counter.get() as f64 / 10.0),
        )),
        spacer(),
        // User profile form
        vstack((
            text("User Profile").size(18.0),
            form(&profile),
            hstack(("Name: ", waterui::text!("{}", profile.project().name))),
            hstack(("Email: ", waterui::text!("{}", profile.project().email))),
        )),
        spacer(),
        // Interactive controls
        vstack((
            text("Controls").size(18.0),
            hstack(("Progress: ", Slider::new(0.0..=1.0, &progress_value))),
            progress(progress_value.get()),
            loading(),
        )),
        spacer(),
        Divider,
        "Built with WaterUI - Cross-platform Reactive UI Framework",
    ))
}

waterui_ffi::export!();
