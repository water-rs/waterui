//! Names derived from realized content: the label a node announces when an
//! enclosing node — a tap region, a list row, a tab — names itself from what
//! it wraps.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

impl<E: crate::text::TextEngine> RenderNode<E> {
    /// The spoken label this subtree's content derives, read from its current
    /// state: a text's content, a control's own label, and a container's
    /// children's labels joined by spaces. Layout-transparent wrappers answer
    /// for their child; scroll surfaces, collections and drawn content name
    /// nothing.
    pub(crate) fn accessibility_label(&self, renderer: &mut SemanticCore<E>) -> Option<String> {
        match self {
            Self::Text(text) => Some(
                renderer
                    .read_signal(&text.content)
                    .to_semantic()
                    .to_string(),
            ),
            Self::Widget(node) => node.behavior.accessibility_label(renderer, &node.env),
            Self::Container(container) => {
                let labels = container
                    .children
                    .iter()
                    .filter_map(|child| child.accessibility_label(renderer))
                    .map(|label| label.trim().to_owned())
                    .filter(|label| !label.is_empty())
                    .collect::<Vec<_>>();
                (!labels.is_empty()).then(|| labels.join(" "))
            }
            Self::Opacity(node) => node.child.accessibility_label(renderer),
            Self::Scale(node) => node.child.accessibility_label(renderer),
            Self::Rotation(node) => node.child.accessibility_label(renderer),
            Self::Offset(node) => node.child.accessibility_label(renderer),
            Self::Retain(node) => node.child.accessibility_label(renderer),
            Self::Env(node) => node.child.accessibility_label(renderer),
            Self::Wrapper(node) => node.child.accessibility_label(renderer),
            Self::Filtered(node) => node.child.accessibility_label(renderer),
            Self::Fill(node) => node.child.accessibility_label(renderer),
            Self::Dynamic(node) => node.child.borrow().accessibility_label(renderer),
            Self::Color(_)
            | Self::Scroll(_)
            | Self::Collection(_)
            | Self::LazyStack(_)
            | Self::SceneView(_) => None,
            #[cfg(hydrolysis_gpu)]
            Self::GpuContent(_) | Self::ExternalFrame(_) => None,
        }
    }
}
