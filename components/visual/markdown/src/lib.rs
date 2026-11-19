//! Markdown and streaming rich-text rendering components for `WaterUI`.

/// Streaming Markdown renderer with incremental flow animations.
pub mod flow_markdown;
/// Rich text widget support.
#[macro_use]
pub mod rich_text;

pub use flow_markdown::{
    FlowAnimationPolicy, FlowAnimationPreset, FlowElementKind, FlowMarkdown, FlowMarkdownConfig,
    FlowStreamMode, FlowTablePolicy, flow_markdown,
};
pub use rich_text::{
    CodeCopyFeedback, MarkdownTableAlignment, RichText, RichTextElement, rich_text,
};
