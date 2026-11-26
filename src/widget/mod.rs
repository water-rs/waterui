#[cfg(feature = "avatar")]
pub use waterui_controls::avatar;
pub use waterui_controls::card;
pub use waterui_controls::condition;
pub mod disclosure;
pub use waterui_controls::divider;
pub use waterui_controls::error;
pub use waterui_controls::suspense;

#[cfg(feature = "avatar")]
pub use avatar::{Avatar, avatar};
pub use card::{Card, CardConfig, CardStyle};
pub use condition::{When, when};
pub use disclosure::{Disclosure, disclosure};
pub use divider::Divider;
pub use error::{DefaultErrorView, ErrorView, ErrorViewBuilder, ResultExt, error_view};
pub use suspense::{SuspendedView, Suspense};

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
