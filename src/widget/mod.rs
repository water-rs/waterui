#[cfg(feature = "avatar")]
pub mod avatar;
pub mod card;
pub mod condition;
pub mod disclosure;
pub mod error;
pub mod suspense;

#[cfg(feature = "avatar")]
pub use avatar::{Avatar, avatar};
pub use card::{Card, CardStyle, CardStyleTokens, CardTheme, card};
pub use condition::{When, when};
pub use disclosure::{Disclosure, disclosure};
pub use error::{DefaultErrorView, ErrorView, ErrorViewBuilder, ResultExt, error_view};
pub use suspense::{SuspendedView, Suspense, suspense};

/// Syntax highlighted code widget. It lives in `waterui-text` so a component
/// crate can claim a fence with a `CodeConfig` hook without depending on
/// this crate; the path here is kept for application code.
#[cfg(feature = "highlight")]
pub use waterui_text::code;
// `waterui_text::code` is both the module and the `code(..)` constructor, so
// the `use` above already brings the function in; only the type is left.
#[cfg(feature = "highlight")]
pub use code::{Code, CodeLanguage};
/// Markdown and streaming rich-text rendering widgets.
#[cfg(feature = "flow-markdown")]
pub use waterui_markdown::{
    flow_markdown::{
        self, FlowAnimationPolicy, FlowAnimationPreset, FlowElementKind, FlowMarkdown,
        FlowMarkdownConfig, FlowStreamMode, FlowTablePolicy, flow_markdown,
    },
    rich_text::{self, MarkdownTableAlignment, RichText, RichTextElement, rich_text},
};
pub mod divider;
pub use divider::Divider;
