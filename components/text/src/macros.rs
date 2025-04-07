#[macro_export]
macro_rules! text {
    ($fmt:tt,$($arg:ident),*) => {
        {
            let args=($($arg.clone()),*);
            use waterui_reactive::ComputeExt;
            #[allow(unused_parens)]
            $crate::Text::new(ComputeExt::map(
                args.clone(),|($($arg),*)|{
                    format!($fmt,$($arg),*)
                }
            ).computed())
        }
    };
}
