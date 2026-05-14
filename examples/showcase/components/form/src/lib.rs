//! Form Example - Demonstrates WaterUI's form building capabilities
//!
//! This example showcases:
//! - The `#[form]` derive macro for automatic form generation
//! - Various form field types (text, bool, numeric, slider)
//! - Reactive data binding with live preview
//! - Manual form control composition
use waterui::app::App;
use waterui::prelude::*;
use waterui::preview;
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

fn main(
    settings: Binding<AppSettings>,
    registration: Binding<RegistrationForm>,
    custom_name: Binding<Str>,
    custom_enabled: Binding<bool>,
    custom_count: Binding<i32>,
    custom_slider: Binding<f64>,
) -> impl View {
    scroll(
        vstack((
            // Header
            text("WaterUI Form Examples").title(),
            "Demonstrating form building with reactive data binding",
            Divider,
            spacer(),
            // Section 1: Auto-generated Registration Form
            vstack((
                text("Registration Form").sub_headline(),
                "Using #[form] derive macro",
                form(&registration),
                Divider,
                // Live preview of form data
                text("Live Preview:").bold(),
                text!(
                    "Name: {registration}",
                    registration = registration.project().full_name
                ),
                text!("Email: {email}", email = registration.project().email),
                text!("Age: {age}", age = registration.project().age),
                text!(
                    "Newsletter: {newsletter}",
                    newsletter = registration.project().newsletter
                ),
                text!("Volume: {volume}", volume = registration.project().volume),
            )),
            spacer(),
            // Section 2: Settings Form
            vstack((
                text("App Settings").sub_headline(),
                "Another form with different field types",
                form(&settings),
                Divider,
                text("Current Settings:").bold(),
                hstack((
                    "Dark Mode: ",
                    text!("{dark_mode}", dark_mode = settings.project().dark_mode),
                )),
                hstack((
                    "Brightness: ",
                    text!(
                        "{brightness:.4}",
                        brightness = settings.project().brightness
                    ),
                )),
            )),
            spacer(),
            // Section 3: Manual Form Controls
            vstack((
                text("Manual Form Controls").sub_headline(),
                "Building forms manually with individual controls",
                // TextField with label and placeholder
                TextField::new(&custom_name)
                    .label("Username")
                    .prompt("Enter your username"),
                // Toggle with label
                Toggle::new(&custom_enabled).label("Enable Feature"),
                // Stepper with custom range
                Stepper::new("Item Count", &custom_count)
                    .range(0..=100)
                    .step(5),
                // Slider with label
                Slider::new("Progress", &custom_slider),
                // Progress bar showing slider value
                progress(custom_slider.clone()),
                Divider,
                text("Manual Controls Preview:").bold(),
                text!("Username: {custom_name}"),
                text!("Feature Enabled: {custom_enabled}"),
                text!("Count: {custom_count}"),
                text!("Progress: {custom_slider}"),
            )),
            spacer(),
            Divider,
            "Built with WaterUI Form Components",
        ))
        .padding(),
    )
}

pub fn app(mut env: Environment) -> App {
    let settings = AppSettings::binding();
    let registration = RegistrationForm::binding();
    let custom_name = binding("");
    let custom_enabled = binding(false);
    let custom_count = binding(5);
    let custom_slider = binding(0.5);

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

    App::new(
        move || {
            main(
                settings.clone(),
                registration.clone(),
                custom_name.clone(),
                custom_enabled.clone(),
                custom_count.clone(),
                custom_slider.clone(),
            )
        },
        env,
    )
}

// Preview function for testing the preview system
#[preview]
fn sample_card() -> impl View {
    vstack((
        text("Sample Card").title(),
        "This is a preview test",
        Divider,
        hstack((text("Status:").bold(), text("Active"))),
    ))
    .padding_with(EdgeInsets::all(16.0))
}

#[preview]
fn form_preview() -> impl View {
    main(
        AppSettings::binding(),
        RegistrationForm::binding(),
        binding(""),
        binding(false),
        binding(5),
        binding(0.5),
    )
}
