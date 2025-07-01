/// Creates a reactive text component with formatted content.
///
/// This macro provides a convenient way to create text components with
/// formatted content that automatically updates when reactive values change.
///
/// # Usage
///
/// ```ignore
/// let name = binding("World");
/// let greeting = text!("Hello, {}!", name);
/// ```
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
