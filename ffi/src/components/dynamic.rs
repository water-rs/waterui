use waterui::component::Dynamic;

use crate::{IntoRust, WuiAnyView, reactive::WuiWatcher};

ffi_type!(WuiDynamic, Dynamic, waterui_drop_dynamic);

ffi_view!(
    Dynamic,
    *mut WuiDynamic,
    waterui_dynamic_id,
    waterui_force_as_dynamic
);

#[unsafe(no_mangle)]
unsafe extern "C" fn waterui_dynamic_connect(
    dynamic: *mut WuiDynamic,
    watcher: WuiWatcher<*mut WuiAnyView>,
) {
    unsafe {
        (dynamic).into_rust().connect(move |ctx| {
            let metadata = ctx.metadata().clone();
            let value = ctx.into_value();
            watcher.call(value, metadata);
        });
    }
}
