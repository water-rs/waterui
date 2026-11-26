//! Floating-surface presentation for elevated interactive views.

pub use waterui_shape::floating::Floating;

#[cfg(test)]
mod tests {
    use waterui_core::{Environment, backend::peel_metadata};
    use waterui_text::Text;

    use super::Floating;
    use crate::ViewExt as _;

    #[test]
    fn floating_attaches_floating_metadata() {
        let env = Environment::new();
        let metadata = peel_metadata::<Floating>(Text::new("label").floating(), &env)
            .expect(".floating() must attach Metadata<Floating>");
        assert_eq!(metadata.value, Floating);
    }
}
