use alloc::boxed::Box;
use waterui::{
    component::navigation::{Bar, NavigationLink, NavigationView},
    Environment,
};

use crate::{computed::waterui_computed_bool, waterui_anyview, waterui_env, IntoFFI, IntoRust};

use super::text::waterui_text;

#[repr(C)]
pub struct waterui_navigation_view {
    bar: waterui_bar,
    content: *mut waterui_anyview,
}

#[repr(C)]
pub struct waterui_navigation_link {
    label: *mut waterui_anyview,
    content: *mut waterui_navigation_view_builder,
}

ffi_type!(
    waterui_navigation_view_builder,
    Box<dyn Fn(Environment) -> NavigationView>,
    waterui_drop_navigation_view_builder
);

#[no_mangle]
unsafe extern "C" fn waterui_navigation_view_builder_call(
    content: *const waterui_navigation_view_builder,
    env: *mut waterui_env,
) -> waterui_navigation_view {
    ((*content).0)(env.into_rust()).into_ffi()
}

into_ffi!(NavigationLink, waterui_navigation_link, label, content);

#[repr(C)]
pub struct waterui_bar {
    title: waterui_text,
    hidden: *mut waterui_computed_bool,
}

into_ffi!(NavigationView, waterui_navigation_view, bar, content);

into_ffi!(Bar, waterui_bar, title, hidden);

ffi_view!(
    NavigationView,
    waterui_navigation_view,
    waterui_view_force_as_navigation_view,
    waterui_view_navigation_view_id
);

ffi_view!(
    NavigationLink,
    waterui_navigation_link,
    waterui_view_force_as_navigation_link,
    waterui_view_navigation_link_id
);
