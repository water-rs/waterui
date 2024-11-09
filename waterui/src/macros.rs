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
                use $crate::view::ViewExt;
                if let Some(modifier) = env.get::<$crate::view::Modifier<Self>>() {
                    modifier
                        .clone()
                        .modify(env.clone(), self.config())
                        .anyview()
                } else {
                    waterui_core::components::native::Native(self.0).anyview()
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

macro_rules! modify_field {
    ($ident:ident,$ty:ty) => {
        pub fn $ident(mut self, size: impl Into<$ty>) -> Self {
            self.$ident = size.into();
            self
        }
    };
}

#[macro_export]
macro_rules! text {
    ($fmt:tt,$($arg:ident),*) => {
        {
            let args=($($arg.clone()),*);
            use $crate::ComputeExt;
            #[allow(unused_parens)]
            ComputeExt::map(
                &args,|($($arg),*)|{
                    format!($fmt,$($arg),*)
                }
            ).computed()
        }
    };
}
