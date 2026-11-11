//! The live operand cell the look realization records against: a signal
//! that carries the animation each change was set under, so the engine
//! animates the recorded command itself.

use std::cell::RefCell;
use std::rc::Rc;

use cherenkov_record::{KeyframeAnimation, Live};
use nami::Signal;
use nami::watcher::{Context, Metadata, WatcherManager, WatcherManagerGuard};
use waterui_core::animation::Animation;
use waterui_graphics::animation::engine_animation;

/// A value a recording reads live.
pub struct Operand<T: 'static>(Rc<OperandCell<T>>);

struct OperandCell<T: 'static> {
    value: RefCell<T>,
    watchers: WatcherManager<T>,
}

impl<T> Clone for Operand<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Clone + PartialEq + 'static> Operand<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(Rc::new(OperandCell {
            value: RefCell::new(value),
            watchers: WatcherManager::new(),
        }))
    }

    /// Moves the operand to `value`, animated with `animation` or snapped
    /// when `None`. An unchanged value notifies nothing.
    pub(crate) fn set(&self, value: T, animation: Option<&Animation>) {
        if *self.0.value.borrow() == value {
            return;
        }
        *self.0.value.borrow_mut() = value.clone();
        let metadata = animation.map_or_else(Metadata::new, |animation| {
            Metadata::new().with(engine_animation(animation))
        });
        self.0.watchers.notify_owned(Context::new(value, metadata));
    }

    /// Runs `track` along the operand: the engine samples its keyframes on
    /// its own clock until the operand changes again. The operand keeps its
    /// value; the track decides what is drawn while it runs.
    pub(crate) fn run(&self, track: &KeyframeAnimation) {
        let value = self.0.value.borrow().clone();
        let metadata = Metadata::new().with(track.clone());
        self.0.watchers.notify_owned(Context::new(value, metadata));
    }

    pub(crate) fn live(&self) -> Live<T> {
        Live::from(self.clone())
    }

    /// Whether `other` is this operand: the same cell, whatever its value.
    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: Clone + PartialEq + 'static> Signal for Operand<T> {
    type Output = T;
    type Guard = WatcherManagerGuard<T>;

    fn snapshot(&self) -> T {
        self.0.value.borrow().clone()
    }

    fn watch(&self, watcher: impl Fn(Context<T>) + 'static) -> Self::Guard {
        self.0.watchers.register_as_guard(watcher)
    }
}
