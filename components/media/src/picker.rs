//! # Media Picker
//!
//! This module provides media selection functionality through `MediaPicker`.
//!
//! ## Platform Support
//!
//! The `MediaPicker` is only available on supported platforms. Please check the
//! documentation for your specific platform to ensure compatibility before use.
//!

use core::{
    cell::RefCell,
    marker::PhantomData,
    ops::DerefMut,
    task::{Poll, Waker},
};
use std::{ops::Deref, rc::Rc, str::from_utf8_unchecked};

use waterui_core::{
    Computed, configurable,
    ffi::{array::WuiData, closure::WuiFn},
};

use crate::Media;

#[derive(Debug)]
pub struct MediaPickerConfig {
    pub selection: Computed<Selected>,
    pub filter: Computed<MediaFilter>,
}

configurable!(MediaPicker, MediaPickerConfig);

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Selected(usize);

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum MediaFilter {
    LivePhoto,
    Video,
    Image,
    All(Vec<MediaFilter>),
    Not(Vec<MediaFilter>),
    Any(Vec<MediaFilter>),
}

impl Selected {
    pub async fn load(self) -> Media {
        with_continuation(|continuation| unsafe {
            waterui_load_media(
                self.0,
                WuiFn::from(|url: WuiData| {
                    let url = from_utf8_unchecked(url.deref());
                    //continuation.finish(Media::new(url));
                    todo!()
                }),
            );
            // Media::new(url)
        })
        .await
    }
}

unsafe extern "C" {
    unsafe fn waterui_load_media(id: usize, callback: WuiFn<WuiData>); // return url
}

struct WithContinuationFuture<F, T> {
    f: F,
    state: SharedContinuationState<T>,
    _marker: PhantomData<T>,
}

pub struct Continuation<T> {
    state: SharedContinuationState<T>,
}

type SharedContinuationState<T> = Rc<RefCell<ContinuationState<T>>>;

#[derive(Debug)]
struct ContinuationState<T> {
    value: Option<T>,
    waker: Option<Waker>,
}

impl<T> Continuation<T> {
    pub fn finish(self, value: T) {
        let mut state = self.state.borrow_mut();
        state.value = Some(value);
        state.waker.take().unwrap().wake();
    }
}

impl<F, T> WithContinuationFuture<F, T>
where
    F: FnOnce(Continuation<T>),
{
    pub fn new(f: F) -> Self {
        WithContinuationFuture {
            f,
            state: Rc::new(RefCell::new(ContinuationState {
                value: None,
                waker: None,
            })),
            _marker: PhantomData,
        }
    }
}

pub async fn with_continuation<F, T>(f: F) -> T
where
    F: FnOnce(Continuation<T>),
{
    WithContinuationFuture::new(f).await
}

impl<F, T> Future for WithContinuationFuture<F, T>
where
    F: FnOnce(Continuation<T>),
{
    type Output = T;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut state = self.state.borrow_mut();
        let state = state.deref_mut();
        if let Some(value) = state.value.take() {
            return Poll::Ready(value);
        }

        state.waker.get_or_insert_with(|| cx.waker().to_owned());

        Poll::Pending
    }
}
