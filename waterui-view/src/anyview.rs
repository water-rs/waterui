use core::{
    any::{type_name, TypeId},
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use alloc::boxed::Box;

use crate::{Environment, View};

trait AnyViewImpl: 'static {
    fn body(self: Box<Self>, env: Environment) -> AnyView;
    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }
}

impl<T: View + 'static> AnyViewImpl for T {
    fn body(self: Box<Self>, env: Environment) -> AnyView {
        AnyView::new(View::body(*self, env))
    }
}

#[repr(transparent)]
pub struct AnyView {
    inner: Box<dyn AnyViewImpl>,
}

impl Debug for AnyView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("AnyView<{}>", self.name()))
    }
}

impl AnyView {
    pub fn new(view: impl View + 'static) -> Self {
        Self {
            inner: Box::new(view),
        }
    }

    pub fn is<T: 'static>(&self) -> bool {
        self.type_id() == TypeId::of::<T>()
    }

    pub fn type_id(&self) -> TypeId {
        AnyViewImpl::type_id(self.inner.deref())
    }

    pub fn downcast<T: 'static>(self) -> Result<Box<T>, AnyView> {
        if self.is::<T>() {
            unsafe { Ok(self.downcast_unchecked()) }
        } else {
            Err(self)
        }
    }

    pub fn name(&self) -> &'static str {
        AnyViewImpl::name(self.inner.deref())
    }

    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    pub unsafe fn downcast_unchecked<T: 'static>(self) -> Box<T> {
        unsafe { Box::from_raw(Box::into_raw(self.inner) as *mut T) }
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

#[cfg(test)]
mod test {
    use core::any::TypeId;

    use super::AnyView;

    #[test]
    pub fn get_type_id() {
        assert_eq!(AnyView::new(()).type_id(), TypeId::of::<()>())
    }
}
