//! Floating-surface presentation for elevated interactive views.

use waterui_core::metadata::MetadataKey;

/// Marks a view as promoted to the floating interaction layer.
///
/// Floating presentation is a semantic attribute realized by each backend:
/// Apple platforms realize it as a native elevated floating container, while
/// Hydrolysis realizes the active style package's floating surface look.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Floating;

impl MetadataKey for Floating {}

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
