use std::{
    any::TypeId,
    ops::{Deref, DerefMut},
};

use crate::{Environment, View, ViewExt};

pub(crate) trait AnyViewImpl: Send + Sync + 'static {
    fn body(self: Box<Self>, env: Environment) -> AnyView;
    fn inner_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

impl<T: View + 'static> AnyViewImpl for T {
    fn body(self: Box<Self>, env: Environment) -> AnyView {
        View::body(*self, env).anyview()
    }
}

#[repr(transparent)]
pub struct AnyView {
    inner: Box<dyn AnyViewImpl>,
}

impl_debug!(AnyView);

impl AnyView {
    pub fn new(view: impl View + 'static) -> Self {
        Self {
            inner: Box::new(view),
        }
    }

    pub fn is<T: 'static>(&self) -> bool {
        self.inner.inner_type_id() == TypeId::of::<T>()
    }

    pub fn downcast<T: 'static>(self) -> Result<Box<T>, AnyView> {
        if self.is::<T>() {
            unsafe { Ok(Box::from_raw(Box::into_raw(self.inner) as *mut T)) }
        } else {
            Err(self)
        }
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        if self.is::<T>() {
            unsafe { Some(&*(self.inner.deref() as *const dyn AnyViewImpl as *const T)) }
        } else {
            None
        }
    }

    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        if self.is::<T>() {
            unsafe { Some(&mut *(self.inner.deref_mut() as *mut dyn AnyViewImpl as *mut T)) }
        } else {
            None
        }
    }
}

impl View for AnyView {
    fn body(self, env: Environment) -> impl View {
        self.inner.body(env)
    }
}
