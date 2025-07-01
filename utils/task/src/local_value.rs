use core::{
    cell::{Ref, RefCell, RefMut},
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};
use std::thread::{ThreadId, current};

/// A thread-local value that can only be accessed on the thread where it was created.
///
/// This type provides thread safety by ensuring that the contained value can only be
/// accessed, mutated, or dropped on the thread where it was originally created.
#[derive(Debug)]
pub struct LocalValue<T> {
    created: ThreadId,
    value: ManuallyDrop<T>,
}

impl<T> LocalValue<T> {
    /// Consumes the `LocalValue` and returns the contained value.
    ///
    /// # Panics
    ///
    /// Panics if called from a different thread than where the value was created.
    pub fn into_inner(self) -> T {
        assert!(
            self.on_local(),
            "Attempted to get a LocalValue on a different thread"
        );
        let mut this = ManuallyDrop::new(self);
        unsafe { ManuallyDrop::take(&mut this.value) }
    }

    /// Returns `true` if the current thread is the same as the thread where this value was created.
    pub fn on_local(&self) -> bool {
        self.created == current().id()
    }
}

impl<T> LocalValue<T> {
    /// Creates a new `LocalValue` containing the given value.
    ///
    /// The value can only be accessed on the current thread.
    pub fn new(value: T) -> Self {
        Self {
            created: current().id(),
            value: ManuallyDrop::new(value),
        }
    }
}

impl<T> Deref for LocalValue<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        assert!(
            self.on_local(),
            "Attempted to access a LocalValue on a different thread"
        );
        &self.value
    }
}

impl<T> DerefMut for LocalValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        assert!(
            self.on_local(),
            "Attempted to mutate a LocalValue on a different thread"
        );
        &mut self.value
    }
}

impl<T> Drop for LocalValue<T> {
    fn drop(&mut self) {
        assert!(
            self.on_local(),
            "Attempted to drop a LocalValue on a different thread"
        );
        unsafe {
            let _ = ManuallyDrop::take(&mut self.value);
        }
    }
}

impl<T> From<T> for LocalValue<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

unsafe impl<T: Send> Send for LocalValue<T> {}
unsafe impl<T: Sync> Sync for LocalValue<T> {}

/// A value that can be taken only once, providing interior mutability and thread-local safety.
#[derive(Debug)]
pub struct OnceValue<T>(LocalValue<RefCell<Option<T>>>);

impl<T> OnceValue<T> {
    /// Creates a new `OnceValue` containing the given value.
    ///
    /// The value can be taken only once using `take()` or consumed using `into_inner()`.
    pub fn new(value: T) -> Self {
        Self(LocalValue::new(RefCell::new(Some(value))))
    }

    /// Returns a reference to the contained value.
    ///
    /// # Panics
    ///
    /// Panics if the value has already been taken or if called from a different thread.
    pub fn get(&self) -> Ref<'_, T> {
        Ref::map(self.0.borrow(), |v| v.as_ref().unwrap())
    }

    /// Returns a mutable reference to the contained value.
    ///
    /// # Panics
    ///
    /// Panics if the value has already been taken or if called from a different thread.
    pub fn get_mut(&self) -> RefMut<'_, T> {
        RefMut::map(self.0.borrow_mut(), |v| v.as_mut().unwrap())
    }

    /// Takes the value out of the `OnceValue`, leaving it empty.
    ///
    /// # Panics
    ///
    /// Panics if the value has already been taken or if called from a different thread.
    pub fn take(&self) -> T {
        self.0.borrow_mut().take().unwrap()
    }

    /// Consumes the `OnceValue` and returns the contained value.
    ///
    /// # Panics
    ///
    /// Panics if the value has already been taken.
    pub fn into_inner(self) -> T {
        self.0
            .into_inner()
            .into_inner()
            .expect("Once value has already been taken")
    }
}

impl<T> From<T> for OnceValue<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
