#[macro_export]
macro_rules! raw_view {
    ($ty:ty) => {
        impl $crate::View for $ty {
            fn body(self, _env: $crate::Environment) -> impl $crate::View {
                panic!("You cannot call `body` for a raw view, may you need to handle this view `{}` manually", core::any::type_name::<$ty>());
            }
        }
    };
}

#[macro_export]
macro_rules! configurable {
    ($view:ident,$config:ty) => {
        #[derive(Debug)]
        pub struct $view($config);

        impl $crate::view::ConfigurableView for $view {
            type Config = $config;

            fn config(self) -> Self::Config {
                self.0
            }
        }

        impl From<$config> for $view {
            fn from(value: $config) -> Self {
                Self(value)
            }
        }

        impl $crate::view::View for $view {
            fn body(self, env: $crate::Environment) -> impl $crate::View {
                use $crate::view::ConfigurableView;
                if let Some(modifier) = env.try_get::<$crate::view::Modifier<Self>>() {
                    $crate::components::AnyView::new(
                        modifier.clone().modify(env.clone(), self.config()),
                    )
                } else {
                    $crate::components::AnyView::new($crate::components::native::Native(self.0))
                }
            }
        }
    };
}

macro_rules! tuples {
    ($macro:ident) => {
        $macro!(T0);
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
