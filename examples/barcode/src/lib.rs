use waterui::{Environment, app::App, prelude::*};
use waterui_barcode::Barcode;

fn main() -> impl View {
    vstack((
        text("Scan me!").title(),
        Barcode::new("https://waterui.dev").size(300.0, 300.0),
    ))
    .spacing(20.0)
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

