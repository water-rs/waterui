use crate::extract::Extractor;
use alloc::boxed::Box;
use core::marker::PhantomData;

use crate::Environment;

pub trait Handler<T>: 'static {
    fn handle(&self, env: &Environment) -> T;
}

pub trait HandlerMut<T>: 'static {
    fn handle(&mut self, env: &Environment) -> T;
}

pub trait HandlerOnce<T>: 'static {
    fn handle(self, env: &Environment) -> T;
}

pub type BoxHandler<T> = Box<dyn Handler<T>>;
pub type BoxHandlerMut<T> = Box<dyn HandlerMut<T>>;
pub type BoxHandlerOnce<T> = Box<dyn HandlerOnce<T>>;

pub trait HandlerFn<P, T>: 'static {
    fn handle_inner(&self, env: &Environment) -> T;
}

pub trait HandlerFnMut<P, T>: 'static {
    fn handle_inner(&mut self, env: &Environment) -> T;
}

pub trait HandlerFnOnce<P, T>: 'static {
    fn handle_inner(self, env: &Environment) -> T;
}

macro_rules! impl_handle_fn {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<F, R, $($ty:Extractor,)*> HandlerFn<($($ty,)*),R> for F
        where
            F: Fn($($ty,)*) -> R+ 'static,
        {
            fn handle_inner(&self, env: &Environment) -> R {

                $(
                    let $ty:$ty=Extractor::extract(env).unwrap();
                )*

                self($($ty,)*)
            }
        }
    };
}

macro_rules! impl_handle_fn_mut {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<F, R, $($ty:Extractor,)*> HandlerFnMut<($($ty,)*),R> for F
        where
            F: FnMut($($ty,)*) -> R+ 'static,
        {
            fn handle_inner(&mut self, env: &Environment) -> R {

                $(
                    let $ty:$ty=Extractor::extract(env).unwrap();
                )*

                self($($ty,)*)
            }
        }
    };
}

tuples!(impl_handle_fn);

tuples!(impl_handle_fn_mut);

macro_rules! impl_handle_fn_once {
    ($($ty:ident),*) => {
        #[allow(unused_variables)]
        #[allow(non_snake_case)]
        impl<F, R, $($ty:Extractor,)*> HandlerFnOnce<($($ty,)*),R> for F
        where
            F: FnOnce($($ty,)*) -> R+ 'static,
        {
            fn handle_inner(self, env: &Environment) -> R {

                $(
                    let $ty:$ty=Extractor::extract(env).unwrap();
                )*

                self($($ty,)*)
            }
        }
    };
}

tuples!(impl_handle_fn_once);

macro_rules! into_handlers {
    ($name:ident,$handler:ident,$handler_fn:ident) => {
        pub struct $name<H, P, T> {
            h: H,
            _marker: PhantomData<(P, T)>,
        }

        impl<H, P, T> $name<H, P, T>
        where
            H: $handler_fn<P, T>,
        {
            pub fn new(h: H) -> Self {
                Self {
                    h,
                    _marker: PhantomData,
                }
            }
        }
    };
}

into_handlers!(IntoHandler, Handler, HandlerFn);

impl<H, P, T> Handler<T> for IntoHandler<H, P, T>
where
    H: HandlerFn<P, T>,
    P: 'static,
    T: 'static,
{
    fn handle(&self, env: &Environment) -> T {
        self.h.handle_inner(env)
    }
}

impl<H, P, T> HandlerMut<T> for IntoHandlerMut<H, P, T>
where
    H: HandlerFnMut<P, T>,
    P: 'static,
    T: 'static,
{
    fn handle(&mut self, env: &Environment) -> T {
        self.h.handle_inner(env)
    }
}

impl<H, P, T> HandlerOnce<T> for IntoHandlerOnce<H, P, T>
where
    H: HandlerFnOnce<P, T>,
    P: 'static,
    T: 'static,
{
    fn handle(self, env: &Environment) -> T {
        self.h.handle_inner(env)
    }
}

into_handlers!(IntoHandlerMut, HandlerMut, HandlerFnMut);

into_handlers!(IntoHandlerOnce, HandlerOnce, HandlerFnOnce);

pub fn into_handler<P, T>(h: impl HandlerFn<P, T>) -> impl Handler<T>
where
    P: 'static,
    T: 'static,
{
    IntoHandler::new(h)
}

pub fn into_handler_mut<P, T>(h: impl HandlerFnMut<P, T>) -> impl HandlerMut<T>
where
    P: 'static,
    T: 'static,
{
    IntoHandlerMut::new(h)
}

pub fn into_handler_once<P, T>(h: impl HandlerFnOnce<P, T>) -> impl HandlerOnce<T>
where
    P: 'static,
    T: 'static,
{
    IntoHandlerOnce::new(h)
}
