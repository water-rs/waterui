use waterui::{
    AnyView,
    views::{AnyViews, Views},
};

use crate::{
    IntoFFI, WuiAnyView, array::WuiArray, ffi_computed, id::WuiId, reactive::WuiWatcherGuard,
    reactive::WuiWatcherMetadata,
};
use alloc::{boxed::Box, vec::Vec};
use nami::Signal;
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
            .unwrap_unchecked()
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
    unsafe { (&*anyviews).len().get() }
}

fn normalize_range(start: usize, end: usize) -> (usize, usize) {
    if start <= end {
        (start, end)
    } else {
        (end, end)
    }
}

fn collect_ids_in_range(anyviews: &WuiAnyViews, start: usize, end: usize) -> Vec<WuiId> {
    let len = anyviews.len().get();
    let (start, end) = normalize_range(start.min(len), end.min(len));

    (start..end)
        .filter_map(|index| anyviews.get_id(index))
        .map(SelfId::into_inner)
        .map(IntoFFI::into_ffi)
        .collect()
}

/// Gets the view IDs in `[start, end)` range.
///
/// # Safety
/// The caller must ensure that `anyviews` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_get_ids_in_range(
    anyviews: *const WuiAnyViews,
    start: usize,
    end: usize,
) -> WuiArray<WuiId> {
    unsafe { WuiArray::new(collect_ids_in_range(&*anyviews, start, end)) }
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
    unsafe { waterui_anyviews_watch_range(anyviews, 0, usize::MAX, data, call, drop) }
}

/// Watches for changes in a views collection within `[start, end)` range.
///
/// The callback receives the current list of view IDs in the watched range.
///
/// # Safety
/// - `anyviews` must be a valid pointer.
/// - `data`, `call`, and `drop` must form a valid callback triplet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_watch_range(
    anyviews: *const WuiAnyViews,
    start: usize,
    end: usize,
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
        let anyviews = &*anyviews;
        let (start, end) = normalize_range(start, end);
        let guard = anyviews.watch(start..end, move |ctx| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::ops::RangeBounds;
    use nami::Computed;
    use nami::watcher::Context;
    use waterui_core::id::SelfId;
    use waterui_core::views::Views;

    struct TestViews {
        len: usize,
    }

    impl Views for TestViews {
        type Id = SelfId<usize>;
        type Guard = ();
        type View = AnyView;

        fn get_id(&self, index: usize) -> Option<Self::Id> {
            (index < self.len).then_some(SelfId::new(index))
        }

        fn len(&self) -> Computed<usize> {
            Computed::constant(self.len)
        }

        fn watch(
            &self,
            _range: impl RangeBounds<usize>,
            _watcher: impl for<'a> Fn(Context<&'a [Self::Id]>) + 'static,
        ) -> Self::Guard {
        }

        fn get_view(&self, index: usize) -> Option<Self::View> {
            (index < self.len).then_some(AnyView::new(()))
        }
    }

    fn make_views(count: usize) -> WuiAnyViews {
        WuiAnyViews(AnyViews::new(TestViews { len: count }))
    }

    #[test]
    fn len_uses_reactive_len_signal_value() {
        let views = make_views(5);
        let len = unsafe { waterui_anyviews_len(&views as *const WuiAnyViews) };
        assert_eq!(len, 5);
    }

    #[test]
    fn collect_ids_in_range_clamps_to_bounds() {
        let views = make_views(4);
        let all = collect_ids_in_range(&views, 0, usize::MAX);
        let tail = collect_ids_in_range(&views, 2, 99);

        assert_eq!(all.len(), 4);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].inner, all[2].inner);
        assert_eq!(tail[1].inner, all[3].inner);
    }

    #[test]
    fn collect_ids_in_range_handles_reversed_range_as_empty() {
        let views = make_views(6);
        let ids = collect_ids_in_range(&views, 5, 2);
        assert!(ids.is_empty());
    }
}
