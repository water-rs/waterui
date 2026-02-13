use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    ops::Range,
    rc::Rc,
    time::Duration,
};

use executor_core::spawn_local;
use nami::{
    Computed, Signal,
    signal::IntoComputed,
    watcher::{Context, Metadata as WatcherMetadata},
};
use native_executor::sleep;
use tree_sitter::{Parser, Tree};
use waterui_core::{AnyView, Metadata, Retain, View, dynamic::Dynamic};
use waterui_graphics::color::Srgb;
use waterui_layout::stack::{HorizontalAlignment, VStack};
use waterui_str::Str;
use waterui_text::{
    highlight::Language,
    styled::{Style, StyledStr},
    text,
};

use crate::{
    ViewExt,
    animation::Animation,
    widget::rich_text::{RichText, RichTextElement},
};

/// Flow-optimized Markdown view for LLM streaming output.
#[derive(Debug, Clone)]
pub struct FlowMarkdown {
    source: Computed<Str>,
    config: FlowMarkdownConfig,
}

/// Convenience constructor for [`FlowMarkdown`].
#[must_use]
pub fn flow_markdown(source: impl IntoComputed<Str>) -> FlowMarkdown {
    FlowMarkdown::new(source)
}

impl FlowMarkdown {
    /// Creates a new [`FlowMarkdown`] view.
    #[must_use]
    pub fn new(source: impl IntoComputed<Str>) -> Self {
        Self {
            source: source.into_computed(),
            config: FlowMarkdownConfig::default(),
        }
    }

    /// Sets the animation preset.
    #[must_use]
    pub fn preset(mut self, preset: FlowAnimationPreset) -> Self {
        self.config.preset = preset;
        self
    }

    /// Overrides animation policy for a specific element kind.
    #[must_use]
    pub fn override_animation(
        mut self,
        kind: FlowElementKind,
        policy: FlowAnimationPolicy,
    ) -> Self {
        self.config.overrides.insert(kind, policy);
        self
    }

    /// Sets stream mode.
    #[must_use]
    pub fn stream(mut self, mode: FlowStreamMode) -> Self {
        self.config.stream_mode = mode;
        self
    }

    /// Sets max pending bytes kept for partial parsing.
    #[must_use]
    pub fn max_pending_bytes(mut self, bytes: usize) -> Self {
        self.config.max_pending_bytes = bytes.max(256);
        self
    }

    /// Sets table rendering policy for incomplete streamed tables.
    #[must_use]
    pub fn table_policy(mut self, policy: FlowTablePolicy) -> Self {
        self.config.table_policy = policy;
        self
    }
}

impl View for FlowMarkdown {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let (handler, dynamic) = Dynamic::new();
        let source = self.source.clone();
        let config = self.config.clone();
        let state = Rc::new(RefCell::new(FlowMarkdownState::new(config)));

        let initial = source.get();
        let initial_update = state
            .borrow_mut()
            .recompute(initial.as_str(), WatcherMetadata::new());
        let initial_typewriter = initial_update.typewriter.clone();
        handler.set_with_metadata(initial_update.view, initial_update.metadata);
        spawn_typewriter_reveal_if_needed(&state, &handler, initial_typewriter);

        let guard_source = source.clone();
        let guard = source.watch({
            let state = Rc::clone(&state);
            let handler = handler.clone();
            move |ctx: Context<Str>| {
                let metadata = ctx.metadata().clone();
                let markdown = ctx.into_value();
                let update = state.borrow_mut().recompute(markdown.as_str(), metadata);
                let typewriter = update.typewriter.clone();
                handler.set_with_metadata(update.view, update.metadata);
                spawn_typewriter_reveal_if_needed(&state, &handler, typewriter);
            }
        });

        Metadata::new(dynamic, Retain::new((guard, guard_source, state)))
    }
}

fn spawn_typewriter_reveal_if_needed(
    state: &Rc<RefCell<FlowMarkdownState>>,
    handler: &waterui_core::dynamic::DynamicHandler,
    run: Option<TypewriterRun>,
) {
    let Some(run) = run else {
        return;
    };

    let state = Rc::clone(state);
    let handler = handler.clone();
    spawn_local(async move {
        loop {
            sleep(Duration::from_millis(run.batch_ms)).await;
            let next = {
                let mut state = state.borrow_mut();
                state.advance_typewriter(run.revision, run.batch_chars)
            };
            let Some(view) = next else {
                return;
            };
            handler.set(view);
        }
    })
    .detach();
}

/// Global configuration for [`FlowMarkdown`].
#[derive(Debug, Clone)]
pub struct FlowMarkdownConfig {
    preset: FlowAnimationPreset,
    overrides: HashMap<FlowElementKind, FlowAnimationPolicy>,
    stream_mode: FlowStreamMode,
    max_pending_bytes: usize,
    table_policy: FlowTablePolicy,
}

impl Default for FlowMarkdownConfig {
    fn default() -> Self {
        Self {
            preset: FlowAnimationPreset::AssistantDefault,
            overrides: HashMap::new(),
            stream_mode: FlowStreamMode::AppendOnly,
            max_pending_bytes: 32 * 1024,
            table_policy: FlowTablePolicy::NoAnimationReadablePending,
        }
    }
}

/// Preset animation profile for streamed markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowAnimationPreset {
    /// Optimized for assistant/LLM output.
    AssistantDefault,
    /// Minimal transitions.
    Minimal,
    /// Disable all component-added animation metadata.
    None,
}

/// Element kinds used for animation overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowElementKind {
    /// Plain paragraph text and fallback text nodes.
    Text,
    /// Heading blocks.
    Heading,
    /// List items (ordered and unordered).
    ListItem,
    /// Block quotes.
    Quote,
    /// Fenced/indented code blocks.
    CodeBlock,
    /// Markdown tables.
    Table,
    /// Images.
    Image,
    /// Link-heavy inline blocks.
    Link,
    /// Horizontal rules / thematic breaks.
    Hr,
}

const FLOW_KIND_PRIORITY: [FlowElementKind; 9] = [
    FlowElementKind::Text,
    FlowElementKind::Heading,
    FlowElementKind::ListItem,
    FlowElementKind::Quote,
    FlowElementKind::Link,
    FlowElementKind::CodeBlock,
    FlowElementKind::Table,
    FlowElementKind::Image,
    FlowElementKind::Hr,
];

/// Animation policy for flow updates.
#[derive(Debug, Clone)]
pub enum FlowAnimationPolicy {
    /// No animation.
    None,
    /// Fade/cross-dissolve using WaterUI animation metadata.
    Fade(Animation),
    /// Typewriter-style progressive reveal for text-like content.
    Typewriter {
        /// Characters revealed per second.
        cps: u32,
        /// Milliseconds per reveal batch.
        batch_ms: u64,
    },
}

/// Stream update model for flow markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowStreamMode {
    /// Tail-append optimized model.
    AppendOnly,
}

/// Policy for incomplete tables in streaming mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowTablePolicy {
    /// Do not animate table blocks and avoid exposing raw markdown while incomplete.
    NoAnimationReadablePending,
}

#[derive(Debug, Clone)]
struct FlowBlock {
    range: Range<usize>,
    kind: FlowElementKind,
    elements: Vec<RichTextElement>,
}

#[derive(Debug)]
struct FlowUpdate {
    view: AnyView,
    metadata: WatcherMetadata,
    typewriter: Option<TypewriterRun>,
}

struct FlowMarkdownState {
    config: FlowMarkdownConfig,
    parser: Option<Parser>,
    tree: Option<Tree>,
    source: String,
    blocks: Vec<FlowBlock>,
    typewriter_visible_chars: usize,
    typewriter_target_chars: usize,
    typewriter_revision: u64,
}

#[derive(Debug, Clone)]
struct TypewriterRun {
    revision: u64,
    batch_chars: usize,
    batch_ms: u64,
}

impl FlowMarkdownState {
    fn new(config: FlowMarkdownConfig) -> Self {
        Self {
            config,
            parser: init_markdown_parser(),
            tree: None,
            source: String::new(),
            blocks: Vec::new(),
            typewriter_visible_chars: 0,
            typewriter_target_chars: 0,
            typewriter_revision: 0,
        }
    }

    fn recompute(&mut self, markdown: &str, upstream_metadata: WatcherMetadata) -> FlowUpdate {
        let is_append_only = markdown.starts_with(&self.source);
        let appended = is_append_only && markdown.len() > self.source.len();
        let should_incremental = self.config.stream_mode == FlowStreamMode::AppendOnly
            && is_append_only
            && !self.source.is_empty();

        let previous_tree = self.tree.take();
        let next_tree = self
            .parser
            .as_mut()
            .and_then(|parser| parser.parse(markdown, previous_tree.as_ref()));

        let mut changed_kinds = HashSet::new();
        let mut blocks = Vec::new();

        if let Some(tree) = &next_tree {
            let ranges = collect_block_ranges(tree, markdown.len());
            let changed_ranges = collect_changed_ranges(
                previous_tree.as_ref(),
                tree,
                markdown.len(),
                should_incremental,
            );

            let previous_map: HashMap<(usize, usize), FlowBlock> = self
                .blocks
                .iter()
                .cloned()
                .map(|b| ((b.range.start, b.range.end), b))
                .collect();

            for block in ranges {
                let key = (block.range.start, block.range.end);
                let has_changed = changed_ranges
                    .iter()
                    .any(|range| ranges_overlap(range, &block.range));

                if !has_changed
                    && should_incremental
                    && let Some(old) = previous_map.get(&key)
                {
                    blocks.push(old.clone());
                    continue;
                }

                let parsed = parse_block(
                    markdown,
                    &block,
                    self.config.table_policy,
                    self.config.max_pending_bytes,
                );
                changed_kinds.insert(parsed.kind);
                blocks.push(parsed);
            }
        } else {
            let fallback = parse_fallback(markdown);
            changed_kinds.insert(FlowElementKind::Text);
            blocks.push(fallback);
        }

        self.source.clear();
        self.source.push_str(markdown);
        self.blocks = blocks;
        self.tree = next_tree;

        self.typewriter_revision = self.typewriter_revision.wrapping_add(1);

        let full_typewriter_chars = self.total_typewriter_char_count(&self.blocks);
        let typewriter = if should_incremental && appended {
            self.resolve_typewriter_run(&changed_kinds)
                .and_then(|(batch_chars, batch_ms)| {
                    let visible = self.typewriter_visible_chars.min(full_typewriter_chars);
                    self.typewriter_visible_chars = visible;
                    self.typewriter_target_chars = full_typewriter_chars;
                    (visible < full_typewriter_chars).then_some(TypewriterRun {
                        revision: self.typewriter_revision,
                        batch_chars,
                        batch_ms,
                    })
                })
        } else {
            self.typewriter_visible_chars = full_typewriter_chars;
            self.typewriter_target_chars = full_typewriter_chars;
            None
        };

        let metadata = if typewriter.is_some() {
            upstream_metadata
        } else {
            self.resolve_animation_metadata(upstream_metadata, &changed_kinds)
        };
        let view = self.build_current_view();
        FlowUpdate {
            view,
            metadata,
            typewriter,
        }
    }

    fn advance_typewriter(&mut self, revision: u64, batch_chars: usize) -> Option<AnyView> {
        if revision != self.typewriter_revision
            || self.typewriter_visible_chars >= self.typewriter_target_chars
        {
            return None;
        }

        self.typewriter_visible_chars =
            (self.typewriter_visible_chars + batch_chars.max(1)).min(self.typewriter_target_chars);
        Some(self.build_current_view())
    }

    fn build_current_view(&self) -> AnyView {
        let budget = (self.typewriter_visible_chars < self.typewriter_target_chars)
            .then_some(self.typewriter_visible_chars);
        build_flow_view(&self.blocks, budget, &self.config)
    }

    fn total_typewriter_char_count(&self, blocks: &[FlowBlock]) -> usize {
        blocks
            .iter()
            .filter(|block| self.has_typewriter_policy(block.kind))
            .map(|block| {
                block
                    .elements
                    .iter()
                    .map(rich_text_element_text_len)
                    .sum::<usize>()
            })
            .sum()
    }

    fn resolve_typewriter_run(
        &self,
        changed_kinds: &HashSet<FlowElementKind>,
    ) -> Option<(usize, u64)> {
        for kind in FLOW_KIND_PRIORITY {
            if !changed_kinds.contains(&kind) {
                continue;
            }
            if let Some(FlowAnimationPolicy::Typewriter { cps, batch_ms }) =
                self.animation_policy(kind)
            {
                let batch_ms = batch_ms.max(1);
                let batch_chars = ((u64::from(cps.max(1)) * batch_ms) / 1000).max(1) as usize;
                return Some((batch_chars, batch_ms));
            }
        }
        None
    }

    fn has_typewriter_policy(&self, kind: FlowElementKind) -> bool {
        matches!(
            self.animation_policy(kind),
            Some(FlowAnimationPolicy::Typewriter { .. })
        )
    }

    fn resolve_animation_metadata(
        &self,
        mut metadata: WatcherMetadata,
        changed_kinds: &HashSet<FlowElementKind>,
    ) -> WatcherMetadata {
        for kind in FLOW_KIND_PRIORITY {
            if !changed_kinds.contains(&kind) {
                continue;
            }
            if let Some(FlowAnimationPolicy::Fade(animation)) = self.animation_policy(kind) {
                metadata = metadata.with(animation);
                break;
            }
        }
        metadata
    }

    fn animation_policy(&self, kind: FlowElementKind) -> Option<FlowAnimationPolicy> {
        Some(animation_policy_for_kind(&self.config, kind))
    }
}

fn animation_policy_for_kind(
    config: &FlowMarkdownConfig,
    kind: FlowElementKind,
) -> FlowAnimationPolicy {
    if let Some(policy) = config.overrides.get(&kind) {
        return policy.clone();
    }

    match config.preset {
        FlowAnimationPreset::AssistantDefault => match kind {
            FlowElementKind::Text
            | FlowElementKind::Heading
            | FlowElementKind::ListItem
            | FlowElementKind::Quote
            | FlowElementKind::Link => FlowAnimationPolicy::Typewriter {
                cps: 64,
                batch_ms: 40,
            },
            FlowElementKind::CodeBlock | FlowElementKind::Table => FlowAnimationPolicy::None,
            FlowElementKind::Image => {
                FlowAnimationPolicy::Fade(Animation::ease_in_out(Duration::from_millis(180)))
            }
            FlowElementKind::Hr => FlowAnimationPolicy::None,
        },
        FlowAnimationPreset::Minimal => match kind {
            FlowElementKind::Image => {
                FlowAnimationPolicy::Fade(Animation::ease_in_out(Duration::from_millis(120)))
            }
            _ => FlowAnimationPolicy::None,
        },
        FlowAnimationPreset::None => FlowAnimationPolicy::None,
    }
}

#[derive(Debug, Clone)]
struct BlockRange {
    range: Range<usize>,
    kind: FlowElementKind,
}

fn init_markdown_parser() -> Option<Parser> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_md::LANGUAGE.into()).ok()?;
    Some(parser)
}

fn collect_changed_ranges(
    previous: Option<&Tree>,
    next: &Tree,
    text_len: usize,
    incremental: bool,
) -> Vec<Range<usize>> {
    if let Some(previous) = previous
        && incremental
    {
        let ranges: Vec<Range<usize>> = previous
            .changed_ranges(next)
            .map(|r| r.start_byte..r.end_byte)
            .collect();
        if !ranges.is_empty() {
            return ranges;
        }
    }
    vec![0..text_len]
}

fn collect_block_ranges(tree: &Tree, text_len: usize) -> Vec<BlockRange> {
    let mut blocks = Vec::new();
    let root = tree.root_node();
    let child_count = root.named_child_count();
    for i in 0..child_count {
        let Some(child) = root.named_child(i) else {
            continue;
        };
        let start = child.start_byte();
        let end = child.end_byte().min(text_len);
        if start >= end {
            continue;
        }
        blocks.push(BlockRange {
            range: start..end,
            kind: classify_block_kind(child.kind()),
        });
    }

    if blocks.is_empty() && text_len > 0 {
        blocks.push(BlockRange {
            range: 0..text_len,
            kind: FlowElementKind::Text,
        });
    }

    blocks
}

fn classify_block_kind(kind: &str) -> FlowElementKind {
    if kind.contains("heading") {
        FlowElementKind::Heading
    } else if kind.contains("list") || kind.contains("item") {
        FlowElementKind::ListItem
    } else if kind.contains("quote") {
        FlowElementKind::Quote
    } else if kind.contains("code") {
        FlowElementKind::CodeBlock
    } else if kind.contains("table") {
        FlowElementKind::Table
    } else if kind.contains("image") {
        FlowElementKind::Image
    } else if kind.contains("link") {
        FlowElementKind::Link
    } else if kind.contains("thematic_break") || kind.contains("horizontal_rule") {
        FlowElementKind::Hr
    } else {
        FlowElementKind::Text
    }
}

fn parse_block(
    markdown: &str,
    block: &BlockRange,
    table_policy: FlowTablePolicy,
    max_pending_bytes: usize,
) -> FlowBlock {
    let mut slice = markdown
        .get(block.range.clone())
        .map_or_else(String::new, ToString::to_string);

    if slice.len() > max_pending_bytes {
        slice.truncate(max_pending_bytes);
    }

    let elements = match block.kind {
        FlowElementKind::Table if is_incomplete_table(&slice) => match table_policy {
            FlowTablePolicy::NoAnimationReadablePending => {
                vec![RichTextElement::Text(StyledStr::plain(
                    "Streaming table...".to_string(),
                ))]
            }
        },
        FlowElementKind::Image if is_incomplete_image_fragment(&slice) => {
            vec![RichTextElement::Text(StyledStr::plain(
                extract_image_alt_or_placeholder(&slice),
            ))]
        }
        _ => {
            let rich = RichText::from_markdown(&slice);
            let parsed = rich.elements().to_vec();
            if parsed.is_empty() {
                vec![RichTextElement::Text(StyledStr::plain(
                    sanitize_pending_text_fragment(&slice),
                ))]
            } else {
                parsed
            }
        }
    };

    FlowBlock {
        range: block.range.clone(),
        kind: block.kind,
        elements,
    }
}

fn parse_fallback(markdown: &str) -> FlowBlock {
    FlowBlock {
        range: 0..markdown.len(),
        kind: FlowElementKind::Text,
        elements: RichText::from_markdown(markdown).elements().to_vec(),
    }
}

fn build_flow_view(
    blocks: &[FlowBlock],
    typewriter_budget: Option<usize>,
    config: &FlowMarkdownConfig,
) -> AnyView {
    let mut views = Vec::new();
    let mut remaining = typewriter_budget.unwrap_or(usize::MAX);
    let enforce_budget = typewriter_budget.is_some();

    for block in blocks {
        if enforce_budget && has_typewriter_policy_for_kind(block.kind, config) {
            for element in &block.elements {
                if remaining == 0 {
                    break;
                }
                if let Some(truncated) = truncate_rich_text_element(element, &mut remaining) {
                    push_rich_text_element_view(&mut views, truncated);
                }
            }
        } else {
            for element in &block.elements {
                push_rich_text_element_view(&mut views, element.clone());
            }
        }
    }

    AnyView::new(VStack::from_iter(views).alignment(HorizontalAlignment::Leading))
}

fn push_rich_text_element_view(views: &mut Vec<AnyView>, element: RichTextElement) {
    match element {
        RichTextElement::Code { code, language } => {
            views.push(AnyView::new(flow_code_view(language, code.as_str())));
        }
        _ => views.push(AnyView::new(element)),
    }
}

fn has_typewriter_policy_for_kind(kind: FlowElementKind, config: &FlowMarkdownConfig) -> bool {
    matches!(
        animation_policy_for_kind(config, kind),
        FlowAnimationPolicy::Typewriter { .. }
    )
}

fn rich_text_element_text_len(element: &RichTextElement) -> usize {
    match element {
        RichTextElement::Text(styled) => styled.to_plain().chars().count(),
        RichTextElement::Link { label, .. } => label.to_plain().chars().count(),
        RichTextElement::Image { alt, .. } => alt.chars().count(),
        RichTextElement::Table { headers, rows, .. } => {
            let header_len: usize = headers.iter().map(rich_text_element_text_len).sum();
            let row_len: usize = rows
                .iter()
                .map(|row| row.iter().map(rich_text_element_text_len).sum::<usize>())
                .sum();
            header_len + row_len
        }
        RichTextElement::List { items, .. } => items.iter().map(rich_text_element_text_len).sum(),
        RichTextElement::Code { code, .. } => code.chars().count(),
        RichTextElement::Quote { content } => content.iter().map(rich_text_element_text_len).sum(),
        RichTextElement::Group { elements, .. } => {
            elements.iter().map(rich_text_element_text_len).sum()
        }
        RichTextElement::Divider => 0,
    }
}

fn truncate_rich_text_element(
    element: &RichTextElement,
    remaining: &mut usize,
) -> Option<RichTextElement> {
    match element {
        RichTextElement::Text(styled) => {
            if *remaining == 0 {
                return None;
            }
            let visible = truncate_styled(styled, *remaining);
            let consumed = visible.to_plain().chars().count();
            if consumed == 0 {
                None
            } else {
                *remaining = (*remaining).saturating_sub(consumed);
                Some(RichTextElement::Text(visible))
            }
        }
        RichTextElement::Link { label, url } => {
            if *remaining == 0 {
                return None;
            }
            let visible = truncate_styled(label, *remaining);
            let consumed = visible.to_plain().chars().count();
            if consumed == 0 {
                None
            } else {
                *remaining = (*remaining).saturating_sub(consumed);
                Some(RichTextElement::Link {
                    label: visible,
                    url: url.clone(),
                })
            }
        }
        RichTextElement::Group { elements, inline } => {
            let mut kept = Vec::new();
            for child in elements {
                if *remaining == 0 {
                    break;
                }
                if let Some(next) = truncate_rich_text_element(child, remaining) {
                    kept.push(next);
                }
            }
            if kept.is_empty() {
                None
            } else {
                Some(RichTextElement::Group {
                    elements: kept,
                    inline: *inline,
                })
            }
        }
        RichTextElement::List {
            items,
            ordered,
            start,
        } => {
            let mut kept = Vec::new();
            for item in items {
                if *remaining == 0 {
                    break;
                }
                if let Some(next) = truncate_rich_text_element(item, remaining) {
                    kept.push(next);
                }
            }
            if kept.is_empty() {
                None
            } else {
                Some(RichTextElement::List {
                    items: kept,
                    ordered: *ordered,
                    start: *start,
                })
            }
        }
        RichTextElement::Quote { content } => {
            let mut kept = Vec::new();
            for item in content {
                if *remaining == 0 {
                    break;
                }
                if let Some(next) = truncate_rich_text_element(item, remaining) {
                    kept.push(next);
                }
            }
            if kept.is_empty() {
                None
            } else {
                Some(RichTextElement::Quote { content: kept })
            }
        }
        // For non-textual structures we only show them after at least one text character
        // from this typewriter window has been revealed.
        RichTextElement::Divider
        | RichTextElement::Code { .. }
        | RichTextElement::Image { .. }
        | RichTextElement::Table { .. } => (*remaining > 0).then_some(element.clone()),
    }
}

fn truncate_styled(styled: &StyledStr, max_chars: usize) -> StyledStr {
    if max_chars == 0 {
        return StyledStr::empty();
    }

    let mut out = StyledStr::empty();
    let mut remaining = max_chars;
    for (chunk, style) in styled.clone().into_chunks() {
        if remaining == 0 {
            break;
        }
        let mut visible = String::new();
        for ch in chunk.chars().take(remaining) {
            visible.push(ch);
        }
        let taken = visible.chars().count();
        if taken > 0 {
            out.push(visible, style);
            remaining -= taken;
        }
    }

    out
}

fn flow_code_view(language: Language, code: &str) -> impl View {
    let highlighted = highlight_code_with_tree_sitter(language, code);
    VStack::new(
        HorizontalAlignment::Leading,
        6.0,
        (text("Code").caption(), text(highlighted)),
    )
    .padding()
    .background(Srgb::new_u8(23, 26, 30))
}

fn highlight_code_with_tree_sitter(language: Language, code: &str) -> StyledStr {
    let Some(ts_language) = language_to_tree_sitter(language) else {
        return code_monospace_plain(code);
    };

    let mut parser = Parser::new();
    if parser.set_language(&ts_language).is_err() {
        return code_monospace_plain(code);
    }

    let Some(tree) = parser.parse(code, None) else {
        return code_monospace_plain(code);
    };

    let mut tokens = Vec::new();
    collect_leaf_tokens(tree.root_node(), &mut tokens);
    tokens.sort_by_key(|(start, _, _)| *start);

    let mut styled = StyledStr::empty();
    let mut cursor = 0usize;
    for (start, end, kind) in tokens {
        if start > cursor
            && let Some(fragment) = code.get(cursor..start)
        {
            styled.push(fragment.to_string(), code_base_style());
        }
        if end > start
            && let Some(fragment) = code.get(start..end)
        {
            styled.push(fragment.to_string(), style_for_token_kind(kind));
            cursor = end;
        }
    }

    if cursor < code.len()
        && let Some(fragment) = code.get(cursor..)
    {
        styled.push(fragment.to_string(), code_base_style());
    }

    if styled.is_empty() {
        code_monospace_plain(code)
    } else {
        styled
    }
}

fn collect_leaf_tokens(node: tree_sitter::Node<'_>, out: &mut Vec<(usize, usize, String)>) {
    if node.child_count() == 0 {
        out.push((node.start_byte(), node.end_byte(), node.kind().to_string()));
        return;
    }

    let child_count = node.child_count();
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            collect_leaf_tokens(child, out);
        }
    }
}

fn code_monospace_plain(code: &str) -> StyledStr {
    let mut styled = StyledStr::empty();
    styled.push(code.to_string(), code_base_style());
    styled
}

fn code_base_style() -> Style {
    Style::default()
        .font(waterui_text::font::Font::from(waterui_text::font::Body).family("monospace"))
        .foreground(Srgb::new_u8(224, 232, 240))
}

fn style_for_token_kind(kind: String) -> Style {
    let base = code_base_style();
    if kind.contains("comment") {
        base.foreground(Srgb::new_u8(141, 153, 168))
    } else if kind.contains("string") {
        base.foreground(Srgb::new_u8(152, 195, 121))
    } else if kind.contains("number") {
        base.foreground(Srgb::new_u8(209, 154, 102))
    } else if kind.contains("keyword")
        || kind.contains("operator")
        || kind.contains("modifier")
        || kind.contains("type")
    {
        base.foreground(Srgb::new_u8(97, 175, 239))
    } else {
        base
    }
}

fn language_to_tree_sitter(language: Language) -> Option<tree_sitter::Language> {
    match language {
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        Language::Javascript => Some(tree_sitter_javascript::LANGUAGE.into()),
        Language::Typescript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::Go => Some(tree_sitter_go::LANGUAGE.into()),
        Language::Java => Some(tree_sitter_java::LANGUAGE.into()),
        Language::Swift => Some(tree_sitter_swift::LANGUAGE.into()),
        Language::Json => Some(tree_sitter_json::LANGUAGE.into()),
        Language::Bash => Some(tree_sitter_bash::LANGUAGE.into()),
        Language::Sql => Some(tree_sitter_sequel::LANGUAGE.into()),
        _ => None,
    }
}

fn is_incomplete_table(markdown: &str) -> bool {
    let lines: Vec<&str> = markdown
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.len() < 2 {
        return false;
    }
    if !lines[0].contains('|') {
        return false;
    }
    let separator = lines[1];
    if !(separator.contains('-') && separator.contains('|')) {
        return true;
    }
    lines.iter().any(|line| line.matches('|').count() < 2)
}

fn is_incomplete_image_fragment(markdown: &str) -> bool {
    if !markdown.contains("![") {
        return false;
    }
    let Some(start) = markdown.rfind("![") else {
        return false;
    };
    !markdown[start..].contains("](") || !markdown[start..].contains(')')
}

fn extract_image_alt_or_placeholder(markdown: &str) -> String {
    let Some(start) = markdown.rfind("![") else {
        return "Loading image...".to_string();
    };
    let remaining = &markdown[start + 2..];
    let alt = remaining.split_once(']').map_or("", |(alt, _)| alt.trim());
    if alt.is_empty() {
        "Loading image...".to_string()
    } else {
        format!("Image: {alt}")
    }
}

fn sanitize_pending_text_fragment(text: &str) -> String {
    if text.contains('|') {
        "Streaming table...".to_string()
    } else {
        text.to_string()
    }
}

fn ranges_overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_styled_honors_char_budget() {
        let mut styled = StyledStr::empty();
        styled.push("hello ", Style::default());
        styled.push("world", Style::default().bold());

        let truncated = truncate_styled(&styled, 7);
        assert_eq!(truncated.to_plain().as_str(), "hello w");
        let chunks = truncated.into_chunks();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].0.as_str(), "hello ");
        assert_eq!(chunks[1].0.as_str(), "w");
    }

    #[test]
    fn truncate_element_handles_nested_group() {
        let element = RichTextElement::Group {
            inline: true,
            elements: vec![
                RichTextElement::Text(StyledStr::plain("abc")),
                RichTextElement::Text(StyledStr::plain("def")),
            ],
        };
        let mut remaining = 4;
        let truncated = truncate_rich_text_element(&element, &mut remaining)
            .expect("group should keep visible content");
        assert_eq!(remaining, 0);
        assert_eq!(rich_text_element_text_len(&truncated), 4);
    }

    #[test]
    fn override_policy_takes_precedence() {
        let mut config = FlowMarkdownConfig::default();
        config
            .overrides
            .insert(FlowElementKind::Text, FlowAnimationPolicy::None);
        let policy = animation_policy_for_kind(&config, FlowElementKind::Text);
        assert!(matches!(policy, FlowAnimationPolicy::None));
    }
}
