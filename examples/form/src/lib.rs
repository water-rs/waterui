//! Form Example - Demonstrates WaterUI's form building capabilities
//!
//! This example showcases:
//! - The `#[form]` derive macro for automatic form generation
//! - Various form field types (text, bool, numeric, slider)
//! - Reactive data binding with live preview
//! - Manual form control composition

use waterui::Str;
use waterui::app::App;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::text::font::FontWeight;
use waterui::text::font::ResolvedFont;

// User registration form using the derive macro
// The #[form] attribute automatically generates form fields for each struct field
#[form]
struct RegistrationForm {
    /// Full name of the user
    full_name: String,
    /// Email address for account
    email: String,
    /// Age in years
    age: i32,
    /// Whether to receive marketing emails
    newsletter: bool,
    /// Preferred volume level (0.0 - 1.0)
    volume: f64,
}

// Settings form demonstrating different field types
#[form]
struct AppSettings {
    /// Application theme brightness
    brightness: f64,
    /// Enable dark mode
    dark_mode: bool,
    /// Font size multiplier
    font_scale: f32,
    /// Auto-save interval (minutes)
    auto_save_minutes: i32,
    /// Enable notifications
    notifications_enabled: bool,
}

fn main(settings: &Binding<AppSettings>) -> impl View {
    // Create reactive bindings for both forms
    let registration = RegistrationForm::binding();
    // Manual form controls for demonstration
    let custom_name = binding("");
    let custom_enabled = binding(false);
    let custom_count = binding(5);
    let custom_slider = binding(0.5);

    scroll(
        vstack((
            // Header
            text("WaterUI Form Examples").size(28.0),
            "Demonstrating form building with reactive data binding",
            Divider,
            spacer(),
            // Section 1: Auto-generated Registration Form
            vstack((
                text("Registration Form").size(20.0),
                "Using #[form] derive macro",
                form(&registration),
                Divider,
                // Live preview of form data
                text("Live Preview:").bold(),
                hstack((
                    "Name: ",
                    waterui::text!("{}", registration.project().full_name),
                )),
                hstack((
                    "Email: ",
                    waterui::text!("{}", registration.project().email),
                )),
                hstack(("Age: ", waterui::text!("{}", registration.project().age))),
                hstack((
                    "Newsletter: ",
                    waterui::text!("{}", registration.project().newsletter),
                )),
                hstack((
                    "Volume: ",
                    waterui::text!("{}", registration.project().volume),
                )),
            )),
            spacer(),
            // Section 2: Settings Form
            vstack((
                text("App Settings").size(20.0),
                "Another form with different field types",
                form(&settings),
                Divider,
                text("Current Settings:").bold(),
                hstack((
                    "Dark Mode: ",
                    waterui::text!("{}", settings.project().dark_mode),
                )),
                hstack((
                    "Brightness: ",
                    waterui::text!("{:.4}", settings.project().brightness),
                )),
            )),
            spacer(),
            // Section 3: Manual Form Controls
            vstack((
                text("Manual Form Controls").size(20.0),
                "Building forms manually with individual controls",
                // TextField with label and placeholder
                TextField::new(&custom_name)
                    .label(text("Username"))
                    .prompt("Enter your username"),
                // Toggle with label
                Toggle::new(&custom_enabled).label(text("Enable Feature")),
                // Stepper with custom range
                Stepper::new(&custom_count)
                    .label(text("Item Count"))
                    .range(0..=100)
                    .step(5),
                // Slider with label
                Slider::new(0.0..=1.0, &custom_slider).label(text("Progress")),
                // Progress bar showing slider value
                progress(custom_slider.clone()),
                Divider,
                text("Manual Controls Preview:").bold(),
                hstack(("Username: ", waterui::text!("{}", custom_name))),
                hstack(("Feature Enabled: ", waterui::text!("{}", custom_enabled))),
                hstack(("Count: ", waterui::text!("{}", custom_count))),
                hstack(("Progress: ", waterui::text!("{}", custom_slider))),
            )),
            spacer(),
            Divider,
            "Built with WaterUI Form Components",
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

pub fn app(mut env: Environment) -> App {
    let settings = AppSettings::binding();

    // Install theme before creating App
    let theme =
        Theme::new()
            .color_scheme(
                settings
                    .project()
                    .dark_mode
                    .select(ColorScheme::Dark, ColorScheme::Light),
            )
            .fonts(FontSettings::new().body(
                settings.project().font_scale.map(|scale| {
                    ResolvedFont::new(16.0 + (1.0 + scale * 10.0), FontWeight::Normal)
                }),
            ));

    env.install(theme);

    App::new(main(&settings), env)
}

waterui_ffi::export!();
