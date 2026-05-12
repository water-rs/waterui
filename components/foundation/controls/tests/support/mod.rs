use waterui::graphics::color::Srgb;
use waterui::prelude::*;
use waterui_testing::{MountedApp, UiTest};

pub const VIEWPORT_WIDTH: u32 = 320;
pub const VIEWPORT_HEIGHT: u32 = 240;

pub fn mount_view<V, F>(build: F) -> MountedApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    let mut env = Environment::new();
    hydrolysis_m3::install(&mut env);

    UiTest::new()
        .environment(env)
        .viewport(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
        .mount(build)
}

pub fn control_shell<V: View>(content: V) -> impl View {
    vstack((content,))
        .spacing(12.0)
        .padding_with(16.0)
        .background(Srgb::BLACK)
}
