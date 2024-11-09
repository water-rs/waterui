use alloc::boxed::Box;
use futures_lite::{stream::BoxedLocal as BoxStream, StreamExt};
use waterui::{AnyView, LocalTask};
use waterui_lazy::{AnyLazyList, LazyList};

use crate::{closure::waterui_fnonce, waterui_anyview, IntoFFI};

ffi_type!(
    waterui_lazy_view_list,
    AnyLazyList<AnyView>,
    waterui_drop_lazy_view_list
);

#[no_mangle]
unsafe extern "C" fn waterui_lazy_view_list_get(
    list: *const waterui_lazy_view_list,
    index: usize,
    callback: waterui_fnonce<*mut waterui_anyview>,
) {
    LocalTask::on_main(async move {
        let value = (*list).get(index).await.into_ffi();
        callback.call(value);
    });
}

#[no_mangle]
unsafe extern "C" fn waterui_lazy_list_len(list: *const waterui_lazy_view_list) -> i32 {
    (*list).len().map(|v| v as i32).unwrap_or(-1)
}

ffi_type!(
    waterui_anyview_iter,
    BoxStream<AnyView>,
    waterui_drop_anyview_iter
);

#[no_mangle]
unsafe extern "C" fn waterui_anyview_iter_next(
    iter: *mut waterui_anyview_iter,
    callback: waterui_fnonce<*mut waterui_anyview>,
) {
    LocalTask::on_main(async move {
        let value = (*iter).next().await;
        callback.call(value.into_ffi())
    });
}

#[no_mangle]
unsafe extern "C" fn waterui_lazy_list_iter(
    list: *const waterui_lazy_view_list,
) -> *mut waterui_anyview_iter {
    let boxed: BoxStream<AnyView> = Box::pin((*list).iter());
    boxed.into_ffi()
}

#[no_mangle]
unsafe extern "C" fn waterui_lazy_list_rev_iter(
    list: *const waterui_lazy_view_list,
) -> *mut waterui_anyview_iter {
    let boxed: BoxStream<AnyView> = Box::pin((*list).rev_iter());
    boxed.into_ffi()
}
