use core::{
    cell::{Ref, RefCell, RefMut},
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};
use std::thread::{ThreadId, current};
pub struct LocalValue<T> {
    created: ThreadId,
    value: ManuallyDrop<T>,
}

impl<T> LocalValue<T> {
    pub fn into_inner(self) -> T {
        assert!(
            self.on_local(),
            "Attempted to get a LocalValue on a different thread"
        );
        let mut this = ManuallyDrop::new(self);
        unsafe { ManuallyDrop::take(&mut this.value) }
    }
    pub fn on_local(&self) -> bool {
        self.created == current().id()
    }
}

impl<T> LocalValue<T> {
    pub fn new(value: T) -> Self {
        LocalValue {
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
        self.value.deref()
    }
}

impl<T> DerefMut for LocalValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        assert!(
            self.on_local(),
            "Attempted to mutate a LocalValue on a different thread"
        );
        self.value.deref_mut()
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
        LocalValue::new(value)
    }
}

unsafe impl<T> Send for LocalValue<T> {}
unsafe impl<T> Sync for LocalValue<T> {}

pub struct OnceValue<T>(LocalValue<RefCell<Option<T>>>);

impl<T> OnceValue<T> {
    pub fn new(value: T) -> Self {
        OnceValue(LocalValue::new(RefCell::new(Some(value))))
    }
    pub fn get(&self) -> Ref<T> {
        Ref::map(self.0.borrow(), |v| v.as_ref().unwrap())
    }
    pub fn get_mut(&self) -> RefMut<T> {
        RefMut::map(self.0.borrow_mut(), |v| v.as_mut().unwrap())
    }

    pub fn take(&self) -> T {
        self.0.borrow_mut().take().unwrap()
    }

    pub fn into_inner(self) -> T {
        self.0
            .into_inner()
            .into_inner()
            .expect("Once value has already been taken")
    }
}

impl<T> From<T> for OnceValue<T> {
    fn from(value: T) -> Self {
        OnceValue::new(value)
    }
}
