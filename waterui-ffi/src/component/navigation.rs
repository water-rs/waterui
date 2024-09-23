use alloc::boxed::Box;
use waterui::{
    navigation::{Bar, NavigationLink, NavigationView},
    view::ConfigurableView,
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
    Box<dyn Fn(Environment) -> NavigationView>
);

#[no_mangle]
unsafe extern "C" fn waterui_navigation_view_builder_call(
    content: *const waterui_navigation_view_builder,
    env: *mut waterui_env,
) -> waterui_navigation_view {
    ((*content).0)(env.into_rust()).into_ffi()
}

impl IntoFFI for NavigationLink {
    type FFI = waterui_navigation_link;
    fn into_ffi(self) -> Self::FFI {
        waterui_navigation_link {
            label: self.label.into_ffi(),
            content: self.view.into_ffi(),
        }
    }
}

#[repr(C)]
pub struct waterui_bar {
    title: waterui_text,
    hidden: *mut waterui_computed_bool,
}

impl IntoFFI for NavigationView {
    type FFI = waterui_navigation_view;
    fn into_ffi(self) -> Self::FFI {
        waterui_navigation_view {
            bar: self.bar.into_ffi(),
            content: self.content.into_ffi(),
        }
    }
}

impl IntoFFI for Bar {
    type FFI = waterui_bar;
    fn into_ffi(self) -> Self::FFI {
        waterui_bar {
            title: self.title.config().into_ffi(),
            hidden: self.hidden.into_ffi(),
        }
    }
}

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
