pub mod accordion;
pub mod card;
pub mod condition;
pub mod error;
pub mod suspense;
// pub mod tree;

pub use accordion::{Accordion, accordion};
pub use card::{Card, CardStyle, CardStyleTokens, CardTheme, card};
pub use suspense::{Suspense, suspense};
// pub use tree::{TreeNode, TreeView, tree_view};

/// Syntax highlighted code widget.
pub mod code;
/// Rich text widget support.
#[macro_use]
pub mod rich_text;
#[cfg(feature = "flow-markdown")]
/// Streaming Markdown renderer with incremental flow animations.
pub mod flow_markdown;
pub use code::{Code, code};
#[cfg(feature = "flow-markdown")]
pub use flow_markdown::{
    FlowAnimationPolicy, FlowAnimationPreset, FlowElementKind, FlowMarkdown, FlowStreamMode,
    FlowTablePolicy, flow_markdown,
};
pub use rich_text::{RichText, RichTextElement, rich_text};
pub mod divider;
pub use divider::Divider;
