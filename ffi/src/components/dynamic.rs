use alloc::boxed::Box;
use waterui::{AnyView, component::Dynamic};

use crate::{IntoRust, reactive::WuiWatcher};

opaque!(WuiDynamic, Dynamic);

ffi_view!(Dynamic, *mut WuiDynamic, dynamic);

/// Connects a watcher to a dynamic view.
/// # Safety
/// - The dynamic pointer must be valid.
/// - The watcher pointer will be consumed and freed when the Dynamic is dropped.
#[unsafe(no_mangle)]
unsafe extern "C" fn waterui_dynamic_connect(
    dynamic: *mut WuiDynamic,
    watcher: *mut WuiWatcher<AnyView>,
) {
    unsafe {
        // Take ownership of the watcher - it will be dropped when Dynamic's callback drops
        let watcher = Box::from_raw(watcher);
        (dynamic).into_rust().connect(move |ctx| {
            let metadata = ctx.metadata().clone();
            let value = ctx.into_value();
            watcher.call(value, metadata);
        });
    }
}

