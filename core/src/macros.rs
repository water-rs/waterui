/// Implements a basic `Debug` trait for types using their type name.
///
/// This macro generates a `Debug` implementation that simply prints the type name,
/// useful for types where the internal structure doesn't need to be exposed.
#[macro_export]
macro_rules! impl_debug {
    ($ty:ty) => {
        impl core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(core::any::type_name::<Self>())
            }
        }
    };
}

/// Implements a raw view that panics when `body()` is called.
///
/// This macro is used for views that should be handled specially by the renderer
/// and should not have their `body()` method called in normal view composition.
#[macro_export]
macro_rules! raw_view {
    ($ty:ty) => {
        impl $crate::View for $ty {
            #[allow(clippy::unused_unit)]
            #[allow(unused)]
            fn body(self, _env: &$crate::Environment) -> impl $crate::View {
                panic!("You cannot call `body` for a raw view, may you need to handle this view `{}` manually", core::any::type_name::<$ty>());
                ()
            }
        }
    };
}

/// Creates a configurable view with builder pattern methods.
///
/// This macro generates a wrapper struct and builder methods for configuring views,
/// following the builder pattern commonly used in UI frameworks.
#[macro_export]
macro_rules! configurable {
    (@impl $(#[$meta:meta])*; $view:ident, $config:ty) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $view($config);

        impl $crate::view::ConfigurableView for $view {
            type Config = $config;
            #[inline] fn config(self) -> Self::Config { self.0 }
        }

        impl $crate::view::ViewConfiguration for $config {
            type View = $view;
            #[inline] fn render(self) -> Self::View { $view(self) }
        }

        impl From<$config> for $view {
            #[inline] fn from(value: $config) -> Self { Self(value) }
        }

        impl $crate::view::View for $view {
            fn body(self, env: &$crate::Environment) -> impl $crate::View {
                use $crate::view::ConfigurableView;
                let config = self.config();
                if let Some(hook) = env.get::<$crate::view::Hook<$config>>() {
                    $crate::components::AnyView::new(hook.apply(env, config))
                } else {
                    $crate::components::AnyView::new($crate::components::Native(config))
                }
            }
        }
    };

    ($(#[$meta:meta])* $view:ident, $config:ty) => {
        $crate::configurable!(@impl $(#[$meta])*; $view, $config);
    };

}
macro_rules! tuples {
    ($macro:ident) => {
        $macro!();
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
        $macro!(
            T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14
        );
    };
}

/// Implements the `Extractor` trait for a type.
///
/// This macro generates an implementation that extracts values from the environment
/// using the `Use<T>` wrapper, commonly used for dependency injection.
#[macro_export]
macro_rules! impl_extractor {
    ($ty:ty) => {
        impl $crate::extract::Extractor for $ty {
            fn extract(env: &$crate::Environment) -> core::result::Result<Self, $crate::Error> {
                $crate::extract::Extractor::extract(env)
                    .map(|value: $crate::extract::Use<$ty>| value.0)
            }
        }
    };
}

/// Implements the `Deref` trait for transparent access to an inner type.
///
/// This macro generates a `Deref` implementation that allows transparent
/// access to the inner value of wrapper types.
#[macro_export]
macro_rules! impl_deref {
    ($ty:ty,$target:ty) => {
        impl core::ops::Deref for $ty {
            type Target = $target;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::ops::DerefMut for $ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}
