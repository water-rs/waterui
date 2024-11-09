use core::{mem::ManuallyDrop, ptr::from_ref};

use crate::{exec_main, Task};

pub struct MainValue<T>(ManuallyDrop<T>);

struct Wrapper<T>(T);

unsafe impl<T> Send for Wrapper<T> {}

impl<T> Drop for MainValue<T> {
    fn drop(&mut self) {
        let this = unsafe { Wrapper(ManuallyDrop::take(&mut self.0)) };
        exec_main(move || {
            let _ = this.0;
        });
    }
}

impl<T: Clone + 'static> MainValue<T> {
    pub async fn clone(&self) -> Self {
        Self::new(self.handle(|v| Wrapper(v.clone())).await.0)
    }
}

unsafe impl<T> Send for MainValue<T> {}
unsafe impl<T> Sync for MainValue<T> {}

impl<T: 'static> MainValue<T> {
    pub const fn new(value: T) -> Self {
        Self(ManuallyDrop::new(value))
    }

    pub async fn handle<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&T) -> R + Send + 'static,
        R: Send + 'static,
    {
        let ptr = Wrapper(from_ref(&self.0));
        Task::on_main(async move {
            let ptr = ptr;
            let ptr = unsafe { &*(ptr.0) };
            f(ptr)
        })
        .await
    }
}
