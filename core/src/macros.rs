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

#[macro_export]
macro_rules! raw_view {
    ($ty:ty) => {
        impl $crate::View for $ty {
            fn body(self, _env: &$crate::Environment) -> impl $crate::View {
                panic!("You cannot call `body` for a raw view, may you need to handle this view `{}` manually", core::any::type_name::<$ty>());
            }
        }
    };
}

#[macro_export]
macro_rules! configurable {
    ($view:ident,$config:ty) => {
            $crate::configurable!($view,$config,"");
    };

    ($view:ident,$config:ty,$doc:expr) => {
        #[derive(Debug)]
        #[doc=$doc]
        pub struct $view($config);
        uniffi::custom_type!($view,$config,{
            lower:|value|{
                $crate::view::ConfigurableView::config(value)
            },
            try_lift:|value|{
                Ok(value.into())
            }
        });

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

        $crate::__paste!{
            #[uniffi::export]

            fn [<$view:lower _id>]() -> alloc::string::String{
                format!("{:?}", core::any::TypeId::of::<$view>())
            }
        }



        impl $crate::view::View for $view {
            fn body(self, env: &$crate::Environment) -> impl $crate::View {
                use $crate::view::ConfigurableView;
                if let Some(modifier) = env.get::<$crate::view::Modifier<Self>>() {
                    $crate::components::AnyView::new(
                        modifier.clone().modify(env.clone(), self.config()),
                    )
                } else {
                    panic!("This view ({}) depends on a platform view, but the renderer is not handling it. Check the implementation of the renderer", core::any::type_name::<$view>())
                }
            }
        }
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
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14);
    };
}

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

#[macro_export]
macro_rules! ffi_handler {
    ($ty:ty) => {
        $crate::__paste! {
            #[derive(uniffi::Object)]
            pub struct [<FFIBoxHandler$ty>]($crate::task::OnceValue<$crate::handler::BoxHandler<$ty>>);
            #[uniffi::export]
            impl [<FFIBoxHandler$ty>] {
                pub fn handle(&self,env:$crate::Environment) -> $ty {
                    let value = self.0.get();
                    $crate::handler::Handler::handle(&**value,&env)
                }
            }

            type [<BoxHandler$ty>] = $crate::handler::BoxHandler<$ty>;
            uniffi::custom_type!([<BoxHandler$ty>], alloc::sync::Arc<[<FFIBoxHandler$ty>]>,{
                remote,
                lower: |value| {alloc::sync::Arc::new([<FFIBoxHandler$ty>](value.into()))},
                try_lift: |value| {Ok(value.0.take())}
            });


        }
    };
}
