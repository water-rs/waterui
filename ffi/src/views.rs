use waterui::{
    AnyView,
    views::{AnyViews, Views},
};

use crate::{
    IntoFFI, WuiAnyView, array::WuiArray, ffi_computed, id::WuiId, reactive::WuiWatcherMetadata,
    reactive::WuiWatcherGuard,
};
use alloc::{boxed::Box, vec::Vec};
use nami::watcher::WatcherGuard;
use waterui_core::id::SelfId;

opaque!(WuiAnyViews, AnyViews<AnyView>, anyviews);

/// Gets the ID of a view at the specified index.
///
/// # Safety
/// The caller must ensure that `anyviews` is a valid pointer and `index` is within bounds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_get_id(
    anyviews: *const WuiAnyViews,
    index: usize,
) -> WuiId {
    unsafe {
        (&*anyviews)
            .get_id(index)
            .expect("Out of bound")
            .into_inner()
            .into_ffi()
    }
}

/// Gets a view at the specified index.
///
/// # Safety
/// The caller must ensure that `anyview` is a valid pointer and `index` is within bounds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_get_view(
    anyview: *const WuiAnyViews,
    index: usize,
) -> *mut WuiAnyView {
    unsafe { (&*anyview).get_view(index).into_ffi() }
}

/// Gets the number of views in the collection.
///
/// # Safety
/// The caller must ensure that `anyviews` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_len(anyviews: *const WuiAnyViews) -> usize {
    unsafe { (&*anyviews).len() }
}

/// Watches for changes in a views collection.
///
/// The callback receives the current list of view IDs (in order) whenever the collection changes.
///
/// # Safety
/// - `anyviews` must be a valid pointer.
/// - `data`, `call`, and `drop` must form a valid callback triplet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_watch(
    anyviews: *const WuiAnyViews,
    data: *mut (),
    call: unsafe extern "C" fn(*mut (), WuiArray<WuiId>, *mut WuiWatcherMetadata),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut WuiWatcherGuard {
    struct Guard {
        // Drop order matters: unregister watcher first, then release native data.
        inner: Option<waterui::reactive::watcher::BoxWatcherGuard>,
        data: *mut (),
        drop: unsafe extern "C" fn(*mut ()),
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            core::mem::drop(self.inner.take());
            unsafe { (self.drop)(self.data) }
        }
    }

    impl WatcherGuard for Guard {}

    unsafe {
        let guard = (&*anyviews).watch(.., move |ctx| {
            let metadata = ctx.metadata().clone();
            let ids: Vec<WuiId> = ctx
                .into_value()
                .iter()
                .copied()
                .map(SelfId::into_inner)
                .map(IntoFFI::into_ffi)
                .collect();
            call(data, WuiArray::new(ids), metadata.into_ffi());
        });

        let boxed: waterui::reactive::watcher::BoxWatcherGuard = Box::new(Guard {
            inner: Some(guard),
            data,
            drop,
        });

        IntoFFI::into_ffi(boxed)
    }
}

ffi_computed!(AnyViews<AnyView>, *mut WuiAnyViews, views);
