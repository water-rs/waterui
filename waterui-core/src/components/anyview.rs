use core::{
    any::{type_name, Any, TypeId},
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

impl<T: View> AnyViewImpl for T {
    fn body(self: Box<Self>, env: Environment) -> AnyView {
        AnyView::new(View::body(*self, env))
    }
}

pub struct AnyView(Box<dyn AnyViewImpl>);

impl Debug for AnyView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("AnyView({})", self.name()))
    }
}

impl Default for AnyView {
    fn default() -> Self {
        AnyView::new(())
    }
}

impl AnyView {
    pub fn new<V: View>(view: V) -> Self {
        if TypeId::of::<V>() == TypeId::of::<AnyView>() {
            let any = &mut Some(view) as &mut dyn Any;
            return any
                .downcast_mut::<Option<AnyView>>()
                .unwrap()
                .take()
                .unwrap();
        }

        Self(Box::new(view))
    }

    pub fn is<T: 'static>(&self) -> bool {
        self.type_id() == TypeId::of::<T>()
    }

    pub fn type_id(&self) -> TypeId {
        AnyViewImpl::type_id(self.0.deref())
    }

    pub fn name(&self) -> &'static str {
        AnyViewImpl::name(self.0.deref())
    }

    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    pub unsafe fn downcast_unchecked<T: 'static>(self) -> Box<T> {
        unsafe { Box::from_raw(Box::into_raw(self.0) as *mut T) }
    }

    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    pub unsafe fn downcast_ref_unchecked<T: 'static>(&self) -> &T {
        &*(self.0.deref() as *const dyn AnyViewImpl as *const T)
    }

    /// # Safety
    /// Calling this method with the incorrect type is undefined behavior.
    pub unsafe fn downcast_mut_unchecked<T: 'static>(&mut self) -> &mut T {
        &mut *(self.0.deref_mut() as *mut dyn AnyViewImpl as *mut T)
    }

    pub fn downcast<T: 'static>(self) -> Result<Box<T>, AnyView> {
        if self.is::<T>() {
            unsafe { Ok(self.downcast_unchecked()) }
        } else {
            Err(self)
        }
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        unsafe { self.is::<T>().then(|| self.downcast_ref_unchecked()) }
    }

    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        unsafe { self.is::<T>().then(|| self.downcast_mut_unchecked()) }
    }
}

impl View for AnyView {
    fn body(self, env: Environment) -> impl View {
        self.0.body(env)
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
