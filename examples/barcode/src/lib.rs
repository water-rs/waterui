use waterui::{prelude::*, app::App, Environment};
use waterui_barcode::Barcode;

fn main() -> impl View {
    vstack((
        text("Scan me!").size(32.0),
        Barcode::new("https://waterui.dev")
            .size(300.0, 300.0),
    ))
    .spacing(20.0)
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
