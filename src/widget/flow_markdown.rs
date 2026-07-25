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
    collection::List as ReactiveList,
    signal::IntoComputed,
    watcher::{Context, Metadata as WatcherMetadata},
};
use native_executor::sleep;
use tree_sitter::{InputEdit, Parser, Point, Tree};
use waterui_core::{
    AnyView, Metadata, Retain, View,
    dynamic::{Dynamic, DynamicHandler},
    id::Identifiable,
};
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
    config: Computed<FlowMarkdownConfig>,
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
            config: Computed::constant(FlowMarkdownConfig::default()),
        }
    }

    /// Sets the dynamic configuration for this markdown view.
    #[must_use]
    pub fn configuration(mut self, config: impl IntoComputed<FlowMarkdownConfig>) -> Self {
        self.config = config.into_computed();
        self
    }

    /// Sets the animation preset.
    #[must_use]
    pub fn preset(mut self, preset: FlowAnimationPreset) -> Self {
        let config = nami::SignalExt::map(&self.config, move |mut config| {
            config.preset = preset;
            config
        });
        self.config = nami::SignalExt::computed(&config);
        self
    }

    /// Overrides animation policy for a specific element kind.
    #[must_use]
    pub fn override_animation(
        mut self,
        kind: FlowElementKind,
        policy: FlowAnimationPolicy,
    ) -> Self {
        let config = nami::SignalExt::map(&self.config, move |mut config| {
            config.overrides.insert(kind, policy.clone());
            config
        });
        self.config = nami::SignalExt::computed(&config);
        self
    }

    /// Sets stream mode.
    #[must_use]
    pub fn stream(mut self, mode: FlowStreamMode) -> Self {
        let config = nami::SignalExt::map(&self.config, move |mut config| {
            config.stream_mode = mode;
            config
        });
        self.config = nami::SignalExt::computed(&config);
        self
    }

    /// Sets max pending bytes kept for partial parsing.
    #[must_use]
    pub fn max_pending_bytes(mut self, bytes: usize) -> Self {
        let config = nami::SignalExt::map(&self.config, move |mut config| {
            config.max_pending_bytes = bytes.max(256);
            config
        });
        self.config = nami::SignalExt::computed(&config);
        self
    }

    /// Sets table rendering policy for incomplete streamed tables.
    #[must_use]
    pub fn table_policy(mut self, policy: FlowTablePolicy) -> Self {
        let config = nami::SignalExt::map(&self.config, move |mut config| {
            config.table_policy = policy;
            config
        });
        self.config = nami::SignalExt::computed(&config);
        self
    }

    /// Sets fade animation for newly revealed typewriter token batches.
    ///
    /// Use `None` to disable token fade-in.
    #[must_use]
    pub fn token_fade_in(mut self, animation: Option<Animation>) -> Self {
        let config = nami::SignalExt::map(&self.config, move |mut config| {
            config.typewriter_token_fade_in.clone_from(&animation);
            config
        });
        self.config = nami::SignalExt::computed(&config);
        self
    }
}

impl View for FlowMarkdown {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let Self { source, config } = self;
        let blocks = ReactiveList::new();
        let state = Rc::new(RefCell::new(FlowMarkdownState::new(
            config.get(),
            blocks.clone(),
        )));

        let initial = source.get();
        let initial_update = state
            .borrow_mut()
            .recompute(initial.as_str(), WatcherMetadata::new());
        spawn_typewriter_reveal_if_needed(&state, initial_update.typewriter);

        let guard_source = source.clone();
        let guard_config = config.clone();
        let source_guard = source.watch({
            let state = Rc::clone(&state);
            move |ctx: Context<Str>| {
                let metadata = ctx.metadata().clone();
                let markdown = ctx.into_value();
                let update = state.borrow_mut().recompute(markdown.as_str(), metadata);
                spawn_typewriter_reveal_if_needed(&state, update.typewriter);
            }
        });
        let config_guard = config.watch({
            let state = Rc::clone(&state);
            move |ctx: Context<FlowMarkdownConfig>| {
                let metadata = ctx.metadata().clone();
                let update = state.borrow_mut().reconfigure(ctx.into_value(), metadata);
                spawn_typewriter_reveal_if_needed(&state, update.typewriter);
            }
        });

        let content = VStack::for_each(blocks, |block: FlowBlockSlot| block.dynamic)
            .alignment(HorizontalAlignment::Leading);
        Metadata::new(
            content,
            Retain::new((
                source_guard,
                config_guard,
                guard_source,
                guard_config,
                state,
            )),
        )
    }
}

fn spawn_typewriter_reveal_if_needed(
    state: &Rc<RefCell<FlowMarkdownState>>,
    run: Option<TypewriterRun>,
) {
    let Some(run) = run else {
        return;
    };

    let state = Rc::clone(state);
    spawn_local(async move {
        loop {
            sleep(Duration::from_millis(run.batch_ms)).await;
            let advanced = {
                let mut state = state.borrow_mut();
                state.advance_typewriter(run.revision, run.batch_chars, run.token_fade_in.clone())
            };
            if !advanced {
                return;
            }
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
    typewriter_token_fade_in: Option<Animation>,
}

impl Default for FlowMarkdownConfig {
    fn default() -> Self {
        Self {
            preset: FlowAnimationPreset::AssistantDefault,
            overrides: HashMap::new(),
            stream_mode: FlowStreamMode::AppendOnly,
            max_pending_bytes: 32 * 1024,
            table_policy: FlowTablePolicy::NoAnimationReadablePending,
            typewriter_token_fade_in: None,
        }
    }
}

impl FlowMarkdownConfig {
    /// Sets the animation preset.
    #[must_use]
    pub const fn preset(mut self, preset: FlowAnimationPreset) -> Self {
        self.preset = preset;
        self
    }

    /// Overrides animation policy for a specific element kind.
    #[must_use]
    pub fn override_animation(
        mut self,
        kind: FlowElementKind,
        policy: FlowAnimationPolicy,
    ) -> Self {
        self.overrides.insert(kind, policy);
        self
    }

    /// Sets stream mode.
    #[must_use]
    pub const fn stream(mut self, mode: FlowStreamMode) -> Self {
        self.stream_mode = mode;
        self
    }

    /// Sets max pending bytes kept for partial parsing.
    #[must_use]
    pub const fn max_pending_bytes(mut self, bytes: usize) -> Self {
        self.max_pending_bytes = if bytes > 256 { bytes } else { 256 };
        self
    }

    /// Sets table rendering policy for incomplete streamed tables.
    #[must_use]
    pub const fn table_policy(mut self, policy: FlowTablePolicy) -> Self {
        self.table_policy = policy;
        self
    }

    /// Sets fade animation for newly revealed typewriter token batches.
    ///
    /// Use `None` to disable token fade-in.
    #[must_use]
    pub const fn token_fade_in(mut self, animation: Option<Animation>) -> Self {
        self.typewriter_token_fade_in = animation;
        self
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

const INCOMPLETE_LINK_SENTINEL: &str = "flowmarkdown:incomplete-link";

/// Animation policy for flow updates.
#[derive(Debug, Clone)]
pub enum FlowAnimationPolicy {
    /// No animation.
    None,
    /// Fade/cross-dissolve using `WaterUI` animation metadata.
    Fade(Animation),
    /// Typewriter-style progressive reveal for text-like content.
    Typewriter {
        /// Characters revealed per second.
        cps: u32,
        /// Milliseconds per reveal batch.
        batch_ms: u64,
        /// Optional fade animation applied when this typewriter batch enters.
        fade_in: Option<Animation>,
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
    identity: u64,
    range: Range<usize>,
    kind: FlowElementKind,
    elements: Vec<RichTextElement>,
}

#[derive(Debug)]
struct FlowUpdate {
    typewriter: Option<TypewriterRun>,
}

#[derive(Clone)]
struct FlowBlockSlot {
    identity: u64,
    handler: DynamicHandler,
    dynamic: Dynamic,
}

impl Identifiable for FlowBlockSlot {
    type Id = u64;

    fn id(&self) -> Self::Id {
        self.identity
    }
}

struct FlowMarkdownState {
    config: FlowMarkdownConfig,
    parser: Parser,
    tree: Option<Tree>,
    source: String,
    source_end_point: Point,
    blocks: Vec<FlowBlock>,
    slots: ReactiveList<FlowBlockSlot>,
    rendered_blocks: HashMap<u64, (usize, u64)>,
    next_block_identity: u64,
    configuration_revision: u64,
    typewriter_visible_chars: usize,
    typewriter_target_chars: usize,
    typewriter_revision: u64,
}

#[derive(Debug, Clone)]
struct TypewriterRun {
    revision: u64,
    batch_chars: usize,
    batch_ms: u64,
    token_fade_in: Option<Animation>,
}

impl FlowMarkdownState {
    fn new(config: FlowMarkdownConfig, slots: ReactiveList<FlowBlockSlot>) -> Self {
        Self {
            config,
            parser: init_markdown_parser(),
            tree: None,
            source: String::new(),
            source_end_point: Point { row: 0, column: 0 },
            blocks: Vec::new(),
            slots,
            rendered_blocks: HashMap::new(),
            next_block_identity: 1,
            configuration_revision: 0,
            typewriter_visible_chars: 0,
            typewriter_target_chars: 0,
            typewriter_revision: 0,
        }
    }

    fn recompute(&mut self, markdown: &str, upstream_metadata: WatcherMetadata) -> FlowUpdate {
        let previous_len = self.source.len();
        let is_append_only = markdown.starts_with(&self.source);
        let appended = is_append_only && markdown.len() > previous_len;
        let should_incremental = self.config.stream_mode == FlowStreamMode::AppendOnly
            && is_append_only
            && !self.source.is_empty();

        let (next_tree, next_source_end_point) = if should_incremental {
            let mut previous_tree = self
                .tree
                .take()
                .expect("FlowMarkdown incremental parse requires cached previous tree");
            let edit = build_append_input_edit(previous_len, self.source_end_point, markdown);
            previous_tree.edit(&edit);
            let next_tree = self
                .parser
                .parse(markdown, Some(&previous_tree))
                .expect("FlowMarkdown incremental parse returned no syntax tree");
            (next_tree, edit.new_end_position)
        } else {
            let next_tree = self
                .parser
                .parse(markdown, None)
                .expect("FlowMarkdown full parse returned no syntax tree");
            (next_tree, text_end_point(markdown))
        };

        let mut changed_kinds = HashSet::new();
        let mut blocks = Vec::new();
        let ranges = collect_block_ranges(&next_tree, markdown.len());
        let previous_map: HashMap<(usize, usize), FlowBlock> = self
            .blocks
            .iter()
            .cloned()
            .map(|b| ((b.range.start, b.range.end), b))
            .collect();

        for block in ranges {
            let key = (block.range.start, block.range.end);
            if should_incremental && let Some(old) = previous_map.get(&key) {
                let previous_source = self
                    .source
                    .as_str()
                    .get(old.range.clone())
                    .expect("FlowMarkdown cached block range must be a valid UTF-8 slice");
                let current_source = markdown
                    .get(block.range.clone())
                    .expect("FlowMarkdown parsed block range must be a valid UTF-8 slice");
                if previous_source == current_source {
                    blocks.push(old.clone());
                    continue;
                }
            }

            let mut parsed = parse_block(
                markdown,
                &block,
                self.config.table_policy,
                self.config.max_pending_bytes,
            );
            parsed.identity = self.allocate_block_identity();
            changed_kinds.insert(parsed.kind);
            blocks.push(parsed);
        }

        if blocks.is_empty() && !markdown.is_empty() {
            let mut fallback = parse_fallback(markdown);
            fallback.identity = self.allocate_block_identity();
            changed_kinds.insert(FlowElementKind::Text);
            blocks.push(fallback);
        }

        self.source.clear();
        self.source.push_str(markdown);
        self.source_end_point = next_source_end_point;
        self.blocks = blocks;
        self.tree = Some(next_tree);

        self.typewriter_revision = self.typewriter_revision.wrapping_add(1);

        let full_typewriter_chars = self.total_typewriter_char_count(&self.blocks);
        let typewriter = if should_incremental && appended {
            self.resolve_typewriter_run(&changed_kinds).and_then(
                |(batch_chars, batch_ms, token_fade_in)| {
                    let visible = self.typewriter_visible_chars.min(full_typewriter_chars);
                    self.typewriter_visible_chars = visible;
                    self.typewriter_target_chars = full_typewriter_chars;
                    (visible < full_typewriter_chars).then_some(TypewriterRun {
                        revision: self.typewriter_revision,
                        batch_chars,
                        batch_ms,
                        token_fade_in,
                    })
                },
            )
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
        self.sync_current_view(metadata);
        FlowUpdate { typewriter }
    }

    fn advance_typewriter(
        &mut self,
        revision: u64,
        batch_chars: usize,
        token_fade_in: Option<Animation>,
    ) -> bool {
        if revision != self.typewriter_revision
            || self.typewriter_visible_chars >= self.typewriter_target_chars
        {
            return false;
        }

        self.typewriter_visible_chars =
            (self.typewriter_visible_chars + batch_chars.max(1)).min(self.typewriter_target_chars);
        let metadata = token_fade_in.map_or_else(WatcherMetadata::new, |animation| {
            WatcherMetadata::new().with(animation)
        });
        self.sync_current_view(metadata);
        true
    }

    fn reconfigure(
        &mut self,
        config: FlowMarkdownConfig,
        upstream_metadata: WatcherMetadata,
    ) -> FlowUpdate {
        self.config = config;
        self.configuration_revision = self.configuration_revision.wrapping_add(1);
        self.typewriter_revision = self.typewriter_revision.wrapping_add(1);
        let full_typewriter_chars = self.total_typewriter_char_count(&self.blocks);
        self.typewriter_visible_chars = self.typewriter_visible_chars.min(full_typewriter_chars);
        self.typewriter_target_chars = full_typewriter_chars;
        self.sync_current_view(upstream_metadata);
        FlowUpdate { typewriter: None }
    }

    const fn allocate_block_identity(&mut self) -> u64 {
        let identity = self.next_block_identity;
        self.next_block_identity = self
            .next_block_identity
            .checked_add(1)
            .expect("FlowMarkdown block identity exhausted");
        identity
    }

    fn sync_current_view(&mut self, metadata: WatcherMetadata) {
        let budget = (self.typewriter_visible_chars < self.typewriter_target_chars)
            .then_some(self.typewriter_visible_chars);
        let mut remaining = budget.unwrap_or(usize::MAX);
        let enforce_budget = budget.is_some();
        let previous_slots = self.slots.snapshot();
        let existing_slots: HashMap<u64, FlowBlockSlot> = previous_slots
            .iter()
            .cloned()
            .map(|slot| (slot.identity, slot))
            .collect();
        let mut next_slots = Vec::with_capacity(self.blocks.len());
        let mut next_rendered = HashMap::with_capacity(self.blocks.len());

        for block in &self.blocks {
            let block_chars = block
                .elements
                .iter()
                .map(rich_text_element_text_len)
                .sum::<usize>();
            let consumes_budget = enforce_budget && self.has_typewriter_policy(block.kind);
            let render_budget = if consumes_budget {
                remaining.min(block_chars)
            } else {
                usize::MAX
            };
            let next_remaining = if consumes_budget {
                remaining.saturating_sub(block_chars)
            } else {
                remaining
            };
            let render_key = (render_budget, self.configuration_revision);

            let slot = existing_slots
                .get(&block.identity)
                .cloned()
                .unwrap_or_else(|| {
                    let (handler, dynamic) = Dynamic::new();
                    FlowBlockSlot {
                        identity: block.identity,
                        handler,
                        dynamic,
                    }
                });
            if self.rendered_blocks.get(&block.identity) != Some(&render_key) {
                let mut render_remaining = remaining;
                let view = build_flow_block_view(
                    block,
                    &mut render_remaining,
                    enforce_budget,
                    &self.config,
                );
                debug_assert_eq!(render_remaining, next_remaining);
                slot.handler.set_with_metadata(view, metadata.clone());
            }
            remaining = next_remaining;
            next_rendered.insert(block.identity, render_key);
            next_slots.push(slot);
        }

        let membership_changed = previous_slots.len() != next_slots.len()
            || previous_slots
                .iter()
                .zip(&next_slots)
                .any(|(previous, next)| previous.identity != next.identity);
        if membership_changed {
            let _previous = self.slots.replace_with_metadata(next_slots, metadata);
        }
        self.rendered_blocks = next_rendered;
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
    ) -> Option<(usize, u64, Option<Animation>)> {
        for kind in FLOW_KIND_PRIORITY {
            if !changed_kinds.contains(&kind) {
                continue;
            }
            if let FlowAnimationPolicy::Typewriter {
                cps,
                batch_ms,
                fade_in,
            } = self.animation_policy(kind)
            {
                let batch_ms = batch_ms.max(1);
                let batch_chars = ((u64::from(cps.max(1)) * batch_ms) / 1000).max(1) as usize;
                return Some((batch_chars, batch_ms, fade_in));
            }
        }
        None
    }

    fn has_typewriter_policy(&self, kind: FlowElementKind) -> bool {
        matches!(
            self.animation_policy(kind),
            FlowAnimationPolicy::Typewriter { .. }
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
            if let FlowAnimationPolicy::Fade(animation) = self.animation_policy(kind) {
                metadata = metadata.with(animation);
                break;
            }
        }
        metadata
    }

    fn animation_policy(&self, kind: FlowElementKind) -> FlowAnimationPolicy {
        animation_policy_for_kind(&self.config, kind)
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
                fade_in: config.typewriter_token_fade_in.clone(),
            },
            FlowElementKind::CodeBlock | FlowElementKind::Table | FlowElementKind::Hr => {
                FlowAnimationPolicy::None
            }
            FlowElementKind::Image => {
                FlowAnimationPolicy::Fade(Animation::ease_in_out(Duration::from_millis(180)))
            }
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

fn init_markdown_parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_md::LANGUAGE.into())
        .expect("FlowMarkdown failed to load tree-sitter markdown grammar");
    parser
}

fn collect_block_ranges(tree: &Tree, text_len: usize) -> Vec<BlockRange> {
    let mut blocks = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
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
        slice.truncate(utf8_boundary_at_or_before(&slice, max_pending_bytes));
    }

    let elements = if block.kind == FlowElementKind::Table && is_incomplete_table(&slice) {
        match table_policy {
            FlowTablePolicy::NoAnimationReadablePending => {
                vec![RichTextElement::Text(StyledStr::plain(
                    "Streaming table...".to_string(),
                ))]
            }
        }
    } else if block.kind == FlowElementKind::Image && is_incomplete_image_fragment(&slice) {
        vec![RichTextElement::Text(StyledStr::plain(
            extract_image_alt_or_placeholder(&slice),
        ))]
    } else {
        let completed = complete_incomplete_markdown_fragment(&slice);
        let rich = RichText::from_markdown(&completed);
        let parsed = normalize_incomplete_link_elements(rich.elements().to_vec());
        if parsed.is_empty() {
            vec![RichTextElement::Text(StyledStr::plain(
                sanitize_pending_text_fragment(&slice),
            ))]
        } else {
            parsed
        }
    };

    let kind = infer_block_kind(block.kind, &elements);

    FlowBlock {
        identity: 0,
        range: block.range.clone(),
        kind,
        elements,
    }
}

const fn utf8_boundary_at_or_before(value: &str, max_len: usize) -> usize {
    if max_len >= value.len() {
        return value.len();
    }

    let mut boundary = max_len;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FenceMarker {
    marker: char,
    len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LinkCandidate {
    label_end: Option<usize>,
    destination_start: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineCompletionToken {
    Backticks(usize),
    Delimiter { marker: char, len: usize },
}

fn complete_incomplete_markdown_fragment(markdown: &str) -> String {
    let fenced = close_unterminated_fence(markdown);
    let inline_closed = close_unterminated_inline_markers(&fenced);
    close_unterminated_link(&inline_closed)
}

fn close_unterminated_fence(markdown: &str) -> String {
    let (_, open_fence) = scan_fenced_ranges(markdown);
    let Some(open_fence) = open_fence else {
        return markdown.to_string();
    };

    let mut completed = String::with_capacity(markdown.len() + open_fence.len + 1);
    completed.push_str(markdown);
    if !completed.ends_with('\n') {
        completed.push('\n');
    }
    for _ in 0..open_fence.len {
        completed.push(open_fence.marker);
    }
    completed
}

fn close_unterminated_inline_markers(markdown: &str) -> String {
    let (fence_ranges, _) = scan_fenced_ranges(markdown);
    let mut fence_cursor = 0usize;
    let mut escaped_next = false;
    let mut stack: Vec<InlineCompletionToken> = Vec::new();
    let mut chars = markdown.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if byte_in_ranges(idx, &fence_ranges, &mut fence_cursor) {
            continue;
        }
        if escaped_next {
            escaped_next = false;
            continue;
        }

        if ch == '\\' {
            if !matches!(stack.last(), Some(InlineCompletionToken::Backticks(_))) {
                escaped_next = true;
            }
            continue;
        }

        if ch == '`' {
            let run = consume_repeated_marker(ch, &mut chars, &fence_ranges, &mut fence_cursor);
            match stack.last().copied() {
                Some(InlineCompletionToken::Backticks(open_len)) if open_len == run => {
                    stack.pop();
                }
                Some(InlineCompletionToken::Backticks(_)) => {}
                _ => stack.push(InlineCompletionToken::Backticks(run)),
            }
            continue;
        }

        if matches!(stack.last(), Some(InlineCompletionToken::Backticks(_))) {
            continue;
        }

        if ch == '~' {
            let run = consume_repeated_marker(ch, &mut chars, &fence_ranges, &mut fence_cursor);
            for _ in 0..(run / 2) {
                toggle_inline_token(
                    &mut stack,
                    InlineCompletionToken::Delimiter {
                        marker: '~',
                        len: 2,
                    },
                );
            }
            continue;
        }

        if ch == '*' || ch == '_' {
            let run = consume_repeated_marker(ch, &mut chars, &fence_ranges, &mut fence_cursor);
            if ch == '_' && is_intraword_underscore(markdown, idx, run) {
                continue;
            }

            for len in decompose_emphasis_run(run) {
                toggle_inline_token(
                    &mut stack,
                    InlineCompletionToken::Delimiter { marker: ch, len },
                );
            }
        }
    }

    if stack.is_empty() {
        return markdown.to_string();
    }

    let mut completed = String::with_capacity(markdown.len() + 8);
    completed.push_str(markdown);
    for token in stack.iter().rev() {
        match *token {
            InlineCompletionToken::Backticks(len) => {
                for _ in 0..len {
                    completed.push('`');
                }
            }
            InlineCompletionToken::Delimiter { marker, len } => {
                for _ in 0..len {
                    completed.push(marker);
                }
            }
        }
    }
    completed
}

fn close_unterminated_link(markdown: &str) -> String {
    let (fence_ranges, _) = scan_fenced_ranges(markdown);
    let mut fence_cursor = 0usize;
    let mut escaped_next = false;
    let mut inline_code_backticks: Option<usize> = None;
    let mut links = Vec::new();
    let bytes = markdown.as_bytes();
    let mut chars = markdown.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if byte_in_ranges(idx, &fence_ranges, &mut fence_cursor) {
            continue;
        }
        if escaped_next {
            escaped_next = false;
            continue;
        }

        if ch == '\\' {
            if inline_code_backticks.is_none() {
                escaped_next = true;
            }
            continue;
        }

        if ch == '`' {
            let run = consume_repeated_marker(ch, &mut chars, &fence_ranges, &mut fence_cursor);
            match inline_code_backticks {
                Some(open) if open == run => inline_code_backticks = None,
                Some(_) => {}
                None => inline_code_backticks = Some(run),
            }
            continue;
        }

        if inline_code_backticks.is_some() {
            continue;
        }

        match ch {
            '[' => {
                let is_image = idx > 0 && bytes[idx - 1] == b'!';
                if !is_image {
                    links.push(LinkCandidate {
                        label_end: None,
                        destination_start: None,
                    });
                }
            }
            ']' => {
                if let Some(last) = links.last_mut()
                    && last.label_end.is_none()
                {
                    last.label_end = Some(idx + ch.len_utf8());
                }
            }
            '(' => {
                if let Some(last) = links.last_mut()
                    && last.label_end == Some(idx)
                {
                    last.destination_start = Some(idx + ch.len_utf8());
                }
            }
            ')' => {
                if let Some(last) = links.last()
                    && last.destination_start.is_some()
                {
                    links.pop();
                }
            }
            _ => {}
        }
    }

    let Some(candidate) = links
        .iter()
        .rev()
        .find(|candidate| candidate.label_end.is_none() || candidate.destination_start.is_some())
        .copied()
    else {
        return markdown.to_string();
    };

    if candidate.label_end.is_none() {
        let mut completed =
            String::with_capacity(markdown.len() + INCOMPLETE_LINK_SENTINEL.len() + 4);
        completed.push_str(markdown);
        completed.push_str("](");
        completed.push_str(INCOMPLETE_LINK_SENTINEL);
        completed.push(')');
        return completed;
    }

    if let Some(destination_start) = candidate.destination_start {
        let mut completed =
            String::with_capacity(markdown.len() + INCOMPLETE_LINK_SENTINEL.len() + 1);
        completed.push_str(
            markdown
                .get(..destination_start)
                .expect("link destination start should be on UTF-8 boundary"),
        );
        completed.push_str(INCOMPLETE_LINK_SENTINEL);
        completed.push(')');
        return completed;
    }

    markdown.to_string()
}

fn normalize_incomplete_link_elements(elements: Vec<RichTextElement>) -> Vec<RichTextElement> {
    elements
        .into_iter()
        .map(normalize_incomplete_link_element)
        .collect()
}

fn normalize_incomplete_link_element(element: RichTextElement) -> RichTextElement {
    match element {
        RichTextElement::Link { label, url } if url.as_str() == INCOMPLETE_LINK_SENTINEL => {
            RichTextElement::Text(label)
        }
        RichTextElement::Group { elements, inline } => RichTextElement::Group {
            elements: normalize_incomplete_link_elements(elements),
            inline,
        },
        RichTextElement::List {
            items,
            ordered,
            start,
        } => RichTextElement::List {
            items: normalize_incomplete_link_elements(items),
            ordered,
            start,
        },
        RichTextElement::Quote { content } => RichTextElement::Quote {
            content: normalize_incomplete_link_elements(content),
        },
        RichTextElement::Table {
            headers,
            rows,
            alignments,
        } => RichTextElement::Table {
            headers: normalize_incomplete_link_elements(headers),
            rows: rows
                .into_iter()
                .map(normalize_incomplete_link_elements)
                .collect(),
            alignments,
        },
        other => other,
    }
}

fn toggle_inline_token(stack: &mut Vec<InlineCompletionToken>, token: InlineCompletionToken) {
    if stack.last().copied() == Some(token) {
        stack.pop();
    } else {
        stack.push(token);
    }
}

fn decompose_emphasis_run(run: usize) -> Vec<usize> {
    let mut remaining = run;
    let mut chunks = Vec::new();
    while remaining >= 2 {
        chunks.push(2);
        remaining -= 2;
    }
    if remaining == 1 {
        chunks.push(1);
    }
    chunks
}

fn consume_repeated_marker(
    marker: char,
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    fence_ranges: &[Range<usize>],
    fence_cursor: &mut usize,
) -> usize {
    let mut run = 1usize;
    while let Some((next_idx, next_ch)) = chars.peek().copied() {
        if next_ch != marker || byte_in_ranges(next_idx, fence_ranges, fence_cursor) {
            break;
        }
        chars.next();
        run += 1;
    }
    run
}

fn is_intraword_underscore(markdown: &str, start: usize, run: usize) -> bool {
    let prev = markdown[..start].chars().next_back();
    let next = markdown[start + run..].chars().next();
    is_word_char(prev) && is_word_char(next)
}

fn is_word_char(ch: Option<char>) -> bool {
    ch.is_some_and(char::is_alphanumeric)
}

fn scan_fenced_ranges(markdown: &str) -> (Vec<Range<usize>>, Option<FenceMarker>) {
    let mut ranges = Vec::new();
    let mut offset = 0usize;
    let mut open: Option<(usize, FenceMarker)> = None;

    for line_with_newline in markdown.split_inclusive('\n') {
        let line = line_with_newline
            .strip_suffix('\n')
            .unwrap_or(line_with_newline);
        if let Some((open_start, open_marker)) = open {
            if let Some((marker, len, rest)) = parse_fence_run(line)
                && marker == open_marker.marker
                && len >= open_marker.len
                && rest.trim().is_empty()
            {
                ranges.push(open_start..(offset + line_with_newline.len()));
                open = None;
            }
        } else if let Some((marker, len, _)) = parse_fence_run(line) {
            open = Some((offset, FenceMarker { marker, len }));
        }
        offset += line_with_newline.len();
    }

    if let Some((start, marker)) = open {
        ranges.push(start..markdown.len());
        return (ranges, Some(marker));
    }

    (ranges, None)
}

fn parse_fence_run(line: &str) -> Option<(char, usize, &str)> {
    let bytes = line.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() && idx < 3 && (bytes[idx] == b' ' || bytes[idx] == b'\t') {
        idx += 1;
    }

    let marker = *bytes.get(idx)?;
    if marker != b'`' && marker != b'~' {
        return None;
    }

    let mut run_len = 0usize;
    while idx + run_len < bytes.len() && bytes[idx + run_len] == marker {
        run_len += 1;
    }
    if run_len < 3 {
        return None;
    }

    Some((marker as char, run_len, &line[idx + run_len..]))
}

fn byte_in_ranges(index: usize, ranges: &[Range<usize>], cursor: &mut usize) -> bool {
    while *cursor < ranges.len() && index >= ranges[*cursor].end {
        *cursor += 1;
    }
    ranges
        .get(*cursor)
        .is_some_and(|range| index >= range.start && index < range.end)
}

fn infer_block_kind(
    default_kind: FlowElementKind,
    elements: &[RichTextElement],
) -> FlowElementKind {
    if elements
        .iter()
        .any(|element| rich_text_contains_kind(element, FlowElementKind::Image))
    {
        return FlowElementKind::Image;
    }
    if elements
        .iter()
        .any(|element| rich_text_contains_kind(element, FlowElementKind::Table))
    {
        return FlowElementKind::Table;
    }
    if elements
        .iter()
        .any(|element| rich_text_contains_kind(element, FlowElementKind::CodeBlock))
    {
        return FlowElementKind::CodeBlock;
    }
    if elements
        .iter()
        .any(|element| rich_text_contains_kind(element, FlowElementKind::ListItem))
    {
        return FlowElementKind::ListItem;
    }
    if elements
        .iter()
        .any(|element| rich_text_contains_kind(element, FlowElementKind::Quote))
    {
        return FlowElementKind::Quote;
    }
    if elements
        .iter()
        .any(|element| rich_text_contains_kind(element, FlowElementKind::Link))
    {
        return FlowElementKind::Link;
    }
    if elements
        .iter()
        .any(|element| rich_text_contains_kind(element, FlowElementKind::Hr))
    {
        return FlowElementKind::Hr;
    }
    default_kind
}

fn rich_text_contains_kind(element: &RichTextElement, kind: FlowElementKind) -> bool {
    match (element, kind) {
        (RichTextElement::Image { .. }, FlowElementKind::Image)
        | (RichTextElement::Table { .. }, FlowElementKind::Table)
        | (RichTextElement::Code { .. }, FlowElementKind::CodeBlock)
        | (RichTextElement::List { .. }, FlowElementKind::ListItem)
        | (RichTextElement::Quote { .. }, FlowElementKind::Quote)
        | (RichTextElement::Link { .. }, FlowElementKind::Link)
        | (RichTextElement::Divider, FlowElementKind::Hr) => true,
        (RichTextElement::Group { elements, .. }, _) => elements
            .iter()
            .any(|child| rich_text_contains_kind(child, kind)),
        (RichTextElement::List { items, .. }, _) => items
            .iter()
            .any(|child| rich_text_contains_kind(child, kind)),
        (RichTextElement::Quote { content }, _) => content
            .iter()
            .any(|child| rich_text_contains_kind(child, kind)),
        (RichTextElement::Table { headers, rows, .. }, _) => {
            headers
                .iter()
                .any(|child| rich_text_contains_kind(child, kind))
                || rows
                    .iter()
                    .any(|row| row.iter().any(|child| rich_text_contains_kind(child, kind)))
        }
        _ => false,
    }
}

fn parse_fallback(markdown: &str) -> FlowBlock {
    FlowBlock {
        identity: 0,
        range: 0..markdown.len(),
        kind: FlowElementKind::Text,
        elements: RichText::from_markdown(markdown).elements().to_vec(),
    }
}

fn build_flow_block_view(
    block: &FlowBlock,
    remaining: &mut usize,
    enforce_budget: bool,
    config: &FlowMarkdownConfig,
) -> AnyView {
    let mut views = Vec::new();
    if enforce_budget && has_typewriter_policy_for_kind(block.kind, config) {
        for element in &block.elements {
            if *remaining == 0 {
                break;
            }
            if let Some(truncated) = truncate_rich_text_element(element, remaining) {
                push_rich_text_element_view(&mut views, truncated);
            }
        }
    } else {
        for element in &block.elements {
            push_rich_text_element_view(&mut views, element.clone());
        }
    }

    AnyView::new(VStack::from_iter(views).alignment(HorizontalAlignment::Leading))
}

fn push_rich_text_element_view(views: &mut Vec<AnyView>, element: RichTextElement) {
    match element {
        RichTextElement::Code { code, language } => {
            views.push(AnyView::new(flow_code_view(&language, code.as_str())));
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

fn flow_code_view(language: &Language, code: &str) -> impl View {
    let highlighted = highlight_code_with_tree_sitter(language, code);
    VStack::new(
        HorizontalAlignment::Leading,
        6.0,
        (text("Code").caption(), text(highlighted)),
    )
    .padding()
    .background(Srgb::new_u8(23, 26, 30))
}

fn highlight_code_with_tree_sitter(language: &Language, code: &str) -> StyledStr {
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
            styled.push(fragment.to_string(), style_for_token_kind(&kind));
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

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_leaf_tokens(child, out);
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

fn style_for_token_kind(kind: &str) -> Style {
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

fn language_to_tree_sitter(language: &Language) -> Option<tree_sitter::Language> {
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
    let header = lines[0];
    if header.matches('|').count() < 1 {
        return false;
    }
    let separator = lines[1].trim();
    if looks_like_table_separator_row(separator) {
        return false;
    }
    looks_like_table_separator_prefix(separator)
}

fn looks_like_table_separator_prefix(line: &str) -> bool {
    !line.is_empty()
        && line.contains('-')
        && line
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

fn looks_like_table_separator_row(line: &str) -> bool {
    let mut has_segment = false;
    for segment in line.split('|').map(str::trim).filter(|seg| !seg.is_empty()) {
        has_segment = true;
        let dashes = segment.trim_matches(':');
        if dashes.is_empty() || !dashes.chars().all(|c| c == '-') {
            return false;
        }
    }
    has_segment
}

fn is_incomplete_image_fragment(markdown: &str) -> bool {
    if !markdown.contains("![") {
        return false;
    }
    let Some(start) = markdown.rfind("![") else {
        return false;
    };
    let fragment = &markdown[start..];
    let Some(open_link) = fragment.find("](") else {
        return false;
    };
    !fragment[open_link + 2..].contains(')')
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
    if is_incomplete_table(text) {
        "Streaming table...".to_string()
    } else {
        text.to_string()
    }
}

fn build_append_input_edit(previous_len: usize, previous_end: Point, next: &str) -> InputEdit {
    assert!(
        next.len() >= previous_len,
        "FlowMarkdown incremental edit requires appended text"
    );
    assert!(
        next.is_char_boundary(previous_len),
        "FlowMarkdown incremental edit boundary must be on UTF-8 boundary"
    );

    let appended = next
        .get(previous_len..)
        .expect("FlowMarkdown incremental append slice should exist");
    let new_end = advance_point(previous_end, appended);

    InputEdit {
        start_byte: previous_len,
        old_end_byte: previous_len,
        new_end_byte: next.len(),
        start_position: previous_end,
        old_end_position: previous_end,
        new_end_position: new_end,
    }
}

fn text_end_point(text: &str) -> Point {
    advance_point(Point { row: 0, column: 0 }, text)
}

fn advance_point(mut point: Point, text: &str) -> Point {
    for byte in text.bytes() {
        if byte == b'\n' {
            point.row += 1;
            point.column = 0;
        } else {
            point.column += 1;
        }
    }
    point
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

    #[test]
    fn append_input_edit_updates_utf8_multiline_positions() {
        let previous = "line1\n\u{4E2D}";
        let appended = "\n\u{03B2}eta";
        let next = format!("{previous}{appended}");
        let edit = build_append_input_edit(previous.len(), text_end_point(previous), next.as_str());

        assert_eq!(edit.start_byte, previous.len());
        assert_eq!(edit.old_end_byte, previous.len());
        assert_eq!(edit.new_end_byte, next.len());
        assert_eq!(edit.start_position, Point { row: 1, column: 3 });
        assert_eq!(edit.old_end_position, Point { row: 1, column: 3 });
        assert_eq!(edit.new_end_position, Point { row: 2, column: 5 });
    }

    #[test]
    fn tree_sitter_incremental_append_reports_tail_changed_ranges() {
        let mut parser = init_markdown_parser();
        let original = "# Title\n\nParagraph";
        let mut old_tree = parser
            .parse(original, None)
            .expect("initial markdown parse should produce a syntax tree");
        let next = format!("{original}\n\n- appended");
        let edit = build_append_input_edit(original.len(), text_end_point(original), next.as_str());
        old_tree.edit(&edit);
        let new_tree = parser
            .parse(next.as_str(), Some(&old_tree))
            .expect("incremental markdown parse should produce a syntax tree");

        let changed_ranges: Vec<Range<usize>> = old_tree
            .changed_ranges(&new_tree)
            .map(|range| range.start_byte..range.end_byte)
            .collect();
        assert!(
            !changed_ranges.is_empty(),
            "incremental append should report changed syntax ranges"
        );
        assert!(
            changed_ranges
                .iter()
                .all(|range| range.start >= original.len().saturating_sub(1)),
            "changed ranges should be localized near append boundary"
        );
    }

    #[test]
    fn typewriter_run_includes_token_fade_animation_when_enabled() {
        let config = FlowMarkdownConfig {
            typewriter_token_fade_in: Some(Animation::ease_in_out(Duration::from_millis(140))),
            ..FlowMarkdownConfig::default()
        };
        let mut state = FlowMarkdownState::new(config, ReactiveList::new());

        state.recompute("# Title", WatcherMetadata::new());

        let update = state.recompute("# Title\n\nNew streamed tokens", WatcherMetadata::new());
        let run = update
            .typewriter
            .as_ref()
            .expect("append should produce a typewriter run");
        assert!(
            run.token_fade_in.is_some(),
            "typewriter run should carry token fade-in animation"
        );
    }

    #[test]
    fn completion_closes_basic_inline_markers_for_streaming_fragments() {
        assert_eq!(complete_incomplete_markdown_fragment("**bold"), "**bold**");
        assert_eq!(complete_incomplete_markdown_fragment("*italic"), "*italic*");
        assert_eq!(complete_incomplete_markdown_fragment("`code"), "`code`");
        assert_eq!(
            complete_incomplete_markdown_fragment("~~strike"),
            "~~strike~~"
        );
    }

    #[test]
    fn completion_repairs_incomplete_links_with_placeholder_target() {
        let expected = format!("[WaterUI]({INCOMPLETE_LINK_SENTINEL})");
        assert_eq!(complete_incomplete_markdown_fragment("[WaterUI"), expected);
        assert_eq!(
            complete_incomplete_markdown_fragment("[WaterUI](https://example"),
            expected
        );
    }

    #[test]
    fn completion_preserves_intraword_underscore_sequences() {
        let markdown = "Contact john_doe@example.com about snake_case parsing.";
        assert_eq!(complete_incomplete_markdown_fragment(markdown), markdown);
    }

    #[test]
    fn completion_closes_unterminated_code_fence_without_inline_noise() {
        let markdown = concat!(
            "```rust\n",
            "fn sample() {\n",
            "    let t = \"**still-code\";\n",
            "}\n"
        );
        let completed = complete_incomplete_markdown_fragment(markdown);
        assert_eq!(completed, format!("{markdown}```"));
        assert_eq!(
            completed.matches("**").count(),
            markdown.matches("**").count()
        );
    }

    #[test]
    fn normalize_incomplete_links_rewrites_placeholder_links_to_text() {
        let elements = vec![RichTextElement::Link {
            label: StyledStr::plain("WaterUI"),
            url: Str::from_static(INCOMPLETE_LINK_SENTINEL),
        }];
        let normalized = normalize_incomplete_link_elements(elements);

        assert_eq!(normalized.len(), 1);
        match &normalized[0] {
            RichTextElement::Text(text) => assert_eq!(text.to_plain().as_str(), "WaterUI"),
            other => panic!("placeholder link should be downgraded to text, got: {other:?}"),
        }
    }

    fn stream_with_constant_chars_per_second(
        state: &mut FlowMarkdownState,
        markdown: &str,
        chars_per_second: usize,
    ) -> bool {
        assert!(
            chars_per_second > 0,
            "stream simulation requires a positive chars_per_second"
        );

        let chars: Vec<char> = markdown.chars().collect();
        let mut streamed = String::new();
        let mut offset = 0usize;
        let mut saw_typewriter = false;

        while offset < chars.len() {
            let end = (offset + chars_per_second).min(chars.len());
            streamed.extend(chars[offset..end].iter().copied());
            let update = state.recompute(streamed.as_str(), WatcherMetadata::new());
            saw_typewriter |= update.typewriter.is_some();
            offset = end;
        }

        assert_eq!(
            streamed, markdown,
            "stream simulator must rebuild the full markdown payload"
        );
        saw_typewriter
    }

    #[test]
    fn recompute_streams_complete_markdown_documents_at_constant_rate() {
        const DOCS: [&str; 3] = [
            concat!(
                "# Stream One\n\n",
                "This document simulates a complete assistant answer delivered at fixed rate.\n\n",
                "- bullet one\n- bullet two\n\n",
                "Final sentence."
            ),
            concat!(
                "## Stream Two\n\n",
                "| Key | Value |\n| --- | --- |\n| Throughput | 128 tok/s |\n| Latency | 42 ms |\n\n",
                "```rust\nfn answer() -> i32 {\n    42\n}\n```\n"
            ),
            concat!(
                "### Stream Three\n\n",
                "> Quoted context for downstream model output.\n\n",
                "1. first item\n2. second item\n\n",
                "See [WaterUI](https://waterui.dev)."
            ),
        ];

        for markdown in DOCS {
            let mut state =
                FlowMarkdownState::new(FlowMarkdownConfig::default(), ReactiveList::new());
            let saw_typewriter = stream_with_constant_chars_per_second(&mut state, markdown, 16);
            assert!(
                saw_typewriter,
                "constant-rate stream should trigger at least one typewriter run"
            );

            let settled = state.recompute(markdown, WatcherMetadata::new());
            assert!(
                settled.typewriter.is_none(),
                "idempotent recompute with full markdown should not schedule new typewriter run"
            );
        }
    }

    #[test]
    fn recompute_handles_streaming_append_sequence_with_tables_and_code() {
        let mut state = FlowMarkdownState::new(FlowMarkdownConfig::default(), ReactiveList::new());
        let mut markdown = String::new();
        let chunks = [
            "# Flow Markdown\n\nStreaming response starts here.\n\n",
            "## Highlights\n\n- Tail append updates\n- Typewriter reveal\n\n",
            "| Metric | Value |\n| --- | --- |\n| Throughput | 128 tok/s |\n",
            "| Latency | 42 ms |\n\n```rust\nfn answer() -> i32 {\n    42\n}\n```\n\n",
            "![WaterUI mark](https://waterui.dev/favicon.ico)\n\nFinal line.",
        ];

        for chunk in chunks {
            markdown.push_str(chunk);
            state.recompute(markdown.as_str(), WatcherMetadata::new());
        }

        // Continue appending at char granularity to stress streaming parse path.
        let tail = "\n\n- postscript";
        for ch in tail.chars() {
            markdown.push(ch);
            state.recompute(markdown.as_str(), WatcherMetadata::new());
        }
    }

    #[test]
    fn recompute_uses_full_parse_for_rewrites_then_recovers_incremental_appends() {
        let mut state = FlowMarkdownState::new(FlowMarkdownConfig::default(), ReactiveList::new());

        let first = state.recompute("# Title", WatcherMetadata::new());
        assert!(
            first.typewriter.is_none(),
            "initial render should not use incremental typewriter run"
        );

        let appended = state.recompute("# Title\n\nnext line", WatcherMetadata::new());
        assert!(
            appended.typewriter.is_some(),
            "append updates should use incremental typewriter run"
        );

        let rewritten = state.recompute("## Rewritten\n\nnew root", WatcherMetadata::new());
        assert!(
            rewritten.typewriter.is_none(),
            "non-append rewrites should force a full reparse"
        );

        let appended_after_rewrite =
            state.recompute("## Rewritten\n\nnew root\n\n+ tail", WatcherMetadata::new());
        assert!(
            appended_after_rewrite.typewriter.is_some(),
            "append after rewrite should recover incremental mode"
        );
    }

    #[test]
    fn incremental_append_preserves_unchanged_block_identity() {
        let slots = ReactiveList::new();
        let mut state = FlowMarkdownState::new(FlowMarkdownConfig::default(), slots.clone());
        state.recompute(
            "# Stable heading\n\n# Streaming body",
            WatcherMetadata::new(),
        );
        let before = slots
            .snapshot()
            .into_iter()
            .map(|slot| slot.identity)
            .collect::<Vec<_>>();

        let update = state.recompute(
            "# Stable heading\n\n# Streaming body grows",
            WatcherMetadata::new(),
        );
        let after_append = slots
            .snapshot()
            .into_iter()
            .map(|slot| slot.identity)
            .collect::<Vec<_>>();

        assert_eq!(before.len(), 2);
        assert_eq!(after_append.len(), 2);
        assert_eq!(before[0], after_append[0]);
        assert_ne!(before[1], after_append[1]);

        let run = update
            .typewriter
            .expect("append must start the configured typewriter update");
        assert!(state.advance_typewriter(run.revision, run.batch_chars, run.token_fade_in,));
        let after_tick = slots
            .snapshot()
            .into_iter()
            .map(|slot| slot.identity)
            .collect::<Vec<_>>();
        assert_eq!(after_append, after_tick);
    }
}
