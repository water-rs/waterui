//! A signal that keeps something alive for as long as it is observed.
//!
//! A materialized cell owns a JavaScript subscription and a watch guard, and
//! must live exactly as long as the value handed to the view tree: while a
//! native view holds the `Binding<T>`, the subscription feeding it has to
//! stay; when the last reference goes, the subscription is disposed. Tying the
//! cell to the signal itself expresses that without a scope to register in —
//! the lifetime *is* the value's.

use std::any::Any;
use std::rc::Rc;

use nami::watcher::{BoxWatcherGuard, Context};
use nami::{Binding, CustomBinding, Signal, SignalIdentity};

/// A binding that carries a passenger, dropped when the last clone is.
pub struct Tethered<T: 'static> {
    binding: Binding<T>,
    tether: Rc<dyn Any>,
}

impl<T: 'static> Tethered<T> {
    /// Ties `tether` to the lifetime of `binding`.
    pub(crate) const fn new(binding: Binding<T>, tether: Rc<dyn Any>) -> Self {
        Self { binding, tether }
    }
}

impl<T: 'static> Clone for Tethered<T> {
    fn clone(&self) -> Self {
        Self {
            binding: self.binding.clone(),
            tether: Rc::clone(&self.tether),
        }
    }
}

impl<T: 'static> Signal for Tethered<T> {
    type Output = T;
    type Guard = BoxWatcherGuard;

    fn get(&self) -> Self::Output {
        self.binding.get()
    }

    fn identity(&self) -> Option<SignalIdentity> {
        self.binding.identity()
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        self.binding.watch(watcher)
    }
}

impl<T: 'static> CustomBinding for Tethered<T> {
    fn set(&self, value: T) {
        self.binding.set(value);
    }
}
