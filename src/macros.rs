#[macro_export]
macro_rules! configurable {
    ($view:ident,$config:ty) => {
        #[derive(Debug)]
        #[must_use]
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
            fn body(self, env: &$crate::Environment) -> impl waterui_core::View {
                use waterui_core::view::ConfigurableView;
                if let Some(modifier) = env.get::<$crate::view::Modifier<Self>>() {
                    waterui_core::AnyView::new(modifier.clone().modify(env.clone(), self.config()))
                } else {
                    waterui_core::AnyView::new(waterui_core::components::native::Native(self.0))
                }
            }
        }
    };
}

macro_rules! impl_debug {
    ($ty:ty) => {
        impl core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(core::any::type_name::<Self>())
            }
        }
    };
}

macro_rules! impl_compute_result {
    ($ty:ty) => {
        impl core::cmp::PartialEq for $ty {
            fn eq(&self, _other: &Self) -> bool {
                false
            }
        }

        impl core::cmp::PartialOrd for $ty {
            fn partial_cmp(&self, _other: &Self) -> Option<core::cmp::Ordering> {
                None
            }
        }
    };
}
