use core::fmt;
use core::{fmt::Display, mem::take, ops::Add, ops::Range};

use crate::{
    font::{Font, FontWeight},
    text,
};
use alloc::{string::String, vec::Vec};
use core::ops::AddAssign;
use nami::signal::IntoComputed;
use nami::{Signal, impl_constant};
#[cfg(feature = "markdown")]
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};
use waterui_core::{
    Environment, Str, View,
    handler::{EventHandler, SharedAction},
};
use waterui_graphics::color::{Color, SurfaceVariantColor};

/// A set of text attributes for rich text formatting.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Style {
    /// The font to use.
    font: Font,
    /// The foreground (text) color.
    foreground: Option<Color>,
    /// The background color.
    background: Option<Color>,
    /// Whether the text is italic.
    italic: bool,
    /// Whether the text has an underline.
    underline: bool,
    /// Whether the text has a strikethrough.
    strikethrough: bool,
}

impl Style {
    /// The font to use.
    #[must_use]
    pub const fn get_font(&self) -> &Font {
        &self.font
    }

    /// The foreground (text) color.
    #[must_use]
    pub const fn get_foreground(&self) -> Option<&Color> {
        self.foreground.as_ref()
    }

    /// Sets the `foreground`.
    pub fn set_foreground(&mut self, foreground: Option<Color>) {
        self.foreground = foreground;
    }

    /// The background color.
    #[must_use]
    pub const fn get_background(&self) -> Option<&Color> {
        self.background.as_ref()
    }

    /// Whether the text is italic.
    #[must_use]
    pub const fn get_italic(&self) -> bool {
        self.italic
    }

    /// Whether the text has an underline.
    #[must_use]
    pub const fn get_underline(&self) -> bool {
        self.underline
    }

    /// Whether the text has a strikethrough.
    #[must_use]
    pub const fn get_strikethrough(&self) -> bool {
        self.strikethrough
    }
}

impl Style {
    /// Creates a new default `Style`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the font.
    #[must_use]
    pub fn font(mut self, font: impl Into<Font>) -> Self {
        self.font = font.into();
        self
    }

    /// Sets the text color.
    #[must_use]
    pub fn foreground(mut self, color: impl Into<Color>) -> Self {
        self.foreground = Some(color.into());
        self
    }

    /// Sets the background color.
    #[must_use]
    pub fn background(mut self, color: impl Into<Color>) -> Self {
        self.background = Some(color.into());
        self
    }

    /// Sets the font weight.
    #[must_use]
    pub fn weight(mut self, weight: FontWeight) -> Self {
        self.font = self.font.weight(weight);
        self
    }

    /// Asks for the platform's fixed-pitch face, keeping size and weight.
    #[must_use]
    pub fn monospaced(mut self) -> Self {
        self.font = self.font.monospaced();
        self
    }

    /// Sets the bold style.
    /// Equal to calling `self.weight(FontWeight::Bold)`.
    #[must_use]
    pub fn bold(mut self) -> Self {
        self.font = self.font.bold();
        self
    }

    /// Sets the font size in points.
    #[must_use]
    pub fn size(mut self, size: f32) -> Self {
        self.font = self.font.size(size);
        self
    }

    /// Sets the italic style.
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Disables the italic style.
    #[must_use]
    pub const fn not_italic(mut self) -> Self {
        self.italic = false;
        self
    }

    /// Sets the underline style.
    #[must_use]
    pub const fn underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// Disables the underline style.
    #[must_use]
    pub const fn not_underline(mut self) -> Self {
        self.underline = false;
        self
    }

    /// Sets the strikethrough style.
    #[must_use]
    pub const fn strikethrough(mut self) -> Self {
        self.strikethrough = true;
        self
    }

    /// Disables the strikethrough style.
    #[must_use]
    pub const fn not_strikethrough(mut self) -> Self {
        self.strikethrough = false;
        self
    }

    /// Returns `true` if this style has no text decorations or colors applied.
    ///
    /// Note: font customizations are not currently considered here.
    #[must_use]
    pub const fn is_plain(&self) -> bool {
        !self.italic
            && !self.underline
            && !self.strikethrough
            && self.foreground.is_none()
            && self.background.is_none()
    }
}

/// A tap-activated region of a [`StyledStr`].
///
/// The range is a half-open UTF-8 byte range over [`StyledStr::to_plain`];
/// construction always lands it on char boundaries, and every style edit
/// preserves it, so a span always covers whole characters. The action reads its payload at
/// activation, not construction, so a `Binding` payload reports the latest
/// value without rebuilding the text.
#[derive(Debug, Clone)]
pub struct TapSpan {
    range: Range<usize>,
    action: SharedAction,
}

impl TapSpan {
    /// The UTF-8 byte range this span covers over [`StyledStr::to_plain`].
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    /// Invokes the span's action against the given environment.
    pub fn activate(&self, env: &Environment) {
        self.action.call(env);
    }
}

/// A string with associated text attributes for rich text formatting.
#[derive(Debug, Clone, Default)]
pub struct StyledStr {
    chunks: Vec<(Str, Style)>,
    tap_spans: Vec<TapSpan>,
}

impl StyledStr {
    /// Creates a new empty `StyledStr`.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            chunks: Vec::new(),
            tap_spans: Vec::new(),
        }
    }

    /// Creates a styled string from Markdown inline semantics.
    ///
    /// Supported features include headings, emphasis, strong text,
    /// strikethrough, and inline code styling.
    #[cfg(feature = "markdown")]
    #[must_use]
    pub fn from_markdown(markdown: &str) -> Self {
        let options =
            Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES | Options::ENABLE_STRIKETHROUGH;
        let parser = Parser::new_ext(markdown, options);

        let mut builder = MarkdownInlineBuilder::new();
        let mut pending_block_break = false;

        for event in parser {
            match event {
                Event::Start(tag) => match tag {
                    Tag::Heading { level, .. } => {
                        if pending_block_break || !builder.is_empty() {
                            builder.push_text("\n\n");
                        }
                        pending_block_break = false;
                        builder.enter_with(move |_| heading_style(level));
                    }
                    Tag::Paragraph => {
                        if pending_block_break || !builder.is_empty() {
                            builder.push_text("\n\n");
                        }
                        pending_block_break = false;
                    }
                    Tag::Emphasis => builder.enter_emphasis(),
                    Tag::Strong => builder.enter_strong(),
                    Tag::Strikethrough => builder.enter_strikethrough(),
                    Tag::CodeBlock(kind) => {
                        if pending_block_break || !builder.is_empty() {
                            builder.push_text("\n\n");
                        }
                        pending_block_break = false;
                        if let CodeBlockKind::Fenced(info) = kind
                            && !info.is_empty()
                        {
                            builder.push_text(info.as_ref());
                            builder.push_text(":\n");
                        }
                    }
                    Tag::List(_) | Tag::Item => {
                        if pending_block_break || !builder.is_empty() {
                            builder.push_text("\n");
                        }
                        pending_block_break = false;
                    }
                    _ => {}
                },
                Event::End(tag) => match tag {
                    pulldown_cmark::TagEnd::Heading(_) => {
                        builder.exit();
                        pending_block_break = true;
                    }
                    pulldown_cmark::TagEnd::Paragraph
                    | pulldown_cmark::TagEnd::CodeBlock
                    | pulldown_cmark::TagEnd::List(_) => {
                        pending_block_break = true;
                    }
                    pulldown_cmark::TagEnd::Emphasis
                    | pulldown_cmark::TagEnd::Strong
                    | pulldown_cmark::TagEnd::Strikethrough => {
                        builder.exit();
                    }
                    _ => {}
                },
                Event::Text(text)
                | Event::Html(text)
                | Event::FootnoteReference(text)
                | Event::InlineMath(text)
                | Event::DisplayMath(text)
                | Event::InlineHtml(text) => {
                    if pending_block_break && !builder.is_empty() {
                        builder.push_text("\n\n");
                        pending_block_break = false;
                    }
                    builder.push_text(text.as_ref());
                }
                Event::Code(text) => {
                    if pending_block_break && !builder.is_empty() {
                        builder.push_text("\n\n");
                        pending_block_break = false;
                    }
                    builder.push_inline_code(text.as_ref());
                }
                Event::SoftBreak => builder.push_soft_break(),
                Event::HardBreak => builder.push_hard_break(),
                Event::Rule => {
                    builder.push_text("\n\n——\n\n");
                    pending_block_break = false;
                }
                Event::TaskListMarker(checked) => {
                    if pending_block_break && !builder.is_empty() {
                        builder.push_text("\n");
                        pending_block_break = false;
                    }
                    builder.push_text(if checked { "[x] " } else { "[ ] " });
                }
            }
        }

        builder.finish()
    }

    /// Creates a plain attributed string with a single unstyled chunk.
    #[must_use]
    pub fn plain(text: impl Into<Str>) -> Self {
        let mut s = Self::empty();
        s.push(text.into(), Style::default());
        s
    }

    /// Adds a new text chunk with the specified style.
    pub fn push(&mut self, text: impl Into<Str>, style: Style) {
        let text = text.into();
        self.chunks.push((text, style));
    }

    /// Appends text to the last chunk, or creates a new chunk if empty.
    ///
    /// Appended text does not inherit an earlier [`TapSpan`]: spans are byte
    /// ranges over the string as it stood when `on_tap` ran, so the new text
    /// starts past the span's end.
    pub fn push_str(&mut self, text: impl Into<Str>) {
        let text = text.into();
        if let Some(last) = self.chunks.last_mut() {
            let (last_text, _) = last;
            last_text.add_assign(text);
        } else {
            self.chunks.push((text, Style::default()));
        }
    }

    /// Returns the total length of the attributed string.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.iter().map(|(text, _)| text.len()).sum()
    }

    /// Checks if the attributed string is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The text as a reader should hear it, without bidi formatting controls.
    ///
    /// Interpolated values are wrapped in directional isolates so that a
    /// right-to-left value cannot reorder the text around it. That is a
    /// rendering concern: the characters are invisible, but they are still
    /// characters, and an accessibility label carrying them is neither what a
    /// screen reader should announce nor what a caller comparing text expects.
    ///
    /// Use this wherever text becomes semantics; use [`Self::to_plain`] where
    /// the exact content matters, such as the value of a text field the user
    /// typed into.
    #[must_use]
    pub fn to_semantic(&self) -> Str {
        let plain = self.to_plain();
        if plain.chars().any(is_bidi_control) {
            return Str::from(
                plain
                    .chars()
                    .filter(|character| !is_bidi_control(*character))
                    .collect::<String>(),
            );
        }
        plain
    }

    /// Converts the attributed string into its plain representation.
    #[must_use]
    pub fn to_plain(&self) -> Str {
        if self.chunks.len() == 1 {
            return self.chunks[0].0.clone();
        }

        let mut result = String::new();
        for (text, _) in &self.chunks {
            result.push_str(text);
        }
        result.into()
    }

    /// Returns `true` if all chunks are plain text style and nothing is
    /// interactive — a tap span is content a plain fast path must not drop.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        self.tap_spans.is_empty() && self.chunks.iter().all(|(_, style)| style.is_plain())
    }

    /// Consumes the attributed string and returns its constituent chunks.
    #[must_use]
    pub fn into_chunks(self) -> Vec<(Str, Style)> {
        self.chunks
    }

    /// Returns the constituent chunks without consuming the styled text.
    #[must_use]
    pub fn chunks(&self) -> &[(Str, Style)] {
        &self.chunks
    }

    /// Annotates the whole current string with a tap action.
    ///
    /// The payload is read through its `Computed` at activation, not captured
    /// at construction — a `Binding` payload reports the latest value without
    /// rebuilding the text. Calling `on_tap` again replaces the previous
    /// annotation rather than overlapping it, and an empty string keeps none.
    #[must_use]
    pub fn on_tap<E, Args>(
        mut self,
        payload: impl IntoComputed<E>,
        mut handler: impl EventHandler<E, Args>,
    ) -> Self
    where
        E: Clone + 'static,
    {
        let end = self.len();
        if end == 0 {
            return self;
        }
        let payload = payload.into_computed();
        let action = SharedAction::new(move |env: Environment| {
            handler.call(payload.snapshot(), &env);
        });
        self.tap_spans = alloc::vec![TapSpan {
            range: 0..end,
            action,
        }];
        self
    }

    /// The tap-activated regions of this string, as byte ranges over
    /// [`Self::to_plain`].
    #[must_use]
    pub fn tap_spans(&self) -> &[TapSpan] {
        &self.tap_spans
    }

    /// Sets the style for all text in this styled text.
    #[must_use]
    pub fn set_style(self, style: &Style) -> Self {
        self.apply_style(|s| *s = style.clone())
    }

    fn apply_style(mut self, f: impl Fn(&mut Style)) -> Self {
        if self.chunks.is_empty() {
            return self;
        }
        let old_chunks = core::mem::take(&mut self.chunks);
        for (text, mut style) in old_chunks {
            f(&mut style);
            self.push(text, style);
        }
        self
    }

    /// Sets the font size for all chunks.
    #[must_use]
    pub fn size(self, size: f32) -> Self {
        self.apply_style(|s| *s = take(s).size(size))
    }

    /// Sets the font for all chunks.
    #[must_use]
    pub fn font(self, font: impl Into<Font>) -> Self {
        let font = font.into();
        self.apply_style(|s| s.font = font.clone())
    }

    /// Sets the foreground color for all chunks.
    #[must_use]
    pub fn foreground(self, color: impl Into<Color>) -> Self {
        let color = color.into();
        self.apply_style(|s| s.foreground = Some(color.clone()))
    }

    /// Sets the background color for all chunks.
    #[must_use]
    pub fn background_color(self, color: impl Into<Color>) -> Self {
        let color = color.into();
        self.apply_style(|s| s.background = Some(color.clone()))
    }

    /// Sets the font weight for all chunks.
    #[must_use]
    pub fn weight(self, weight: FontWeight) -> Self {
        self.apply_style(|s| {
            *s = take(s).weight(weight);
        })
    }

    /// Sets the font to bold for all chunks.
    #[must_use]
    pub fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    /// Asks for the platform's fixed-pitch face for all chunks.
    #[must_use]
    pub fn monospaced(self) -> Self {
        self.apply_style(|s| {
            *s = take(s).monospaced();
        })
    }

    /// Sets the italic style for all chunks.
    #[must_use]
    pub fn italic(self, italic: bool) -> Self {
        self.apply_style(|s| s.italic = italic)
    }

    /// Sets the underline style for all chunks.
    #[must_use]
    pub fn underline(self, underline: bool) -> Self {
        self.apply_style(|s| s.underline = underline)
    }

    /// Sets the strikethrough style for all chunks.
    #[must_use]
    pub fn strikethrough(self, strikethrough: bool) -> Self {
        self.apply_style(|s| s.strikethrough = strikethrough)
    }
}

/// Utility builder that incrementally constructs a [`StyledStr`] from Markdown
/// events. The builder keeps track of the active style stack and merges
/// contiguous text runs that share the same styling.
#[derive(Debug, Clone)]
pub struct MarkdownInlineBuilder {
    base_style: Style,
    stack: smallvec::SmallVec<[Style; 4]>,
    buffer: String,
    result: StyledStr,
}

impl Default for MarkdownInlineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownInlineBuilder {
    /// Creates a new builder using the default style.
    #[must_use]
    pub fn new() -> Self {
        Self::with_base_style(Style::default())
    }

    /// Creates a new builder with a custom base style.
    #[must_use]
    pub fn with_base_style(style: Style) -> Self {
        Self {
            base_style: style.clone(),
            stack: smallvec::smallvec![style],
            buffer: String::new(),
            result: StyledStr::empty(),
        }
    }

    fn current_style(&self) -> Style {
        self.stack.last().cloned().unwrap_or_else(Style::default)
    }

    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        let text = take(&mut self.buffer);
        self.result.push(text, self.current_style());
    }

    /// Appends raw text to the builder.
    pub fn push_text(&mut self, text: &str) {
        if !text.is_empty() {
            self.buffer.push_str(text);
        }
    }

    /// Appends a soft break (space) to the builder.
    pub fn push_soft_break(&mut self) {
        self.buffer.push(' ');
    }

    /// Appends a hard break (newline) to the builder.
    pub fn push_hard_break(&mut self) {
        self.buffer.push('\n');
    }

    /// Starts a new styled span using the closure to derive the child style.
    pub fn enter_with(&mut self, f: impl FnOnce(Style) -> Style) {
        self.flush();
        let style = self.current_style();
        self.stack.push(f(style));
    }

    /// Exits the most recently entered styled span.
    pub fn exit(&mut self) {
        self.flush();
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Enters an italic styled span.
    pub fn enter_emphasis(&mut self) {
        self.enter_with(Style::italic);
    }

    /// Enters a bold styled span.
    pub fn enter_strong(&mut self) {
        self.enter_with(Style::bold);
    }

    /// Enters a strikethrough styled span.
    pub fn enter_strikethrough(&mut self) {
        self.enter_with(Style::strikethrough);
    }

    /// Enters an inline code styled span.
    pub fn enter_code(&mut self) {
        self.enter_with(inline_code_style);
    }

    /// Pushes inline code text with code styling applied.
    pub fn push_inline_code(&mut self, text: &str) {
        self.enter_code();
        self.push_text(text);
        self.exit();
    }

    /// Returns `true` if the builder has emitted no content yet.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.result.is_empty() && self.buffer.is_empty()
    }

    /// Returns the base style used by the builder.
    #[must_use]
    pub fn base_style(&self) -> Style {
        self.base_style.clone()
    }

    /// Takes the currently buffered content, resetting the builder to the base
    /// style. If no content has been emitted, `None` is returned.
    #[must_use]
    pub fn take(&mut self) -> Option<StyledStr> {
        self.flush();

        if self.result.is_empty() {
            return None;
        }

        let mut output = StyledStr::empty();
        core::mem::swap(&mut output, &mut self.result);
        self.stack.truncate(1);
        if let Some(first) = self.stack.first_mut() {
            *first = self.base_style.clone();
        }

        Some(output)
    }

    /// Consumes the builder and returns the final `StyledStr`.
    #[must_use]
    pub fn finish(mut self) -> StyledStr {
        self.flush();
        self.result
    }
}

/// Returns the default style applied to Markdown headings.
///
/// Levels consume the semantic font slots in descending platform order: on
/// Apple platforms this resolves to `.title`, `.headline`, `.body`,
/// `.subheadline`, `.footnote`, `.caption`, so the rendered sizes step down
/// with the heading level. Do not reorder — `Headline` is an emphasized body
/// style on Apple platforms, so ordering slots by their *names* scrambles the
/// scale (`#` body-sized, `###` smaller than body).
#[cfg(feature = "markdown")]
#[must_use]
pub fn heading_style(level: HeadingLevel) -> Style {
    use crate::font::{Body, Caption, Footnote, Headline, Subheadline, Title};

    let font: Font = match level {
        HeadingLevel::H1 => Title.into(),
        HeadingLevel::H2 => Headline.into(),
        HeadingLevel::H3 => Body.into(),
        HeadingLevel::H4 => Subheadline.into(),
        HeadingLevel::H5 => Footnote.into(),
        HeadingLevel::H6 => Caption.into(),
    };

    Style::default().font(font).bold()
}

fn inline_code_style(style: Style) -> Style {
    style
        .monospaced()
        .background(Color::new(SurfaceVariantColor))
}

impl View for StyledStr {
    fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
        text(self)
    }
}

impl Add for StyledStr {
    type Output = Self;

    fn add(mut self, rhs: Self) -> Self::Output {
        let offset = self.len();
        for (text, style) in rhs.chunks {
            self.push(text, style);
        }
        self.tap_spans
            .extend(rhs.tap_spans.into_iter().map(|mut span| {
                span.range = span.range.start + offset..span.range.end + offset;
                span
            }));
        self
    }
}

impl Add<&'static str> for StyledStr {
    type Output = Self;

    fn add(mut self, rhs: &'static str) -> Self::Output {
        self.push(rhs, Style::default());
        self
    }
}

impl Extend<(Str, Style)> for StyledStr {
    fn extend<T: IntoIterator<Item = (Str, Style)>>(&mut self, iter: T) {
        for (text, style) in iter {
            self.push(text, style);
        }
    }
}

impl From<Str> for StyledStr {
    fn from(value: Str) -> Self {
        Self::plain(value)
    }
}

impl From<&'static str> for StyledStr {
    fn from(value: &'static str) -> Self {
        Self::plain(value)
    }
}

impl From<String> for StyledStr {
    fn from(value: String) -> Self {
        Self::plain(value)
    }
}

impl Display for StyledStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_plain())
    }
}

impl_constant!(Style, StyledStr);

/// Whether `character` only tells the bidi algorithm what to do.
///
/// Covers the isolates and embeddings — the marks a formatter inserts around
/// interpolated values — and not the text itself.
const fn is_bidi_control(character: char) -> bool {
    matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{rc::Rc, vec::Vec};
    use core::cell::RefCell;
    use nami::binding;

    fn env() -> Environment {
        Environment::new()
    }

    #[test]
    fn each_tap_span_delivers_its_own_typed_payload() {
        let calls = Rc::new(RefCell::new(Vec::<&'static str>::new()));
        let seen = calls.clone();
        let text = StyledStr::plain("https://example.com").on_tap("url", move |v: &'static str| {
            seen.borrow_mut().push(v);
        });
        text.tap_spans()[0].activate(&env());
        assert_eq!(calls.borrow().as_slice(), ["url"]);

        let numbers = Rc::new(RefCell::new(Vec::<u64>::new()));
        let seen = numbers.clone();
        let text = StyledStr::plain("#release").on_tap(7_u64, move |v: u64| {
            seen.borrow_mut().push(v);
        });
        text.tap_spans()[0].activate(&env());
        assert_eq!(numbers.borrow().as_slice(), [7]);

        let names = Rc::new(RefCell::new(Vec::<String>::new()));
        let seen = names.clone();
        let text =
            StyledStr::plain("@lexoliu").on_tap(String::from("mention"), move |v: String| {
                seen.borrow_mut().push(v);
            });
        text.tap_spans()[0].activate(&env());
        assert_eq!(names.borrow().as_slice(), ["mention"]);
    }

    #[test]
    fn a_binding_payload_reports_the_latest_value_at_activation() {
        let payload = binding(1_i32);
        let calls = Rc::new(RefCell::new(Vec::<i32>::new()));
        let seen = calls.clone();
        let text = StyledStr::plain("counter").on_tap(payload.clone(), move |v: i32| {
            seen.borrow_mut().push(v);
        });
        payload.set(2);
        text.tap_spans()[0].activate(&env());
        assert_eq!(calls.borrow().as_slice(), [2]);
    }

    #[test]
    fn concatenation_offsets_spans_across_multibyte_boundaries() {
        let hits = Rc::new(RefCell::new(Vec::<u8>::new()));
        let seen = hits.clone();
        let seen2 = hits.clone();
        // "a\u{1F600}\u{0301}" is 1 + 4 + 2 = 7 bytes of utf-8.
        let first = StyledStr::plain("a\u{1F600}\u{0301}").on_tap(1_u8, move |v: u8| {
            seen.borrow_mut().push(v);
        });
        let second = StyledStr::plain("\u{1F601}b").on_tap(2_u8, move |v: u8| {
            seen2.borrow_mut().push(v);
        });
        let combined = first + second;
        let spans = combined.tap_spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].range(), 0..7);
        assert_eq!(spans[1].range(), 7..12);
        // Every span boundary sits on a char boundary of the plain text.
        let plain = combined.to_plain();
        for span in spans {
            assert!(plain.is_char_boundary(span.range().start));
            assert!(plain.is_char_boundary(span.range().end));
        }
        spans[1].activate(&env());
        assert_eq!(hits.borrow().as_slice(), [2]);
    }

    #[test]
    fn styling_edits_preserve_tap_spans() {
        let calls = Rc::new(RefCell::new(Vec::<u8>::new()));
        let seen = calls.clone();
        let text = StyledStr::plain("styled")
            .on_tap(3_u8, move |v: u8| {
                seen.borrow_mut().push(v);
            })
            .foreground(waterui_graphics::color::Srgb::new(1.0, 0.0, 0.0))
            .underline(true)
            .set_style(&Style::new().italic());
        assert_eq!(text.tap_spans()[0].range(), 0..6);
        text.tap_spans()[0].activate(&env());
        assert_eq!(calls.borrow().as_slice(), [3]);
    }

    #[test]
    fn an_empty_string_carries_no_tap_spans() {
        assert!(
            StyledStr::empty()
                .on_tap(0_u8, |_: u8| {})
                .tap_spans()
                .is_empty()
        );
        assert!(
            StyledStr::plain("")
                .on_tap(0_u8, |_: u8| {})
                .tap_spans()
                .is_empty(),
            "a plain empty string has one chunk but zero bytes to annotate"
        );
        let mut chunks_only = StyledStr::empty();
        chunks_only.push("", Style::new());
        chunks_only.push("", Style::new());
        assert!(
            chunks_only.on_tap(0_u8, |_: u8| {}).tap_spans().is_empty(),
            "empty chunks are still zero bytes"
        );
    }

    #[test]
    fn a_second_on_tap_replaces_the_first_annotation() {
        let calls = Rc::new(RefCell::new(Vec::<u8>::new()));
        let seen = calls.clone();
        let text = StyledStr::plain("replace")
            .on_tap(1_u8, {
                let seen = calls.clone();
                move |v: u8| {
                    seen.borrow_mut().push(v);
                }
            })
            .on_tap(9_u8, move |v: u8| {
                seen.borrow_mut().push(v);
            });
        assert_eq!(text.tap_spans().len(), 1);
        text.tap_spans()[0].activate(&env());
        assert_eq!(calls.borrow().as_slice(), [9]);
    }

    #[test]
    fn appended_text_falls_outside_an_earlier_span() {
        let text = StyledStr::plain("old").on_tap(1_u8, |_: u8| {});
        let appended = text + " new";
        assert_eq!(appended.tap_spans()[0].range(), 0..3);

        let mut pushed = StyledStr::plain("old").on_tap(1_u8, |_: u8| {});
        pushed.push_str(" more");
        assert_eq!(pushed.tap_spans()[0].range(), 0..3);
    }

    #[test]
    fn cloned_and_concatenated_spans_invoke_their_own_handlers() {
        let hits = Rc::new(RefCell::new(Vec::<u8>::new()));
        let first = StyledStr::plain("aa").on_tap(1_u8, {
            let hits = hits.clone();
            move |v: u8| {
                hits.borrow_mut().push(v);
            }
        });
        let second = StyledStr::plain("bb").on_tap(2_u8, {
            let hits = hits.clone();
            move |v: u8| {
                hits.borrow_mut().push(v);
            }
        });
        let combined = first.clone() + second;
        combined.tap_spans()[0].activate(&env());
        combined.tap_spans()[1].activate(&env());
        // The clone shares the action, so it reports the same payload.
        first.tap_spans()[0].activate(&env());
        assert_eq!(hits.borrow().as_slice(), [1, 2, 1]);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn parses_emphasis_markdown() {
        let styled = StyledStr::from_markdown("Hello *world*!");
        let chunks = styled.into_chunks();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].0.as_str(), "Hello ");
        assert_eq!(chunks[1].0.as_str(), "world");
        assert!(chunks[1].1.italic);
        assert_eq!(chunks[2].0.as_str(), "!");
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn parses_heading_markdown() {
        let styled = StyledStr::from_markdown("# Title");
        let chunks = styled.into_chunks();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0.as_str(), "Title");
    }

    /// Heading levels must resolve to a non-increasing type scale. A backend
    /// installs its own platform sizes for each slot, so the test mimics the
    /// iOS scale (`title` 28, `headline` 17, `body` 17, `subheadline` 15,
    /// `footnote` 13, `caption` 12); on the previous slot assignment this
    /// produced 17/28/15/17/12/13 — `##` larger than `#` and `####` larger
    /// than `###`.
    #[cfg(feature = "markdown")]
    #[test]
    fn heading_levels_descend_the_platform_type_scale() {
        use crate::font::{Body, Caption, Footnote, Headline, ResolvedFont, Subheadline, Title};
        use nami::{Computed, Signal};
        use waterui_core::{Environment, env::Store};

        let mut env = Environment::new();
        env.insert(Store::<Title, _>::new(Computed::constant(
            ResolvedFont::new(28.0, FontWeight::Normal),
        )));
        env.insert(Store::<Headline, _>::new(Computed::constant(
            ResolvedFont::new(17.0, FontWeight::SemiBold),
        )));
        env.insert(Store::<Body, _>::new(Computed::constant(
            ResolvedFont::new(17.0, FontWeight::Normal),
        )));
        env.insert(Store::<Subheadline, _>::new(Computed::constant(
            ResolvedFont::new(15.0, FontWeight::Normal),
        )));
        env.insert(Store::<Footnote, _>::new(Computed::constant(
            ResolvedFont::new(13.0, FontWeight::Normal),
        )));
        env.insert(Store::<Caption, _>::new(Computed::constant(
            ResolvedFont::new(12.0, FontWeight::Normal),
        )));

        let sizes: Vec<f32> = [
            HeadingLevel::H1,
            HeadingLevel::H2,
            HeadingLevel::H3,
            HeadingLevel::H4,
            HeadingLevel::H5,
            HeadingLevel::H6,
        ]
        .iter()
        .map(|&level| heading_style(level).font.resolve(&env).snapshot().size())
        .collect();

        assert_eq!(sizes.as_slice(), &[28.0, 17.0, 17.0, 15.0, 13.0, 12.0]);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn parses_strikethrough_markdown() {
        let styled = StyledStr::from_markdown("Normal ~~removed~~ text");
        let chunks = styled.into_chunks();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[1].0.as_str(), "removed");
        assert!(chunks[1].1.strikethrough);
    }

    #[cfg(feature = "markdown")]
    #[test]
    fn parses_inline_code_markdown() {
        let styled = StyledStr::from_markdown("Run `cargo test` now");
        let chunks = styled.into_chunks();
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[1].0.as_str(), "cargo test");
        assert!(chunks[1].1.background.is_some());
    }

    #[test]
    fn plain_detection_works() {
        assert!(StyledStr::plain("abc").is_plain());
        assert!(!StyledStr::plain("abc").italic(true).is_plain());
    }
}
