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
            use nami::SignalExt;
            #[allow(unused_parens)]
            $crate::Text::new(SignalExt::map(
                args.clone(),|($($arg),*)|{
                    format!($fmt,$($arg),*)
                }
            ).computed())
        }
    };
}
