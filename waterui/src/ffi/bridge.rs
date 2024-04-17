use super::{Closure, Environment};

ffi_opaque!(crate::app::Bridge, Bridge, 1);

#[no_mangle]
unsafe extern "C" fn waterui_send_to_bridge(bridge: *const Bridge, f: Closure) -> i8 {
    if (*bridge).send_blocking(move || f.call()).is_ok() {
        0
    } else {
        -1
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_create_bridge(env: *mut Environment) -> Bridge {
    crate::app::Bridge::new(&mut *env).into()
}

#[no_mangle]
unsafe extern "C" fn waterui_clone_bridge(bridge: *const Bridge) -> Bridge {
    (*bridge).clone().into()
}

#[no_mangle]
unsafe extern "C" fn waterui_drop_bridge(bridge: Bridge) {
    let _ = bridge;
}
