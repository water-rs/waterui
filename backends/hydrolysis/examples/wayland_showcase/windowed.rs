//! The example program, compiled where `hydrolysis::run` exists.

use std::str::FromStr as _;
use std::thread;
use std::time::Duration;

use hydrolysis::run;
use waterui::Environment;
use waterui::app::{App, Windows};
use waterui::component::list::List;
use waterui::component::progress::loading;
use waterui::component::table::{col, table};
use waterui::form::picker::{Picker, PickerStyle};
use waterui::form::secure::{Secure, SecureField};
use waterui::id::SelfId;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::window::Window;
use waterui_controls::{slider::slider, stepper::stepper};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum RenderMode {
    #[default]
    Balanced,
    Quality,
    Speed,
}

impl RenderMode {
    fn options() -> Vec<(Self, &'static str)> {
        vec![
            (Self::Balanced, "Balanced"),
            (Self::Quality, "Quality"),
            (Self::Speed, "Speed"),
        ]
    }
}

fn main_view() -> impl View {
    let username = binding("lexo");
    let password = binding(Secure::from_str("hunter2").expect("static secure seed must parse"));
    let toggle_value = binding(true);
    let slider_value = binding::<f64>(0.62);
    let stepper_value = binding::<i32>(4);
    let list_editing = binding(false);
    let render_mode = binding(RenderMode::Balanced);
    let diagnostics_rows = vec![
        SelfId::new(("Renderer", "Hydrolysis", "#166534")),
        SelfId::new(("Scene backend", "Cherenkov", "#0F766E")),
        SelfId::new(("Event routing", "Pointer/Key/IME", "#7C3AED")),
    ];

    let picker_items: Vec<_> = RenderMode::options()
        .into_iter()
        .map(|(mode, label)| text(label).tag(mode))
        .collect();

    scroll(
        vstack((
            text("Hydrolysis Wayland Showcase").font_size(34.0),
            text("Control matrix on pure-Rust backend").font_size(16.0),
            TextField::new("Username", &username).prompt("Type your name"),
            SecureField::new("Password", &password),
            Toggle::new("Realtime rebuild", &toggle_value),
            slider("Animation blend", &slider_value),
            stepper("Detail level", &stepper_value).range(1..=12),
            hstack((loading(), text("Background task running").font_size(14.0))).spacing(12.0),
            Picker::new("Render mode", picker_items, &render_mode).style(PickerStyle::Menu),
            Toggle::new("List editing", &list_editing),
            List::for_each(diagnostics_rows, |row| {
                let (name, value, color) = row.into_inner();
                hstack((
                    text(name),
                    spacer(),
                    text(value).foreground(Color::srgb_hex(color)),
                ))
            })
            .editing(list_editing.clone()),
            table(
                vec![
                    SelfId::new(("Window Size", "1366x900", "Ready")),
                    SelfId::new(("Surface", "Wayland", "Ready")),
                    SelfId::new(("Text Pipeline", "Parley", "Ready")),
                    SelfId::new(("Filters", "GPU AppliedFilter", "Ready")),
                ],
                (
                    col("Metric", |row: &SelfId<(&str, &str, &str)>| {
                        text((*row).into_inner().0)
                    }),
                    col("Value", |row: &SelfId<(&str, &str, &str)>| {
                        text((*row).into_inner().1)
                    }),
                    col("Status", |row: &SelfId<(&str, &str, &str)>| {
                        text((*row).into_inner().2)
                    }),
                ),
            ),
        ))
        .spacing(14.0)
        .padding_with(EdgeInsets::all(24.0)),
    )
    .background(Color::srgb_hex("#EEF2FF"))
    .foreground(Color::srgb_hex("#0F172A"))
}

fn app(env: Environment) -> App<impl Windows> {
    let window = Window::new("Hydrolysis Wayland Showcase", main_view)
        .resizable(true)
        .default_size(Size::new(1366.0, 900.0))
        .background(Color::srgb_hex("#EEF2FF"));
    App::new(env).window(window)
}

fn showcase_lifetime() -> Duration {
    let seconds = std::env::var("HYDROLYSIS_WAYLAND_SECONDS").map_or(15, |value| {
        value
            .parse::<u64>()
            .expect("HYDROLYSIS_WAYLAND_SECONDS must be an unsigned integer")
    });
    Duration::from_secs(seconds)
}

pub fn main() {
    thread::spawn(|| {
        thread::sleep(showcase_lifetime());
        std::process::exit(0);
    });

    // `run` installs the style's tokens (fonts, colors) over the framework
    // defaults before the first frame; the app-level environment carries none.
    run(
        app,
        Environment::new(),
        hydrolysis::testing::TestStyle::new(),
    );
}
