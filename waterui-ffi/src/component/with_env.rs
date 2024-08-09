use waterui::view::WithEnv;

use crate::{waterui_anyview, waterui_env, IntoFFI};

#[repr(C)]
pub struct waterui_with_env {
    view: *mut waterui_anyview,
    env: *mut waterui_env,
}

impl IntoFFI for WithEnv {
    type FFI = waterui_with_env;
    fn into_ffi(self) -> Self::FFI {
        waterui_with_env {
            view: self.view.into_ffi(),
            env: self.env.into_ffi(),
        }
    }
}

ffi_view!(
    WithEnv,
    waterui_with_env,
    waterui_view_force_as_with_env,
    waterui_view_with_env_id
);
