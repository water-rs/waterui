//! The value a canvas command takes for each of its operands.

use core::fmt;

use nami::{Computed, Signal};

/// One operand of a canvas command: a value, or any signal of one.
///
/// Every command operand takes `impl Into<Operand<T>>`. A plain value is a
/// constant signal, so it converts the same way a `Binding<T>`, a
/// `Computed<T>` or a mapped signal does. A signal binds as a live operand:
/// its later changes update that one operand of the recording in place, and
/// a change made under an animation animates it.
pub struct Operand<T: 'static>(Computed<T>);

impl<T: 'static> Operand<T> {
    pub(crate) fn into_computed(self) -> Computed<T> {
        self.0
    }
}

impl<T: 'static> fmt::Debug for Operand<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Operand(..)")
    }
}

impl<T: 'static, S: Signal<Output = T>> From<S> for Operand<T> {
    fn from(signal: S) -> Self {
        Self(Computed::new(signal))
    }
}
