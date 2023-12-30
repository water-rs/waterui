#[repr(C)]
#[derive(Debug)]
pub struct Subscriber {
    state: *mut (),
    subscriber: unsafe extern "C" fn(*mut ()),
}

impl<F> From<F> for Subscriber
where
    F: Fn() + Send + Sync,
{
    fn from(value: F) -> Self {
        Self::new(value)
    }
}

unsafe impl Send for Subscriber {}
unsafe impl Sync for Subscriber {}

impl Drop for Subscriber {
    fn drop(&mut self) {
        unsafe { drop(Box::from_raw(self.state)) }
    }
}

impl Subscriber {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn() + Send + Sync,
    {
        let boxed: Box<Box<dyn Fn()>> = Box::new(Box::new(f));
        let state = Box::into_raw(boxed) as *mut ();
        extern "C" fn from_fn_impl(state: *mut ()) {
            let boxed = state as *mut Box<dyn Fn()>;
            unsafe {
                let f = &*boxed;
                (f)()
            }
        }
        unsafe { Self::from_raw(state, from_fn_impl) }
    }

    pub fn call(&self) {
        unsafe { (self.subscriber)(self.state) }
    }

    /// Constructs subscribers from raw state and subscriber.
    /// # Safety
    /// The subscriber function is marked as unsafe because it requires a raw pointer.
    /// You must make sure the state and subscriber function is implemented correctly.
    pub unsafe fn from_raw(state: *mut (), subscriber: unsafe extern "C" fn(*mut ())) -> Self {
        Self { state, subscriber }
    }
}
