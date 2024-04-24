use core::num::NonZeroUsize;

use crate::{impl_constant, Compute, CowStr};
use alloc::string::String;

impl_constant!(CowStr, isize, bool);

impl Compute for &'static str {
    type Output = CowStr;

    fn compute(&self) -> Self::Output {
        (*self).into()
    }

    fn register_subscriber(&self, _subscriber: crate::Subscriber) -> Option<NonZeroUsize> {
        None
    }

    fn cancel_subscriber(&self, _id: NonZeroUsize) {}
    fn notify(&self) {}
}

impl Compute for String {
    type Output = CowStr;

    fn compute(&self) -> Self::Output {
        self.clone().into()
    }

    fn register_subscriber(&self, _subscriber: crate::Subscriber) -> Option<NonZeroUsize> {
        None
    }

    fn cancel_subscriber(&self, _id: NonZeroUsize) {}
    fn notify(&self) {}
}
