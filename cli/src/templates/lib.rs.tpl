use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;

// Demo form data structure
#[form]
struct UserProfile {
    name: String,
    email: String,
    age: i32,
    notifications: bool,
    theme_brightness: f64,
}

#[preview]
fn main() -> impl View {
    // Reactive state
    let profile = UserProfile::binding();
    let counter = Binding::i32(0);
    let progress_value = Binding::f64(0.3);

    scroll(
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
                    text!("{counter}"),
                    spacer(),
                    stepper("Count", &counter),
                )),
                progress(counter.map(|count| count as f64 / 10.0)),
            )),
            spacer(),
            // User profile form
            {
                let proj = profile.project();
                vstack((
                    text("User Profile").size(18.0f32),
                    form(&profile),
                    text!("Name: {name}", name = proj.name).bold(),
                    text!("Email: {email}", email = proj.email),
                ))
            },
            spacer(),
            // Interactive controls
            vstack((
                text("Controls").size(18.0f32),
                slider("Progress", &progress_value),
                progress(progress_value),
                loading(),
            )),
            spacer(),
            Divider,
            "Built with WaterUI - Cross-platform Reactive UI Framework",
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}
