#![allow(non_snake_case)]

use crate::{zip::zip, Compute};

macro_rules! nested {
    ($a:tt,$b:tt) => {
        ($a, $b)
    };

    ($a:tt,$b:tt,$($remain:ident),*) => {
        nested!(($a,$b),$($remain),*)
    };
}

macro_rules! nested_call {
    ($a:ident,$b:ident) => {
        zip($a.clone(), $b.clone())
    };

    ($a:ident,$b:ident,$($remain:ident),*) => {{
        let v=nested_call!($a,$b);
        nested_call!(v,$($remain),*)
    }};
}

macro_rules! impl_tuples {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <$($ty:Compute+'static,)*>Compute for ($($ty),*)
        where ($($ty::Output),*):$crate::ComputeResult,
        {
            type Output = ($($ty::Output),*);
            fn compute(&self) -> Self::Output {
                let ($($ty),*)=self;
                let zip=nested_call!($($ty),*);

                let nested!($($ty),*)=zip.compute();
                ($($ty),*)

            }

            fn watch(&self, watcher: impl Into<crate::watcher::Watcher<Self::Output>>) -> crate::watcher::WatcherGuard {
                let ($($ty),*)=self;
                let watcher=watcher.into();
                let zip=nested_call!($($ty),*);
                zip.watch(crate::watcher::Watcher::new(move |nested!($($ty),*),metadata|{
                    $(
                        let $ty:$ty::Output=Clone::clone(&$ty);
                    )*
                    watcher.notify_with_metadata(($($ty),*),metadata)
                }))
            }
        }
    };
}

macro_rules! tuples {
    ($macro:ident) => {
        $macro!(T0, T1);
        $macro!(T0, T1, T2);
        $macro!(T0, T1, T2, T3);
        $macro!(T0, T1, T2, T3, T4);
        $macro!(T0, T1, T2, T3, T4, T5);
        $macro!(T0, T1, T2, T3, T4, T5, T6);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14);
    };
}
tuples!(impl_tuples);
