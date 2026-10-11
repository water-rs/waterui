//! First presentation readiness for an owning application environment.

use cocoa_ui::PlatformView;
use std::{cell::Cell, rc::Rc};
use waterui_backend_core::Environment;

#[derive(Clone, Debug, Default)]
pub struct FirstPaint(Rc<Cell<bool>>);

impl FirstPaint {
    fn claim(&self) -> bool {
        !self.0.replace(true)
    }
}

pub fn mark(view: &PlatformView, env: &Environment) {
    let state = env
        .get::<FirstPaint>()
        .expect("first-paint state is installed");
    if !state.claim() {
        return;
    }
    let view = cocoa_ui::view::retain_base(view);
    executor_core::spawn_local(async move {
        cocoa_ui::view::layout_immediately(&view);
        #[cfg(feature = "gpu_surface")]
        crate::components::gpu_surface::wait_for_first_frames(&view).await;
        cocoa_ui::view::display_immediately(&view);
        cocoa_ui::core_animation::flush_transaction();
        match cocoa_ui::process::time_since_start() {
            Ok(elapsed) => cocoa_ui::log::Log::new("dev.waterui", "Startup")
                .notice(&format!("waterui_first_paint_ms={}", elapsed.as_millis())),
            Err(error) => tracing::warn!("could not measure first paint: {error}"),
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::FirstPaint;
    use waterui_backend_core::Environment;

    #[test]
    fn mounts_share_one_runtime_launch_marker() {
        let mut runtime = Environment::new();
        runtime.insert(FirstPaint::default());
        let first_mount = runtime.clone();
        let second_mount = runtime.clone();
        assert!(first_mount.get::<FirstPaint>().unwrap().claim());
        drop(first_mount);
        assert!(!second_mount.get::<FirstPaint>().unwrap().claim());
        assert!(!runtime.get::<FirstPaint>().unwrap().claim());
    }
}
