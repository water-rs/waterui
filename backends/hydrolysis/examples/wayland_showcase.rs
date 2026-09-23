use std::str::FromStr as _;
use std::thread;
use std::time::Duration;

use hydrolysis::run;
use waterui::Environment;
use waterui::app::App;
use waterui::component::list::{List, ListItem};
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
    let slider_value = binding(0.62_f64);
    let stepper_value = binding(4_i32);
    let list_editing = binding(false);
    let render_mode = binding(RenderMode::Balanced);
    let diagnostics_rows = vec![
        SelfId::new(("Renderer", "Hydrolysis", "#166534")),
        SelfId::new(("Scene backend", "Vello Scene2D", "#0F766E")),
        SelfId::new(("Event routing", "Pointer/Key/IME", "#7C3AED")),
    ];

    let picker_items: Vec<_> = RenderMode::options()
        .into_iter()
        .map(|(mode, label)| text(label).tag(mode))
        .collect();

    scroll(
        vstack((
            text("Hydrolysis Wayland Showcase").size(34.0),
            text("Control matrix on pure-Rust backend").size(16.0),
            TextField::new(&username)
                .label("Username")
                .prompt("Type your name"),
            SecureField::new("Password", &password),
            Toggle::new(&toggle_value).label("Realtime rebuild"),
            slider("Animation blend", &slider_value),
            stepper("Detail level", &stepper_value).range(1..=12),
            hstack((loading(), text("Background task running").size(14.0))).spacing(12.0),
            Picker::new(picker_items, &render_mode).style(PickerStyle::Menu),
            Toggle::new(&list_editing).label("List editing"),
            List::for_each(diagnostics_rows, |row| {
                let (name, value, color) = row.into_inner();
                ListItem::new(hstack((
                    text(name),
                    spacer(),
                    text(value).foreground(Color::srgb_hex(color)),
                )))
            })
            .editing(list_editing.clone()),
            table(vec![
                col(
                    "Metric",
                    vec![
                        text("Window Size"),
                        text("Surface"),
                        text("Text Pipeline"),
                        text("Filters"),
                    ],
                ),
                col(
                    "Value",
                    vec![
                        text("1366x900"),
                        text("Wayland"),
                        text("Parley"),
                        text("GPU AppliedFilter"),
                    ],
                ),
                col(
                    "Status",
                    vec![text("Ready"), text("Ready"), text("Ready"), text("Ready")],
                ),
            ]),
        ))
        .spacing(14.0)
        .padding_with(EdgeInsets::all(24.0)),
    )
    .background(Color::srgb_hex("#EEF2FF"))
    .foreground(Color::srgb_hex("#0F172A"))
}

fn app() -> App {
    let env = Environment::new();
    let window = Window::new(
        "Hydrolysis Wayland Showcase",
        binding(waterui::window::WindowState::Normal),
        main_view,
    )
    .resizable(true)
    .background(Color::srgb_hex("#EEF2FF"));
    window
        .frame
        .set(Rect::new(Point::zero(), Size::new(1366.0, 900.0)));
    App::new_with_windows([window], env)
}

fn showcase_lifetime() -> Duration {
    let seconds = std::env::var("HYDROLYSIS_WAYLAND_SECONDS")
        .map(|value| {
            value
                .parse::<u64>()
                .expect("HYDROLYSIS_WAYLAND_SECONDS must be an unsigned integer")
        })
        .unwrap_or(15);
    Duration::from_secs(seconds)
}

fn main() {
    thread::spawn(|| {
        thread::sleep(showcase_lifetime());
        std::process::exit(0);
    });

    run(app());
}
