use waterui::{
    AnyView,
    views::{AnyViews, Views},
};

use crate::{
    IntoFFI, WuiAnyView, array::WuiArray, id::WuiId, reactive::WuiWatcherGuard,
    reactive::WuiWatcherMetadata,
};
use alloc::{boxed::Box, vec::Vec};
use core::hash::Hash;
use nami::watcher::WatcherGuard;
use nami::{Signal, SignalExt};
use waterui_core::id::SelfId;

opaque!(WuiAnyViews, AnyViews<AnyView>, anyviews, any());

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

fn collect_ids_in_range(anyviews: &WuiAnyViews, start: usize, end: usize) -> Vec<WuiId> {
    (start..end)
        .map(|index| {
            anyviews
                .get_id(index)
                .expect("native requested an out-of-bounds view collection id")
        })
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
    struct ForeignWatcher {
        data: *mut (),
        call: unsafe extern "C" fn(*mut (), WuiArray<WuiId>, *mut WuiWatcherMetadata),
        drop: unsafe extern "C" fn(*mut ()),
    }

    impl Drop for ForeignWatcher {
        fn drop(&mut self) {
            unsafe { (self.drop)(self.data) }
        }
    }

    struct Guard {
        inner: Option<waterui::reactive::watcher::BoxWatcherGuard>,
        _watcher: alloc::rc::Rc<ForeignWatcher>,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            core::mem::drop(self.inner.take());
        }
    }

    impl WatcherGuard for Guard {}

    unsafe {
        let anyviews = &*anyviews;
        let watcher = alloc::rc::Rc::new(ForeignWatcher { data, call, drop });
        let callback_watcher = alloc::rc::Rc::clone(&watcher);
        let guard = anyviews.watch(start..end, move |ctx| {
            let watcher = alloc::rc::Rc::clone(&callback_watcher);
            let metadata = ctx.metadata().clone();
            let ids: Vec<WuiId> = ctx
                .into_value()
                .iter()
                .copied()
                .map(SelfId::into_inner)
                .map(IntoFFI::into_ffi)
                .collect();
            (watcher.call)(watcher.data, WuiArray::new(ids), metadata.into_ffi());
        });

        let boxed: waterui::reactive::watcher::BoxWatcherGuard = Box::new(Guard {
            inner: Some(guard),
            _watcher: watcher,
        });

        IntoFFI::into_ffi(boxed)
    }
}

struct SignalVecViews<S, T, Id, IdAt, BuildView> {
    source: S,
    id_at: IdAt,
    build_view: BuildView,
    _marker: core::marker::PhantomData<fn(T) -> Id>,
}

impl<S, T, Id, IdAt, BuildView> Views for SignalVecViews<S, T, Id, IdAt, BuildView>
where
    S: Signal<Output = Vec<T>> + Clone,
    T: 'static,
    Id: Hash + Ord + Clone + 'static,
    IdAt: Fn(&[T], usize) -> Id + Clone + 'static,
    BuildView: Fn(T) -> AnyView + Clone + 'static,
{
    type Id = Id;
    type Guard = S::Guard;
    type View = AnyView;

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        let items = self.source.get();
        (index < items.len()).then(|| (self.id_at)(&items, index))
    }

    fn len(&self) -> nami::Computed<usize> {
        self.source.clone().map(|items| items.len()).computed()
    }

    fn watch(
        &self,
        range: impl core::ops::RangeBounds<usize>,
        watcher: impl for<'a> Fn(nami::watcher::Context<&'a [Self::Id]>) + 'static,
    ) -> Self::Guard {
        let start = match range.start_bound() {
            core::ops::Bound::Included(index) => *index,
            core::ops::Bound::Excluded(index) => index + 1,
            core::ops::Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            core::ops::Bound::Included(index) => index + 1,
            core::ops::Bound::Excluded(index) => *index,
            core::ops::Bound::Unbounded => usize::MAX,
        };
        let id_at = self.id_at.clone();
        self.source.watch(move |ctx| {
            let ctx = ctx.map(|items| {
                let end = end.min(items.len());
                (start..end)
                    .map(|index| id_at(&items, index))
                    .collect::<Vec<_>>()
            });
            watcher(ctx.as_deref());
        })
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        self.source
            .get()
            .into_iter()
            .nth(index)
            .map(&self.build_view)
    }
}

/// Erases a reactive vector as the framework's existing identity-aware view collection.
///
/// Native backends use the ordinary `waterui_anyviews_*` ABI to reconcile semantic
/// collection membership and then force each returned raw item view to its descriptor.
pub(crate) fn signal_vec_views<S, T, Id, IdAt, BuildView>(
    source: S,
    id_at: IdAt,
    build_view: BuildView,
) -> *mut WuiAnyViews
where
    S: Signal<Output = Vec<T>> + Clone,
    T: 'static,
    Id: Hash + Ord + Clone + 'static,
    IdAt: Fn(&[T], usize) -> Id + Clone + 'static,
    BuildView: Fn(T) -> AnyView + Clone + 'static,
{
    AnyViews::new(SignalVecViews {
        source,
        id_at,
        build_view,
        _marker: core::marker::PhantomData,
    })
    .into_ffi()
}
