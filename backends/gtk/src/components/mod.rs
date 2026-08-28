//! GTK widget implementations for `WaterUI` components.

pub mod controls;
pub mod graphics;
pub mod layout;
pub mod nav;
pub mod platform;
pub mod typography;

pub use controls::{
    button, color_picker, date_picker, multi_date_picker, picker, progress, secure_field, slider,
    stepper, text_field, toggle,
};
pub use graphics::{color, gpu_surface, gradient, shape};
pub use layout::{
    container, divider, fixed_container_widget, lazy_container, list, scroll_view, spacer,
};
pub use nav::{menu, navigation, tabs};
#[cfg(feature = "webview-system")]
pub use platform::webview;
pub use platform::{dynamic, system_icon};
pub use typography::text;
