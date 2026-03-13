//! Renderer-provided persistent local state slots.
//!
//! Renderers can install a [`LocalStateStore`] together with the current
//! [`LocalStateScope`] into the [`Environment`](crate::Environment). Composite
//! views can then keep local bindings or shared state alive across rebuilds,
//! while remaining stable across repeated `body()` expansion during layout.

extern crate alloc;

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::{Any, TypeId};
use core::cell::{Cell, RefCell};

use crate::Binding;

type SlotFactory = dyn Fn(u64, usize, TypeId, Box<dyn Fn() -> Rc<dyn Any>>) -> Rc<dyn Any>;

#[inline]
const fn mix_scope(parent: u64, child: usize) -> u64 {
    parent.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(7)
        ^ ((child as u64).wrapping_add(0x517C_C1B7_2722_0A95))
}

/// Current body scope used to key local state lookups.
#[derive(Clone)]
pub struct LocalStateScope {
    path: u64,
    slot_cursor: Rc<Cell<usize>>,
}

impl core::fmt::Debug for LocalStateScope {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocalStateScope")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl LocalStateScope {
    /// Creates the root scope for a rebuild traversal.
    #[must_use]
    pub fn root() -> Self {
        Self {
            path: 0x6A09_E667_F3BC_C909,
            slot_cursor: Rc::new(Cell::new(0)),
        }
    }

    /// Resets the local slot cursor for a fresh body evaluation at the same path.
    #[must_use]
    pub fn reset(&self) -> Self {
        Self {
            path: self.path,
            slot_cursor: Rc::new(Cell::new(0)),
        }
    }

    /// Creates a child scope for a deterministic child index.
    #[must_use]
    pub fn child(&self, index: usize) -> Self {
        Self {
            path: mix_scope(self.path, index),
            slot_cursor: Rc::new(Cell::new(0)),
        }
    }

    fn next_slot_index(&self) -> usize {
        let index = self.slot_cursor.get();
        self.slot_cursor.set(
            index
                .checked_add(1)
                .expect("LocalStateScope slot cursor overflow"),
        );
        index
    }
}

/// Renderer-provided local state storage keyed by scope path and local slot order.
#[derive(Clone)]
pub struct LocalStateStore {
    bind_slot: Rc<SlotFactory>,
}

impl core::fmt::Debug for LocalStateStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocalStateStore").finish_non_exhaustive()
    }
}

impl LocalStateStore {
    /// Creates a local state store from a renderer-owned slot allocator.
    #[must_use]
    pub fn new(
        bind_slot: impl Fn(u64, usize, TypeId, Box<dyn Fn() -> Rc<dyn Any>>) -> Rc<dyn Any> + 'static,
    ) -> Self {
        Self {
            bind_slot: Rc::new(bind_slot),
        }
    }

    /// Returns the current slot value or initializes it on first use.
    #[must_use]
    pub fn get_or_init<T: 'static>(
        &self,
        scope: &LocalStateScope,
        init: impl FnOnce() -> T + 'static,
    ) -> Rc<T> {
        let init = RefCell::new(Some(init));
        let value = (self.bind_slot)(
            scope.path,
            scope.next_slot_index(),
            TypeId::of::<T>(),
            Box::new(move || {
                let init = init.borrow_mut().take().unwrap_or_else(|| {
                    panic!("LocalStateStore initializer was invoked more than once")
                });
                Rc::new(init()) as Rc<dyn Any>
            }),
        );
        value.downcast::<T>().unwrap_or_else(|_| {
            panic!(
                "LocalStateStore slot type mismatch for {}",
                core::any::type_name::<T>()
            )
        })
    }

    /// Returns a persistent local binding initialized on first use.
    #[must_use]
    pub fn binding<T: Clone + 'static>(
        &self,
        scope: &LocalStateScope,
        init: impl FnOnce() -> T + 'static,
    ) -> Binding<T> {
        self.get_or_init(scope, move || Binding::container(init()))
            .as_ref()
            .clone()
    }
}
