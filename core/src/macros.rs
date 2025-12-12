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

/// Implements a native view that is handled by the platform backend.
///
/// This macro implements both `NativeView` and `View` traits for a type.
/// The `View::body()` returns `Native(self)` to delegate to the native backend.
///
/// # Usage
///
/// ```ignore
/// // Default stretch axis (None)
/// raw_view!(Text);
///
/// // With explicit stretch axis
/// raw_view!(Color, StretchAxis::Both);
/// raw_view!(Spacer, StretchAxis::MainAxis);
/// ```
#[macro_export]
macro_rules! raw_view {
    // With explicit stretch axis
    ($ty:ty, $axis:expr) => {
        impl $crate::NativeView for $ty {
            fn stretch_axis(&self) -> $crate::layout::StretchAxis {
                $axis
            }
        }

        impl $crate::View for $ty {
            fn body(self, _env: &$crate::Environment) -> impl $crate::View {
                $crate::Native::new(self)
            }

            fn stretch_axis(&self) -> $crate::layout::StretchAxis {
                $axis
            }
        }
    };

    // Default stretch axis (None)
    ($ty:ty) => {
        impl $crate::NativeView for $ty {}

        impl $crate::View for $ty {
            fn body(self, _env: &$crate::Environment) -> impl $crate::View {
                $crate::Native::new(self)
            }

            fn stretch_axis(&self) -> $crate::layout::StretchAxis {
                $crate::layout::StretchAxis::None
            }
        }
    };
}

/// Creates a configurable view with builder pattern methods.
///
/// This macro generates a wrapper struct and builder methods for configuring views,
/// following the builder pattern commonly used in UI frameworks.
///
/// # Usage
///
/// ```ignore
/// // Default stretch axis (None) - for content-sized views
/// configurable!(Button, ButtonConfig);
///
/// // With explicit stretch axis - for views that expand
/// configurable!(Slider, SliderConfig, StretchAxis::Horizontal);
/// configurable!(Color, ColorConfig, StretchAxis::Both);
///
/// // With dynamic stretch axis (closure) - for runtime-dependent behavior
/// configurable!(Progress, ProgressConfig, |config| match config.style {
///     ProgressStyle::Linear => StretchAxis::Horizontal,
///     ProgressStyle::Circular => StretchAxis::None,
/// });
/// ```
#[macro_export]
macro_rules! configurable {
    // Internal implementation with stretch axis
    (@impl $(#[$meta:meta])*; $view:ident, $config:ty, $axis:expr) => {
        $(#[$meta])*
        pub struct $view($config);

        impl $crate::NativeView for $config {
            fn stretch_axis(&self) -> $crate::layout::StretchAxis {
                $axis
            }
        }

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
                    $crate::AnyView::new(hook.apply(env, config))
                } else {
                    $crate::AnyView::new($crate::Native::new(config))
                }
            }

            fn stretch_axis(&self) -> $crate::layout::StretchAxis {
                $crate::NativeView::stretch_axis(&self.0)
            }
        }
    };

    // Dynamic stretch axis with closure/function
    // Internal implementation that generates NativeView with the provided function
    (@impl_dynamic $(#[$meta:meta])*; $view:ident, $config:ty, $stretch_fn:expr) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $view($config);

        impl $crate::NativeView for $config {
            fn stretch_axis(&self) -> $crate::layout::StretchAxis {
                ($stretch_fn)(self)
            }
        }

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
                    $crate::AnyView::new(hook.apply(env, config))
                } else {
                    $crate::AnyView::new($crate::Native::new(config))
                }
            }

            fn stretch_axis(&self) -> $crate::layout::StretchAxis {
                $crate::NativeView::stretch_axis(&self.0)
            }
        }
    };

    // Public variant for dynamic stretch_axis with closure: |config| -> StretchAxis
    // IMPORTANT: This must come BEFORE the $axis:expr variant (closure pattern)
    ($(#[$meta:meta])* $view:ident, $config:ty, |$param:ident| $body:expr) => {
        $crate::configurable!(@impl_dynamic $(#[$meta])*; $view, $config, |$param: &$config| $body);
    };

    // With explicit stretch axis
    ($(#[$meta:meta])* $view:ident, $config:ty, $axis:expr) => {
        $crate::configurable!(@impl $(#[$meta])*; $view, $config, $axis);
    };

    // Default stretch axis (None)
    ($(#[$meta:meta])* $view:ident, $config:ty) => {
        $crate::configurable!(@impl $(#[$meta])*; $view, $config, $crate::layout::StretchAxis::None);
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
