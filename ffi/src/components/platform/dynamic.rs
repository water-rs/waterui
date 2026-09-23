use alloc::boxed::Box;
use alloc::rc::Rc;
use nami::watcher::Context;
use waterui::{AnyView, component::Dynamic};

use crate::reactive::WuiWatcher;

opaque!(WuiDynamic, Dynamic);

ffi_view!(Dynamic, *mut WuiDynamic, dynamic);

/// Connects a watcher to a dynamic view.
/// # Safety
/// - The dynamic pointer must be valid.
/// - The watcher pointer will be consumed and freed when the Dynamic is dropped.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_dynamic_connect(
    dynamic: *mut WuiDynamic,
    watcher: *mut WuiWatcher<AnyView>,
) {
    unsafe {
        let watcher = (*Box::from_raw(watcher)).into_inner();
        // IMPORTANT: do NOT consume/free `dynamic` here. The native side still owns the
        // opaque pointer and will call `waterui_drop_dynamic` when the view is disposed.
        //
        // We clone the inner Dynamic (cheap Rc clone) and connect the receiver on the
        // shared state so subsequent updates flow to native callbacks.
        let dynamic = &mut *dynamic;
        dynamic.0.clone().connect(move |ctx| {
            let metadata = ctx.metadata().clone();
            let value = ctx.into_value();
            let callback = Rc::clone(&watcher);
            callback(Context::new(value, metadata));
        });
    }
}
