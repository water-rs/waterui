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
    task::{Poll, Waker},
};
use std::rc::Rc;

use waterui_core::{Computed, configurable};

use crate::Media;

/// Configuration for the `MediaPicker` component.
#[derive(Debug)]
pub struct MediaPickerConfig {
    /// The items selected in the picker.
    pub selection: Computed<Selected>,
    /// A filter to apply to media selection.
    pub filter: Computed<MediaFilter>,
}

configurable!(
    #[doc = "A media picker view that lets users select photos, videos, or live media."]
    MediaPicker,
    MediaPickerConfig
);

/// Represents a selected media item by its unique identifier.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Selected(u32);

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
/// Represents filters that can be applied to media selection.
pub enum MediaFilter {
    /// Filter for live photos.
    LivePhoto,
    /// Filter for videos.
    Video,
    /// Filter for images.
    Image,
    /// Filter for all of the specified filters.
    All(Vec<MediaFilter>),
    /// Filter for none of the specified filters.
    Not(Vec<MediaFilter>),
    /// Filter for any of the specified filters.
    Any(Vec<MediaFilter>),
}

impl Selected {
    /// Load the selected media item.
    #[allow(clippy::unused_async)]
    pub async fn load(self) -> Media {
        todo!()
    }
}

#[allow(dead_code)]
struct WithContinuationFuture<F, T> {
    f: F,
    state: SharedContinuationState<T>,
    _marker: PhantomData<T>,
}

/// A future that allows continuation with a value.
#[derive(Debug)]
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
    /// Completes the continuation with the provided value, waking the waiting task.
    ///
    /// # Panics
    ///
    /// Panics if there is no waker set (i.e., if `finish` is called before the future is polled).
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
        Self {
            f,
            state: Rc::new(RefCell::new(ContinuationState {
                value: None,
                waker: None,
            })),
            _marker: PhantomData,
        }
    }
}

/// Creates a new future that allows continuation with a value.
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
        let state = &mut *state;
        if let Some(value) = state.value.take() {
            return Poll::Ready(value);
        }

        state.waker.get_or_insert_with(|| cx.waker().to_owned());

        Poll::Pending
    }
}
