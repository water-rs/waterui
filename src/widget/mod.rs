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

/// Syntax highlighted code widget. It lives in `waterui-text` so a component
/// crate can claim a fence through `Hook<CodeConfig>` without depending on
/// this crate; the path here is kept for application code.
#[cfg(feature = "highlight")]
pub use waterui_text::code;
/// Rich text widget support.
#[cfg(feature = "flow-markdown")]
#[macro_use]
pub mod rich_text;
#[cfg(feature = "flow-markdown")]
/// Streaming Markdown renderer with incremental flow animations.
pub mod flow_markdown;
// `waterui_text::code` is both the module and the `code(..)` constructor, so
// the `use` above already brings the function in; only the type is left.
#[cfg(feature = "highlight")]
pub use code::Code;
#[cfg(feature = "flow-markdown")]
pub use flow_markdown::{
    FlowAnimationPolicy, FlowAnimationPreset, FlowElementKind, FlowMarkdown, FlowStreamMode,
    FlowTablePolicy, flow_markdown,
};
#[cfg(feature = "flow-markdown")]
pub use rich_text::{RichText, RichTextElement, rich_text};
pub mod divider;
pub use divider::Divider;
