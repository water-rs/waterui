use waterui::{
    AnyView,
    views::{AnyViews, AnyViewsSnapshot, ViewSnapshot, Views},
};

use crate::{
    IntoFFI, WuiAnyView, array::WuiArray, id::WuiId, reactive::WuiWatcherGuard,
    reactive::WuiWatcherMetadata,
};
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::hash::Hash;
use core::marker::PhantomData;
use core::ops::Range;
use nami::collection::CollectionChange;
use nami::watcher::WatcherGuard;
use nami::{Signal, SignalExt};
use waterui_core::id::SelfId;
use waterui_core::views::resolve_range;

opaque!(WuiAnyViews, AnyViews<AnyView>, anyviews, any());

opaque!(
    WuiViewSnapshot,
    AnyViewsSnapshot<AnyView>,
    view_snapshot,
    any()
);

/// Captures the collection's current state as an immutable, owning snapshot.
///
/// # Safety
/// The caller must ensure that `anyviews` is a valid pointer. The returned
/// handle is owned by the caller and must be released with
/// `waterui_drop_view_snapshot` (or cloned with `waterui_view_snapshot_clone`
/// when several owners need it).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_anyviews_snapshot(
    anyviews: *const WuiAnyViews,
) -> *mut WuiViewSnapshot {
    // SAFETY: the caller contract requires `anyviews` to be a valid handle alive
    // for this call; it is only borrowed.
    unsafe { (&*anyviews).snapshot().into_ffi() }
}

/// Clones a view snapshot, returning an independently owned handle.
///
/// # Safety
/// The caller must ensure that `snapshot` is a valid pointer. The returned
/// handle is owned by the caller and must be released with
/// `waterui_drop_view_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_snapshot_clone(
    snapshot: *const WuiViewSnapshot,
) -> *mut WuiViewSnapshot {
    // SAFETY: the caller contract requires `snapshot` to be a valid handle alive
    // for this call; it is only borrowed.
    unsafe { (&*snapshot).0.clone().into_ffi() }
}

/// Gets the number of positions the snapshot captured.
///
/// # Safety
/// The caller must ensure that `snapshot` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_snapshot_len(snapshot: *const WuiViewSnapshot) -> usize {
    // SAFETY: the caller contract requires `snapshot` to be a valid handle alive
    // for this call; it is only borrowed.
    unsafe { (&*snapshot).len() }
}

/// Gets the first collection-wide index the snapshot captured.
///
/// # Safety
/// The caller must ensure that `snapshot` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_snapshot_range_start(
    snapshot: *const WuiViewSnapshot,
) -> usize {
    // SAFETY: the caller contract requires `snapshot` to be a valid handle alive
    // for this call; it is only borrowed.
    unsafe { (&*snapshot).range().start }
}

/// Gets the collection-wide index one past the last position the snapshot captured.
///
/// # Safety
/// The caller must ensure that `snapshot` is a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_snapshot_range_end(
    snapshot: *const WuiViewSnapshot,
) -> usize {
    // SAFETY: the caller contract requires `snapshot` to be a valid handle alive
    // for this call; it is only borrowed.
    unsafe { (&*snapshot).range().end }
}

/// Gets a view at the specified collection-wide index from the snapshot.
///
/// # Safety
/// The caller must ensure that `snapshot` is a valid pointer. Indices outside
/// the snapshot's range return null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_snapshot_get_view(
    snapshot: *const WuiViewSnapshot,
    index: usize,
) -> *mut WuiAnyView {
    // SAFETY: the caller contract requires `snapshot` to be a valid handle alive
    // for this call; it is only borrowed.
    unsafe { (&*snapshot).get_view(index).into_ffi() }
}

fn collect_ids_in_range(snapshot: &WuiViewSnapshot, start: usize, end: usize) -> Vec<WuiId> {
    (start..end)
        .map(|index| {
            snapshot
                .get_id(index)
                .expect("native requested an out-of-snapshot view collection id")
        })
        .map(SelfId::into_inner)
        .map(IntoFFI::into_ffi)
        .collect()
}

/// Gets the view IDs in `[start, end)` collection-wide range from the snapshot.
///
/// # Safety
/// The caller must ensure that `snapshot` is a valid pointer and that
/// `[start, end)` lies inside the snapshot's range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_view_snapshot_get_ids_in_range(
    snapshot: *const WuiViewSnapshot,
    start: usize,
    end: usize,
) -> WuiArray<WuiId> {
    // SAFETY: the caller contract requires `snapshot` to be a valid handle alive
    // for this call; it is only borrowed while the ids are collected.
    unsafe { WuiArray::new(collect_ids_in_range(&*snapshot, start, end)) }
}

/// Watches for changes in a views collection within `[start, end)` range.
///
/// Each callback receives an owning snapshot handle covering the requested
/// range resolved against that notification's captured data, plus the watcher
/// metadata. The callee owns the snapshot handle and must release it with
/// `waterui_drop_view_snapshot` once the reconciliation it supplies is done.
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
    call: unsafe extern "C" fn(*mut (), *mut WuiViewSnapshot, *mut WuiWatcherMetadata),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut WuiWatcherGuard {
    struct ForeignWatcher {
        data: *mut (),
        call: unsafe extern "C" fn(*mut (), *mut WuiViewSnapshot, *mut WuiWatcherMetadata),
        drop: unsafe extern "C" fn(*mut ()),
    }

    impl Drop for ForeignWatcher {
        fn drop(&mut self) {
            // SAFETY: `drop` and `data` are one registration, and `Drop` runs once.
            unsafe { (self.drop)(self.data) }
        }
    }

    struct Guard {
        inner: Option<waterui::reactive::watcher::BoxWatcherGuard>,
        _watcher: Rc<ForeignWatcher>,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            core::mem::drop(self.inner.take());
        }
    }

    impl WatcherGuard for Guard {}

    // SAFETY: the caller contract requires `anyviews` to be a valid handle alive for
    // this call, and `data`/`call`/`drop` to be one registration from the backend.
    unsafe {
        let anyviews = &*anyviews;
        let watcher = Rc::new(ForeignWatcher { data, call, drop });
        let callback_watcher = Rc::clone(&watcher);
        let guard = anyviews.watch(start..end, move |ctx, change| {
            let watcher = Rc::clone(&callback_watcher);
            // The metadata handed to the backend always carries the
            // authoritative `CollectionChange` — including the synthetic one a
            // source produced when its notification arrived without it.
            let metadata = ctx.metadata().clone().with(change);
            let snapshot = ctx.into_value();
            (watcher.call)(watcher.data, snapshot.into_ffi(), metadata.into_ffi());
        });

        let boxed: waterui::reactive::watcher::BoxWatcherGuard = Box::new(Guard {
            inner: Some(guard),
            _watcher: watcher,
        });

        IntoFFI::into_ffi(boxed)
    }
}

/// The immutable snapshot a [`SignalVecViews`] captures: the notification's
/// own `Vec<T>` shared behind `Rc` plus the id and view factories, so a
/// retained snapshot reads the exact data the signal emitted and stays lazy.
struct SignalVecSnapshot<T, Id, IdAt, BuildView> {
    items: Rc<Vec<T>>,
    range: Range<usize>,
    id_at: Rc<IdAt>,
    build_view: Rc<BuildView>,
    _marker: PhantomData<fn() -> Id>,
}

impl<T, Id, IdAt, BuildView> Clone for SignalVecSnapshot<T, Id, IdAt, BuildView> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            range: self.range.clone(),
            id_at: self.id_at.clone(),
            build_view: self.build_view.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T, Id, IdAt, BuildView> core::fmt::Debug for SignalVecSnapshot<T, Id, IdAt, BuildView> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(core::any::type_name::<Self>())
    }
}

impl<T, Id, IdAt, BuildView> ViewSnapshot for SignalVecSnapshot<T, Id, IdAt, BuildView>
where
    T: Clone + 'static,
    Id: Hash + Ord + Clone + 'static,
    IdAt: Fn(&[T], usize) -> Id + 'static,
    BuildView: Fn(T) -> AnyView + 'static,
{
    type Id = Id;
    type View = AnyView;

    fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    fn get_id(&self, index: usize) -> Option<Self::Id> {
        self.range
            .contains(&index)
            .then(|| (self.id_at)(&self.items, index))
    }

    fn get_view(&self, index: usize) -> Option<Self::View> {
        if self.range.contains(&index) {
            self.items
                .get(index)
                .map(|item| (self.build_view)(item.clone()))
        } else {
            None
        }
    }
}

struct SignalVecViews<S, T, Id, IdAt, BuildView> {
    source: S,
    id_at: Rc<IdAt>,
    build_view: Rc<BuildView>,
    _marker: PhantomData<fn(T) -> Id>,
}

impl<S, T, Id, IdAt, BuildView> Views for SignalVecViews<S, T, Id, IdAt, BuildView>
where
    S: Signal<Output = Vec<T>> + Clone,
    T: Clone + 'static,
    Id: Hash + Ord + Clone + 'static,
    IdAt: Fn(&[T], usize) -> Id + 'static,
    BuildView: Fn(T) -> AnyView + 'static,
{
    type Id = Id;
    type Guard = S::Guard;
    type View = AnyView;
    type Snapshot = SignalVecSnapshot<T, Id, IdAt, BuildView>;

    fn len(&self) -> nami::Computed<usize> {
        self.source.clone().map(|items| items.len()).computed()
    }

    fn snapshot(&self) -> Self::Snapshot {
        let items = self.source.snapshot();
        SignalVecSnapshot {
            range: 0..items.len(),
            items: Rc::new(items),
            id_at: self.id_at.clone(),
            build_view: self.build_view.clone(),
            _marker: PhantomData,
        }
    }

    fn watch(
        &self,
        range: impl core::ops::RangeBounds<usize>,
        watcher: impl Fn(nami::watcher::Context<Self::Snapshot>, nami::collection::CollectionChange)
        + 'static,
    ) -> Self::Guard {
        let bounds = (range.start_bound().cloned(), range.end_bound().cloned());
        let id_at = self.id_at.clone();
        let build_view = self.build_view.clone();
        // Every emission snapshots the exact `Vec<T>` the notification
        // carried — never `source.snapshot()`, which may already be a newer
        // value by the time this observer runs.
        self.source.watch(move |ctx| {
            let metadata = ctx.metadata().clone();
            let len = ctx.value().len();
            let change = metadata
                .try_get::<CollectionChange>()
                .unwrap_or_else(|| CollectionChange::everything(len));
            let range = resolve_range(bounds, len);
            let snapshot = ctx.map(|items| SignalVecSnapshot {
                items: Rc::new(items),
                range,
                id_at: id_at.clone(),
                build_view: build_view.clone(),
                _marker: PhantomData,
            });
            watcher(snapshot, change);
        })
    }
}

/// Erases a reactive vector as the framework's existing identity-aware view collection.
///
/// Native backends use the ordinary `waterui_view_snapshot_*` ABI to reconcile
/// semantic collection membership and then force each returned raw item view to
/// its descriptor.
pub(crate) fn signal_vec_views<S, T, Id, IdAt, BuildView>(
    source: S,
    id_at: IdAt,
    build_view: BuildView,
) -> *mut WuiAnyViews
where
    S: Signal<Output = Vec<T>> + Clone,
    T: Clone + 'static,
    Id: Hash + Ord + Clone + 'static,
    IdAt: Fn(&[T], usize) -> Id + 'static,
    BuildView: Fn(T) -> AnyView + 'static,
{
    AnyViews::new(SignalVecViews {
        source,
        id_at: Rc::new(id_at),
        build_view: Rc::new(build_view),
        _marker: PhantomData,
    })
    .into_ffi()
}
