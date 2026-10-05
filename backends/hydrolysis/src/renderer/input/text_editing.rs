// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use unicode_segmentation::UnicodeSegmentation;
use waterui_graphics::draw::Draw as _;

/// What became of a key press once the framework finished with it.
///
/// The distinction that matters is `Consumed` vs `ForwardedToSurface`:
/// only a consumed press suppresses the paired `TextInput` that follows it
/// in the event queue (the web platform's `keydown` → `beforeinput` rule —
/// `preventDefault` on the press cancels the text). A press forwarded to an
/// embedded surface was *delivered*, not consumed: the surface owns its
/// key+text pair, exactly as `SurfaceInputEvent` documents, and decides
/// internally what the press meant.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyPressOutcome {
    /// No handler, editor, or surface accepted the press.
    Ignored,
    /// A handler consumed the press — editing action, focus traversal, or
    /// an `OnKeyPress` ancestor reporting `KeyHandling::Handled`.
    Consumed,
    /// The press was forwarded to an embedded surface; its paired text is
    /// delivered too, as the surface contract.
    ForwardedToSurface,
}

#[derive(Clone)]
pub enum TextInputModel {
    TextField {
        value: nami::Binding<StyledStr>,
        line_limit: Option<usize>,
        selection_menu: nami::Computed<Vec<ResolvedMenuItem>>,
        /// `TextField::on_submit` — run on Return in a line-limited field; a
        /// field without one leaves the key unconsumed so it bubbles.
        on_submit: Option<SharedAction>,
    },
    SecureField {
        value: nami::Binding<FormSecure>,
    },
}

/// Text editing state that outlives a frame.
///
/// [`Self::text_input_targets`] is pure emission: it is cleared in
/// `reset_scene` and re-pushed in flush order every frame, so a position in it
/// only means anything within the frame that produced it. Everything here that
/// survives a frame — focus, the drag-selection target, the multi-click streak,
/// the open context menu — therefore stores the target's stable
/// [`InteractionKey`] and resolves it back to a position through
/// [`Self::index_of`] at the point of use. Storing a position instead would let
/// focus, caret and selection migrate to a different field whenever flush order
/// changes (a row inserted above a focused field, a `when(...)` revealing an
/// earlier one).
#[derive(Default)]
pub struct TextEditingState {
    pub(crate) text_input_targets: Vec<TextInputTarget>,
    pub(crate) active_text_selection_drag: Option<ActiveTextSelectionDrag>,
    pub(crate) last_text_selection_click: Option<TextSelectionClickState>,
    pub(crate) active_text_context_menu: Option<ActiveTextContextMenu>,
    focused_text_input: RefCell<Option<InteractionKey>>,
    /// §7.1 ownership for nested scroll surfaces: the surface that cleared
    /// the focused field this frame records its target key here, and outer
    /// surfaces skip targets an inner one already claimed (an outer surface
    /// must not scroll for a field only its inner surface covers). Cleared
    /// with the per-flush target list in `reset_scene`.
    pub(crate) focused_clearance_claim: Option<InteractionKey>,
    /// The `.focused()` binding of the field holding text focus, captured
    /// while the target is emitted so unfocus writes still reach it after
    /// the target has been truncated or unmounted.
    pub(crate) focused_binding: Option<nami::Binding<bool>>,
    pub(crate) ime_preedit: Option<Str>,
    /// Byte offset of the caret inside `ime_preedit`, as the platform
    /// reported it — the live composition caret the candidate window must
    /// follow. `None` when the platform did not report one, or whenever
    /// `ime_preedit` is `None`.
    pub(crate) ime_preedit_caret: Option<usize>,
    pub(crate) text_caret_fade_started_at: Option<Instant>,
    pub(crate) text_caret_next_frame_at: Option<Instant>,
    pub(crate) text_caret_motion: Option<TextCaretMotion>,
    /// How many primary presses have landed on an editable target. A
    /// platform with a soft keyboard shows it again when this changes, so a
    /// tap on the field already holding focus brings back a keyboard the
    /// user dismissed.
    activations: u64,
}

impl TextEditingState {
    /// Records a primary press on an editable target.
    pub(crate) const fn note_activation(&mut self) {
        self.activations = self
            .activations
            .checked_add(1)
            .expect("text input activation count overflow");
    }

    /// The press count [`Self::note_activation`] maintains.
    pub(crate) const fn activations(&self) -> u64 {
        self.activations
    }
    /// This frame's position for a stable target identity, if that target is
    /// still emitted.
    pub(crate) fn index_of(&self, key: &InteractionKey) -> Option<usize> {
        self.text_input_targets
            .iter()
            .position(|target| &target.interaction_key == key)
    }

    /// The stable identity of this frame's target at `index`.
    pub(crate) fn key_at(&self, index: usize) -> Option<&InteractionKey> {
        self.text_input_targets
            .as_slice()
            .get(index)
            .map(|target| &target.interaction_key)
    }

    /// The focused input's identity, whether or not it is emitted this frame.
    pub(crate) fn focused_key(&self) -> Option<InteractionKey> {
        self.focused_text_input.borrow().clone()
    }

    /// This frame's position of the focused input, if it is still emitted.
    pub(crate) fn focused_index(&self) -> Option<usize> {
        let focused = self.focused_text_input.borrow();
        self.index_of(focused.as_ref()?)
    }

    /// This frame's focused target, if it is still emitted.
    pub(crate) fn focused_target(&self) -> Option<&TextInputTarget> {
        self.text_input_targets
            .as_slice()
            .get(self.focused_index()?)
    }

    pub(crate) fn is_focused(&self, key: &InteractionKey) -> bool {
        self.focused_text_input.borrow().as_ref() == Some(key)
    }

    pub(crate) fn has_focus(&self) -> bool {
        self.focused_text_input.borrow().is_some()
    }

    fn store_focused_key(&self, key: Option<InteractionKey>) {
        *self.focused_text_input.borrow_mut() = key;
    }

    /// This frame's position of the drag-selected input, if it is still emitted.
    pub(crate) fn selection_drag_index(&self) -> Option<usize> {
        self.index_of(&self.active_text_selection_drag.as_ref()?.target)
    }

    /// Drops the stored composition, keeping its caret consistent.
    const fn take_ime_preedit(&mut self) -> Option<Str> {
        self.ime_preedit_caret = None;
        self.ime_preedit.take()
    }
}

#[derive(Debug, Default)]
pub struct TextSelectionSlot {
    pub(crate) anchor: usize,
    pub(crate) focus: usize,
    pub initialized: bool,
}

#[derive(Debug, Clone)]
pub struct TextSelectionClickState {
    pub(crate) target: InteractionKey,
    pub(crate) point: kurbo::Point,
    pub(crate) at: Instant,
    pub(crate) count: u8,
}

/// An in-flight text-selection drag: which field it belongs to, the click
/// streak granularity that armed it, and the plain-text range that gesture
/// selected at pointer-down. The range is the drag's anchor side — for a
/// multi-click drag it is the word/line the gesture snapped to — so later
/// moves extend by whole units instead of collapsing the gesture back to a
/// caret.
#[derive(Debug, Clone)]
pub struct ActiveTextSelectionDrag {
    pub(crate) target: InteractionKey,
    /// The click streak that armed this drag: 1 = caret, 2 = word, 3+ = line.
    pub(crate) click_count: u8,
    /// The selection (plain-text byte indices) the arming gesture applied.
    pub(crate) anchor: usize,
    pub(crate) focus: usize,
}

#[derive(Clone)]
pub struct TextInputTarget {
    pub(crate) interaction_key: InteractionKey,
    pub(crate) modal: bool,
    pub(crate) bounds: kurbo::Rect,
    /// The field's laid-out frame in the same window coordinates, before
    /// the hit clip [`Self::bounds`] went through. A field covered by a
    /// scroll surface's clip keeps a real rectangle here — the §7.1
    /// focused-field clearance measures "the field's frame" against it,
    /// which the hit bounds cannot answer once they degenerate to the
    /// clip's edge.
    pub(crate) frame: kurbo::Rect,
    pub(crate) cursor_area: kurbo::Rect,
    pub(crate) text_bounds: kurbo::Rect,
    pub(crate) text_clip_bounds: kurbo::Rect,
    pub(crate) content_alpha: f32,
    pub(crate) layout: std::sync::Arc<parley::Layout<[u8; 4]>>,
    /// The string the IME-visible layout was typeset from — the committed
    /// text with the live pre-edit spliced in (a text field) or the mask
    /// glyphs (a secure field) — and that layout, in `text_bounds`
    /// coordinates. The platform editing session resolves cursor-anchor
    /// character bounds against it.
    #[cfg_attr(
        not(any(target_os = "android", test)),
        allow(
            dead_code,
            reason = "read by the Android editing session and its tests"
        )
    )]
    pub(crate) display_text: Str,
    #[cfg_attr(
        not(any(target_os = "android", test)),
        allow(
            dead_code,
            reason = "read by the Android editing session and its tests"
        )
    )]
    pub(crate) display_layout: std::sync::Arc<parley::Layout<[u8; 4]>>,
    pub(crate) purpose: TextInputPurpose,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) model: TextInputModel,
    pub(crate) selection: Rc<RefCell<TextSelectionSlot>>,
    /// The environment of the view the target was registered from — the
    /// context menu it opens runs inside it (water-rs/hydrolysis#140).
    pub(crate) env: Environment,
    /// The `OnKeyPress` scopes enclosing the view this target was registered
    /// from — the chain a key the field did not consume bubbles through,
    /// innermost first at dispatch time.
    pub(crate) key_handlers: Option<Rc<KeyHandlerNode>>,
    pub(crate) focus_binding: Option<Binding<bool>>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_node_id: Option<AccessibilityNodeId>,
}

/// One `OnKeyPress` ancestor scope a focused input's unconsumed keys bubble
/// into. The scope stack is pushed while the retained tree flushes the
/// `OnKeyPress` wrapper (outermost first), and a target snapshotting it keeps
/// the whole chain even after the frame that produced it is gone.
pub struct KeyHandlerScope {
    /// The environment the `.on_key_press` view was built under — the handler
    /// resolves `State`/`Use` extractors against it, extended with the press.
    pub(crate) env: Environment,
    pub(crate) handler: Rc<RefCell<OnKeyPress>>,
}

/// One link of the `OnKeyPress` scope chain: the innermost scope at a
/// registration point plus the rest of its ancestors. Pushing a scope
/// allocates a single node, and a target's snapshot of the chain is a single
/// `Rc` clone, so neither the walk nor the snapshot allocates per frame.
pub struct KeyHandlerNode {
    pub scope: KeyHandlerScope,
    pub(crate) parent: Option<Rc<Self>>,
}

/// The innermost chain node shared by `a` and `b` — the scopes enclosing
/// every registration the two chains were snapped from.
pub fn common_key_handler_scope(
    a: Option<Rc<KeyHandlerNode>>,
    b: Option<Rc<KeyHandlerNode>>,
) -> Option<Rc<KeyHandlerNode>> {
    fn depth(mut node: Option<Rc<KeyHandlerNode>>) -> usize {
        let mut depth = 0;
        // The while-let moves `node` into the pattern each pass, so a fresh
        // clone is the only way to rebind it — `clone_from` needs a live
        // target binding, which this loop does not have.
        #[allow(clippy::assigning_clones)]
        while let Some(link) = node {
            depth += 1;
            node = link.parent.clone();
        }
        depth
    }
    let mut a = a;
    let mut b = b;
    let mut a_depth = depth(a.clone());
    let mut b_depth = depth(b.clone());
    while a_depth > b_depth {
        a = a.and_then(|link| link.parent.clone());
        a_depth -= 1;
    }
    while b_depth > a_depth {
        b = b.and_then(|link| link.parent.clone());
        b_depth -= 1;
    }
    loop {
        match (a, b) {
            (Some(x), Some(y)) => {
                if Rc::ptr_eq(&x, &y) {
                    return Some(x);
                }
                // `a`/`b` are moved into the match pattern, so rebinding
                // them takes fresh clones — `clone_from` cannot run on a
                // moved binding.
                #[allow(clippy::assigning_clones)]
                {
                    a = x.parent.clone();
                    b = y.parent.clone();
                }
            }
            _ => return None,
        }
    }
}

/// The focused text input's authoritative editing state, projected for a
/// platform editing session — see
/// [`SemanticCore::focused_editor_snapshot`]. Offsets are byte indices into
/// `text` (the committed, pre-edit-free plain text) except `preedit_caret`,
/// which indexes `preedit`.
#[cfg_attr(
    not(any(target_os = "android", test)),
    allow(
        dead_code,
        reason = "read by the Android editing session and its tests"
    )
)]
pub struct FocusedEditorSnapshot {
    /// The focused field's stable identity; a host editing session tags its
    /// writes with the editor this snapshot minted so a stale connection's
    /// writes never reach a different field.
    pub(crate) key: InteractionKey,
    /// Committed (pre-edit-free) plain text.
    pub(crate) text: String,
    /// Selection slot in byte offsets of [`Self::text`]; `anchor` may lead
    /// `focus` (a keyboard selection made backwards).
    pub(crate) anchor: usize,
    pub(crate) focus: usize,
    /// Live pre-edit and the platform-reported caret byte offset in it.
    pub(crate) preedit: Option<String>,
    pub(crate) preedit_caret: Option<usize>,
    /// `SecureField` — marked text is refused and copy/cut stay inert.
    pub(crate) password: bool,
    /// `Some(1)` for a single-line field, `None` for unbounded multiline.
    pub(crate) line_limit: Option<usize>,
    /// Whether the field declares `on_submit` — the editor action an IME's
    /// Done key fires.
    pub(crate) has_submit: bool,
    /// The field's hit bounds in window logical coordinates — the editor
    /// bounds an IME sizes candidate windows against.
    pub(crate) bounds: kurbo::Rect,
    /// Caret rect in window logical coordinates (composition-aware).
    pub(crate) cursor_area: kurbo::Rect,
    /// Origin of the layout [`Self::display_layout`] was typeset on.
    pub(crate) text_bounds: kurbo::Rect,
    /// The text `display_layout` describes — committed + pre-edit for a
    /// field, the mask glyphs for a secure field.
    pub(crate) display_text: String,
    pub(crate) display_layout: std::sync::Arc<parley::Layout<[u8; 4]>>,
}

pub struct TextInputTargetRegistration {
    pub interaction_key: InteractionKey,
    pub(crate) modal: bool,
    pub(crate) bounds: kurbo::Rect,
    pub(crate) cursor_area: kurbo::Rect,
    pub(crate) text_bounds: kurbo::Rect,
    pub(crate) text_clip_bounds: kurbo::Rect,
    pub(crate) content_alpha: f32,
    pub(crate) layout: std::sync::Arc<parley::Layout<[u8; 4]>>,
    /// See [`TextInputTarget::display_text`].
    #[cfg_attr(
        not(any(target_os = "android", test)),
        allow(
            dead_code,
            reason = "read by the Android editing session and its tests"
        )
    )]
    pub(crate) display_text: Str,
    #[cfg_attr(
        not(any(target_os = "android", test)),
        allow(
            dead_code,
            reason = "read by the Android editing session and its tests"
        )
    )]
    pub(crate) display_layout: std::sync::Arc<parley::Layout<[u8; 4]>>,
    pub(crate) purpose: TextInputPurpose,
    pub(crate) model: TextInputModel,
    pub(crate) selection: Rc<RefCell<TextSelectionSlot>>,
    /// The environment of the registering view; the context menu opens inside
    /// it (water-rs/hydrolysis#140).
    pub(crate) env: Environment,
}

pub struct TextInputTargetData {
    pub target: TextInputTargetRegistration,
    pub(crate) depth: usize,
    pub(crate) focus_binding: Option<Binding<bool>>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_node_id: Option<AccessibilityNodeId>,
}

#[derive(Clone, Copy)]
pub enum TextContextMenuAction {
    Copy,
    Cut,
    Paste,
    SelectAll,
}

/// A built-in selection-menu row as a [`PopupMenuNode`]: its action runs
/// `action` against `model`/`selection` in the row's dispatch environment —
/// the environment the menu opened in, which `popup_menu_window` and the
/// drawn overlay both hand the press.
fn text_context_menu_builtin_node(
    label: String,
    action: TextContextMenuAction,
    model: &TextInputModel,
    selection: &Rc<RefCell<TextSelectionSlot>>,
) -> PopupMenuNode {
    let model = model.clone();
    let selection = Rc::clone(selection);
    let semantic_text = label.clone();
    let content = label.clone();
    PopupMenuNode::Command {
        label: waterui_controls::label::Label::new(semantic_text, move || {
            AnyView::new(
                waterui_layout::frame::Frame::new(Text::new(StyledStr::plain(content.clone())))
                    .alignment(waterui_layout::alignment::Leading)
                    .max_width(f32::INFINITY),
            )
        }),
        plain_label: label,
        action: SharedAction::new(move |env: Environment| {
            let _ = execute_text_context_menu_action(action, &model, &selection, &env);
        }),
        disabled: nami::Computed::constant(false),
        shortcut: None,
        subtitle: None,
    }
}

#[derive(Clone)]
pub struct TextContextMenuOverlayRow {
    pub(crate) bounds: kurbo::Rect,
    pub node: PopupMenuNode,
}

#[derive(Clone)]
pub struct TextContextMenuOverlay {
    pub(crate) bounds: kurbo::Rect,
    pub rows: Vec<TextContextMenuOverlayRow>,
    /// Open/closed handles for the submenu popup windows this overlay opens.
    /// Dismissal closes the whole chain.
    pub(crate) menu_group: PopupMenuStateGroup,
    /// The overlay's handle in `menu_group`, at index 0: the overlay is drawn,
    /// not a window, so this sentinel stands in for the root window a
    /// `.context_menu` chain starts with. A command row's `close_all()` marks
    /// it `Closed`, which the next render and pointer-down read as "the menu's
    /// command already ran — dismiss the overlay too".
    pub(crate) dismiss_state: nami::Binding<WindowState>,
    /// The widget theme the overlay was opened under: submenu windows draw
    /// with the same menu metrics and surface treatment.
    pub(crate) theme: Rc<dyn crate::engine::WidgetTheme>,
    pub(crate) env: Environment,
}

#[derive(Clone)]
pub enum ActiveTextContextMenu {
    Overlay {
        target: InteractionKey,
        overlay: TextContextMenuOverlay,
    },
    NativeWindow {
        target: InteractionKey,
        group: PopupMenuStateGroup,
    },
}

impl TextInputModel {
    pub(crate) fn plain_text(&self) -> String {
        match self {
            Self::TextField { value, .. } => value.snapshot().to_plain().to_string(),
            Self::SecureField { value } => value.snapshot().expose().to_owned(),
        }
    }

    pub(crate) fn set_plain_text(&self, text: String) {
        match self {
            Self::TextField { value, .. } => value.set(StyledStr::plain(text)),
            Self::SecureField { value } => {
                let mut next = FormSecure::default();
                next.set(text);
                value.set(next);
            }
        }
    }

    pub(crate) const fn line_limit(&self) -> Option<usize> {
        match self {
            Self::TextField { line_limit, .. } => *line_limit,
            Self::SecureField { .. } => Some(1),
        }
    }

    pub(crate) const fn is_secure(&self) -> bool {
        matches!(self, Self::SecureField { .. })
    }

    pub(crate) fn custom_selection_menu_items(&self) -> Vec<ResolvedMenuItem> {
        match self {
            Self::TextField { selection_menu, .. } => selection_menu.snapshot(),
            Self::SecureField { .. } => Vec::new(),
        }
    }

    pub(crate) fn layout_index_from_plain_index(&self, plain_index: usize) -> usize {
        match self {
            Self::TextField { .. } => {
                let text = self.plain_text();
                clamp_to_char_boundary(text.as_str(), plain_index)
            }
            Self::SecureField { .. } => {
                let text = self.plain_text();
                byte_index_to_char_offset(text.as_str(), plain_index)
            }
        }
    }

    pub(crate) fn plain_index_from_layout_index(&self, layout_index: usize) -> usize {
        match self {
            Self::TextField { .. } => {
                let text = self.plain_text();
                clamp_to_char_boundary(text.as_str(), layout_index)
            }
            Self::SecureField { .. } => {
                let text = self.plain_text();
                char_offset_to_byte_index(text.as_str(), layout_index)
            }
        }
    }

    pub(crate) fn layout_len_for_plain_text(&self, text: &str) -> usize {
        match self {
            Self::TextField { .. } => text.len(),
            Self::SecureField { .. } => text.chars().count(),
        }
    }
}

pub fn normalized_insert_text(inserted: &str, max_lines: Option<usize>) -> String {
    if max_lines == Some(1) {
        inserted
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r')
            .collect()
    } else {
        inserted.chars().filter(|ch| *ch != '\r').collect()
    }
}

pub fn line_count(value: &str) -> usize {
    value.chars().filter(|ch| *ch == '\n').count() + 1
}

pub fn exceeds_line_limit(value: &str, max_lines: Option<usize>) -> bool {
    max_lines.is_some_and(|max| line_count(value) > max)
}

/// Appends `inserted` to `buffer` (normalized, line-limit enforced); used by the
/// accessibility text-input action handlers.
#[cfg(feature = "accessibility")]
pub fn apply_text_insert(buffer: &mut String, inserted: &str, max_lines: Option<usize>) -> bool {
    let normalized = normalized_insert_text(inserted, max_lines);
    if normalized.is_empty() {
        return false;
    }
    let original_len = buffer.len();
    buffer.push_str(normalized.as_str());
    if exceeds_line_limit(buffer, max_lines) {
        buffer.truncate(original_len);
        return false;
    }
    true
}

pub const fn clamp_to_char_boundary(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        index = text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub fn previous_grapheme_boundary(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    if clamped == 0 {
        return 0;
    }
    text[..clamped]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(value, _)| value)
}

pub fn next_grapheme_boundary(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    if clamped >= text.len() {
        return text.len();
    }
    let Some(grapheme) = text[clamped..].graphemes(true).next() else {
        return text.len();
    };
    clamped + grapheme.len()
}

pub fn byte_index_to_char_offset(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    text[..clamped].chars().count()
}

pub fn char_offset_to_byte_index(text: &str, char_offset: usize) -> usize {
    if char_offset == 0 {
        return 0;
    }
    let mut consumed = 0usize;
    for (index, _) in text.char_indices() {
        if consumed == char_offset {
            return index;
        }
        consumed = consumed
            .checked_add(1)
            .expect("char offset conversion overflow");
    }
    text.len()
}

pub fn normalized_selection_range(anchor: usize, focus: usize) -> std::ops::Range<usize> {
    anchor.min(focus)..anchor.max(focus)
}

pub fn replace_text_selection(
    text: &mut String,
    anchor: &mut usize,
    focus: &mut usize,
    inserted: &str,
    line_limit: Option<usize>,
) -> bool {
    let start = clamp_to_char_boundary(text.as_str(), (*anchor).min(*focus));
    let end = clamp_to_char_boundary(text.as_str(), (*anchor).max(*focus));
    let normalized = normalized_insert_text(inserted, line_limit);
    if normalized.is_empty() {
        return false;
    }
    let mut next = text.clone();
    next.replace_range(start..end, normalized.as_str());
    if exceeds_line_limit(next.as_str(), line_limit) {
        return false;
    }
    *text = next;
    let caret = start + normalized.len();
    *anchor = caret;
    *focus = caret;
    true
}

pub fn delete_backward_in_selection(
    text: &mut String,
    anchor: &mut usize,
    focus: &mut usize,
) -> bool {
    let start = clamp_to_char_boundary(text.as_str(), (*anchor).min(*focus));
    let end = clamp_to_char_boundary(text.as_str(), (*anchor).max(*focus));
    if start != end {
        text.replace_range(start..end, "");
        *anchor = start;
        *focus = start;
        return true;
    }
    if start == 0 {
        return false;
    }
    let previous = previous_grapheme_boundary(text.as_str(), start);
    text.replace_range(previous..start, "");
    *anchor = previous;
    *focus = previous;
    true
}

pub fn delete_forward_in_selection(
    text: &mut String,
    anchor: &mut usize,
    focus: &mut usize,
) -> bool {
    let start = clamp_to_char_boundary(text.as_str(), (*anchor).min(*focus));
    let end = clamp_to_char_boundary(text.as_str(), (*anchor).max(*focus));
    if start != end {
        text.replace_range(start..end, "");
        *anchor = start;
        *focus = start;
        return true;
    }
    if start >= text.len() {
        return false;
    }
    let next = next_grapheme_boundary(text.as_str(), start);
    text.replace_range(start..next, "");
    *anchor = start;
    *focus = start;
    true
}

pub fn selection_slot_range_for_text(
    slot: &TextSelectionSlot,
    text: &str,
) -> std::ops::Range<usize> {
    let anchor = clamp_to_char_boundary(text, slot.anchor);
    let focus = clamp_to_char_boundary(text, slot.focus);
    normalized_selection_range(anchor, focus)
}

pub fn selected_text_for_model(model: &TextInputModel, slot: &TextSelectionSlot) -> Option<String> {
    let text = model.plain_text();
    let range = selection_slot_range_for_text(slot, text.as_str());
    if range.is_empty() {
        return None;
    }
    text.as_str().get(range).map(str::to_owned)
}

pub fn replace_model_selection(
    model: &TextInputModel,
    slot: &mut TextSelectionSlot,
    inserted: &str,
) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !replace_text_selection(
        &mut text,
        &mut anchor,
        &mut focus,
        inserted,
        model.line_limit(),
    ) {
        return false;
    }
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    // Binding updates synchronously request a retained-tree refresh. Publish the
    // new selection first so that refresh observes the caret belonging to the
    // edited value instead of reusing the previous character's position.
    model.set_plain_text(text);
    true
}

pub fn delete_model_selection(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    let range = normalized_selection_range(anchor, focus);
    if range.is_empty() {
        return false;
    }
    text.replace_range(range.clone(), "");
    slot.anchor = range.start;
    slot.focus = range.start;
    slot.initialized = true;
    model.set_plain_text(text);
    true
}

pub fn delete_model_backward(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !delete_backward_in_selection(&mut text, &mut anchor, &mut focus) {
        return false;
    }
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    model.set_plain_text(text);
    true
}

pub fn delete_model_forward(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !delete_forward_in_selection(&mut text, &mut anchor, &mut focus) {
        return false;
    }
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    model.set_plain_text(text);
    true
}

pub fn set_model_caret_position(
    model: &TextInputModel,
    slot: &mut TextSelectionSlot,
    index: usize,
) -> bool {
    let text = model.plain_text();
    let index = clamp_to_char_boundary(text.as_str(), index);
    let changed = slot.anchor != index || slot.focus != index || !slot.initialized;
    slot.anchor = index;
    slot.focus = index;
    slot.initialized = true;
    changed
}

#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::future_not_send,
        reason = "wasm32 is single-threaded; the browser Clipboard handle is a JS object and `!Send` by design"
    )
)]
pub async fn read_clipboard_text_async() -> Option<String> {
    let clipboard = match Clipboard::new() {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                target: "waterui::hydrolysis::input",
                error = %error,
                "failed to initialize clipboard for paste"
            );
            return None;
        }
    };
    match clipboard.text().await {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                target: "waterui::hydrolysis::input",
                error = %error,
                "failed to read clipboard text"
            );
            None
        }
    }
}

pub fn spawn_clipboard_paste_task(
    model: TextInputModel,
    selection: Rc<RefCell<TextSelectionSlot>>,
) {
    spawn_local(async move {
        let Some(text) = read_clipboard_text_async().await else {
            return;
        };
        if text.is_empty() {
            return;
        }
        let mut slot = selection.borrow_mut();
        let _ = replace_model_selection(&model, &mut slot, text.as_str());
    })
    .detach();
}

pub fn select_all_model_text(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let text = model.plain_text();
    if text.is_empty() {
        let changed = slot.anchor != 0 || slot.focus != 0 || !slot.initialized;
        slot.anchor = 0;
        slot.focus = 0;
        slot.initialized = true;
        return changed;
    }
    let changed = slot.anchor != 0 || slot.focus != text.len() || !slot.initialized;
    slot.anchor = 0;
    slot.focus = text.len();
    slot.initialized = true;
    changed
}

pub fn write_clipboard_text(text: &str) -> bool {
    let mut clipboard: Clipboard = match Clipboard::new() {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(
                target: "waterui::hydrolysis::input",
                error = %error,
                "failed to initialize clipboard for copy/cut"
            );
            return false;
        }
    };
    if let Err(error) = clipboard.set_text(text) {
        tracing::warn!(
            target: "waterui::hydrolysis::input",
            error = %error,
            "failed to set clipboard text"
        );
        return false;
    }
    true
}

pub fn selection_range_contains_index(
    model: &TextInputModel,
    slot: &TextSelectionSlot,
    index: usize,
) -> bool {
    let text = model.plain_text();
    let range = selection_slot_range_for_text(slot, text.as_str());
    !range.is_empty() && range.contains(&index)
}

pub fn text_context_menu_size(
    nodes: &[PopupMenuNode],
    metrics: TextContextMenuMetrics,
) -> (f64, f64) {
    let max_label_chars = crate::num_cast::usize_as_f64(
        nodes
            .iter()
            .filter_map(|node| match node {
                PopupMenuNode::Command { plain_label, .. }
                | PopupMenuNode::Menu { plain_label, .. } => Some(plain_label.chars().count()),
                PopupMenuNode::Divider => None,
            })
            .max()
            .unwrap_or(0),
    );
    let width = max_label_chars
        .mul_add(metrics.width_per_char, metrics.horizontal_padding * 2.0)
        .clamp(metrics.min_width, metrics.max_width);
    let height =
        (crate::num_cast::usize_as_f64(nodes.len()) * metrics.row_height).max(metrics.row_height);
    (width, height)
}

pub fn text_context_menu_overlay_bounds(
    anchor: kurbo::Point,
    nodes: &[PopupMenuNode],
    window_bounds: kurbo::Rect,
    metrics: TextContextMenuMetrics,
) -> kurbo::Rect {
    let (width, height) = text_context_menu_size(nodes, metrics);
    let preferred_x = anchor.x;
    let preferred_y = anchor.y;
    let fallback_x = anchor.x - width;
    let fallback_y = anchor.y - height;

    let mut x0 = if preferred_x + width <= window_bounds.x1 {
        preferred_x
    } else {
        fallback_x
    };
    let mut y0 = if preferred_y + height <= window_bounds.y1 {
        preferred_y
    } else {
        fallback_y
    };

    if x0 < window_bounds.x0 {
        x0 = window_bounds.x0;
    }
    if x0 + width > window_bounds.x1 {
        x0 = window_bounds.x1 - width;
    }
    if y0 < window_bounds.y0 {
        y0 = window_bounds.y0;
    }
    if y0 + height > window_bounds.y1 {
        y0 = window_bounds.y1 - height;
    }
    kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

pub fn execute_text_context_menu_action(
    action: TextContextMenuAction,
    model: &TextInputModel,
    selection: &Rc<RefCell<TextSelectionSlot>>,
    _env: &Environment,
) -> bool {
    match action {
        TextContextMenuAction::Copy => {
            let slot = selection.borrow();
            let Some(value) = selected_text_for_model(model, &slot) else {
                return false;
            };
            write_clipboard_text(value.as_str())
        }
        TextContextMenuAction::Cut => {
            let mut slot = selection.borrow_mut();
            let Some(value) = selected_text_for_model(model, &slot) else {
                return false;
            };
            write_clipboard_text(value.as_str()) && delete_model_selection(model, &mut slot)
        }
        TextContextMenuAction::Paste => {
            spawn_clipboard_paste_task(model.clone(), Rc::clone(selection));
            true
        }
        TextContextMenuAction::SelectAll => {
            let mut slot = selection.borrow_mut();
            select_all_model_text(model, &mut slot)
        }
    }
}

fn selection_for_target_layout(
    model: &TextInputModel,
    layout: &parley::Layout<[u8; 4]>,
    slot: &TextSelectionSlot,
) -> parley::Selection {
    assert!(
        slot.initialized,
        "hydrolysis registered a text-input target before initializing its selection"
    );
    let plain_text = model.plain_text();
    let anchor = clamp_to_char_boundary(plain_text.as_str(), slot.anchor);
    let focus = clamp_to_char_boundary(plain_text.as_str(), slot.focus);
    let layout_len = model.layout_len_for_plain_text(plain_text.as_str());
    let anchor_layout = model.layout_index_from_plain_index(anchor);
    let focus_layout = model.layout_index_from_plain_index(focus);
    let anchor_affinity = if anchor_layout >= layout_len {
        parley::Affinity::Upstream
    } else {
        parley::Affinity::Downstream
    };
    let focus_affinity = if focus_layout >= layout_len {
        parley::Affinity::Upstream
    } else {
        parley::Affinity::Downstream
    };
    parley::Selection::new(
        parley::Cursor::from_byte_index(layout, anchor_layout, anchor_affinity),
        parley::Cursor::from_byte_index(layout, focus_layout, focus_affinity),
    )
    .refresh(layout)
}

fn refreshed_target_selection(target: &TextInputTarget) -> parley::Selection {
    let slot = target.selection.borrow();
    // The transient overlay may run after the model changes but before the next
    // retained-tree refresh replaces `target.layout`. Project the source
    // selection into that stale layout for this frame without writing the
    // clamped display position back into the model-owned selection slot.
    selection_for_target_layout(&target.model, &target.layout, &slot)
}

impl SemanticCore {
    pub(crate) const fn set_text_caret_motion(&mut self, motion: TextCaretMotion) {
        self.text_editing.text_caret_motion = Some(motion);
    }

    const fn text_caret_motion(&self) -> TextCaretMotion {
        self.text_editing
            .text_caret_motion
            .expect("hydrolysis text input render must install text caret motion before focus")
    }

    pub(crate) fn reset_text_caret_animation(&mut self, now: Instant) {
        // A semantic core never installs a caret motion — the blink is
        // presentation — so focusing a text field there clears the animation
        // state instead of scheduling frames. `advance_text_caret_animation`
        // and `text_caret_opacity` keep their strict contract: they only run
        // on the rendered pump.
        let Some(motion) = self.text_editing.text_caret_motion else {
            self.clear_text_caret_animation();
            return;
        };
        self.text_editing.text_caret_fade_started_at = Some(now);
        self.text_editing.text_caret_next_frame_at = Some(
            now.checked_add(motion.frame_interval)
                .expect("hydrolysis text caret frame timestamp overflow"),
        );
    }

    pub(crate) const fn clear_text_caret_animation(&mut self) {
        self.text_editing.text_caret_fade_started_at = None;
        self.text_editing.text_caret_next_frame_at = None;
    }
}

impl HydrolysisRenderer {
    pub(crate) fn prepare_transient_text_input_overlay(
        &mut self,
        _env: &Environment,
        transform: kurbo::Affine,
    ) {
        let focused = self.text_editing.focused_index();
        let menu_target = self.active_text_context_menu_target();
        let mut scene = Recording::new();
        let theme = self.theme();
        {
            scene.record_picture(transform, |draw| {
                for (index, target) in self.text_editing.text_input_targets.iter().enumerate() {
                    if target.content_alpha <= 0.0 {
                        continue;
                    }
                    let selection_visible = focused == Some(index) || menu_target == Some(index);
                    if !selection_visible {
                        continue;
                    }
                    let selection = refreshed_target_selection(target);
                    let selection_paint = theme.input_selection_paint();
                    let caret_opacity = (selection.is_collapsed() && focused == Some(index))
                        .then(|| self.text_caret_opacity(self.frame_instant()));
                    let caret_paint = caret_opacity
                        .filter(|opacity| *opacity > 0.0)
                        .map(|opacity| theme.input_caret_paint(opacity));
                    draw.clip(target.text_clip_bounds, |draw| {
                        draw.group(
                            waterui_graphics::draw::Group::new().opacity(target.content_alpha),
                            |draw| {
                                if selection.is_collapsed() {
                                    if let Some(paint) = &caret_paint {
                                        draw.fill(target.cursor_area, paint.clone());
                                    }
                                } else {
                                    for (rect, _) in selection.geometry(&target.layout) {
                                        let highlight = kurbo::Rect::new(
                                            target.text_bounds.x0 + rect.x0,
                                            target.text_bounds.y0 + rect.y0,
                                            target.text_bounds.x0 + rect.x1,
                                            target.text_bounds.y0 + rect.y1,
                                        );
                                        draw.fill(highlight, selection_paint.clone());
                                    }
                                }
                            },
                        );
                    });
                }
            });
        }
        self.transient_scene = Some(scene);
    }
}

impl SemanticCore {
    pub(crate) fn advance_text_caret_animation(&mut self, now: Instant) -> bool {
        if !self.text_editing.has_focus() {
            return false;
        }
        let motion = self.text_caret_motion();
        let mut next = self
            .text_editing
            .text_caret_next_frame_at
            .unwrap_or_else(|| {
                self.reset_text_caret_animation(now);
                self.text_editing
                    .text_caret_next_frame_at
                    .expect("hydrolysis text caret animation state missing next frame timestamp")
            });
        if now < next {
            return false;
        }
        while now >= next {
            next = next
                .checked_add(motion.frame_interval)
                .expect("hydrolysis text caret frame timestamp overflow");
        }
        self.text_editing.text_caret_next_frame_at = Some(next);
        true
    }

    pub(crate) fn text_caret_opacity(&self, now: Instant) -> f32 {
        if !self.text_editing.has_focus() {
            return 0.0;
        }
        let motion = self.text_caret_motion();
        let started = self.text_editing.text_caret_fade_started_at.unwrap_or(now);
        let elapsed = now.saturating_duration_since(started);
        let cycle_secs = motion.fade_cycle_duration.as_secs_f32();
        assert!(
            cycle_secs > 0.0,
            "hydrolysis text caret fade cycle duration must be > 0"
        );
        let phase = (elapsed.as_secs_f32() / cycle_secs).fract();
        let wave = f32::midpoint((core::f32::consts::TAU * phase).cos(), 1.0);
        (1.0 - motion.min_opacity).mul_add(wave, motion.min_opacity)
    }

    /// Move focus to this frame's text input at `focused`, or clear it with
    /// `None`.
    ///
    /// The index is frame-local, so it is resolved to the target's stable
    /// [`InteractionKey`] here and only that key is retained. An index with no
    /// target is a call-site bug, not a recoverable state.
    pub(crate) fn set_focused_text_input(&mut self, focused: Option<usize>) -> bool {
        let key = focused.map(|index| {
            self.text_editing
                .key_at(index)
                .unwrap_or_else(|| {
                    panic!(
                        "hydrolysis text input focus index {index} has no target ({} emitted this frame)",
                        self.text_editing.text_input_targets.len()
                    )
                })
                .clone()
        });
        self.set_focused_text_input_key(key)
    }

    /// Move focus to the text input with this stable identity, or clear it with
    /// `None`. The target need not be emitted this frame; focus simply resolves
    /// to nothing until it is.
    /// Wires a `.focused(binding)` modifier to the single focusable target —
    /// a text field or an input surface — registered inside the spans
    /// (`text_start`, `embedded_start`): writes the binding onto that target,
    /// then applies the binding's value to the focused-input key. Shared by
    /// the rendered flush and the semantic accessibility walk.
    pub(crate) fn wire_focused_target(
        &mut self,
        value: &waterui::component::focus::Focused,
        should_focus: bool,
        text_start: usize,
        embedded_start: usize,
    ) {
        let text_count = self.text_editing.text_input_targets.len() - text_start;
        let embedded_count = self.hit_test.embedded_input_targets.len() - embedded_start;
        let focus_target_count = text_count + embedded_count;
        assert!(
            focus_target_count == 1,
            "hydrolysis .focused() requires exactly one TextField or SecureField or input surface in the wrapped subtree, found {focus_target_count}"
        );
        if embedded_count == 1 {
            let target = self
                .hit_test
                .embedded_input_targets
                .get_mut(embedded_start)
                .expect("hydrolysis focused metadata missing registered surface input target");
            assert!(
                target.focus_binding.is_none(),
                "hydrolysis does not allow multiple .focused() modifiers to target the same control"
            );
            target.focus_binding = Some(value.0.clone());
            let target_key = target.interaction_key.clone();

            if should_focus {
                self.set_focused_embedded_key(Some(target_key));
            } else if self.is_focused_embedded(&target_key) {
                self.set_focused_embedded_key(None);
            }
            return;
        }
        let target = self
            .text_editing
            .text_input_targets
            .get_mut(text_start)
            .expect("hydrolysis focused metadata missing registered text input target");
        assert!(
            target.focus_binding.is_none(),
            "hydrolysis does not allow multiple .focused() modifiers to target the same control"
        );
        target.focus_binding = Some(value.0.clone());
        let target_key = target.interaction_key.clone();

        if should_focus {
            self.set_focused_text_input_key(Some(target_key));
        } else if self.text_editing.is_focused(&target_key) {
            self.set_focused_text_input_key(None);
        }
    }

    pub(crate) fn set_focused_text_input_key(&mut self, focused: Option<InteractionKey>) -> bool {
        let previous = self.text_editing.focused_key();
        let mut changed = false;
        match focused.as_ref() {
            // A field taking UI focus takes the semantic focus with it —
            // the tree reports focus on the field's node.
            Some(key) if previous.as_ref() != Some(key) => {
                #[cfg(feature = "accessibility")]
                let node = self.focus_node_for_key(key);
                changed |= self.set_keyboard_focus_impl(
                    Some(key.clone()),
                    #[cfg(feature = "accessibility")]
                    node,
                    self.hit_test.keyboard_focus_visible,
                );
            }
            // The same field re-asserted: repair only a stale link — keyboard
            // focus still claims the key while its node went un-emitted when
            // the link was made. Semantic focus sitting on another node is a
            // legitimate move, not staleness.
            #[cfg(feature = "accessibility")]
            Some(key)
                if self.hit_test.keyboard_focus.as_ref() == Some(key)
                    && self
                        .focus_node_for_key(key)
                        .is_some_and(|node| self.accessibility.focus != node) =>
            {
                let node = self.focus_node_for_key(key);
                changed |= self.set_keyboard_focus_impl(
                    Some(key.clone()),
                    node,
                    self.hit_test.keyboard_focus_visible,
                );
            }
            Some(_) => {}
            // Clearing UI focus drops the semantic focus only when the tree
            // still rests on the cleared field. A focus move that already
            // landed elsewhere — traversal ends editing after re-targeting
            // semantic focus — is left alone.
            None if previous.is_some() && self.hit_test.keyboard_focus == previous => {
                changed |= self.set_keyboard_focus_impl(
                    None,
                    #[cfg(feature = "accessibility")]
                    None,
                    false,
                );
            }
            None => {}
        }
        if previous == focused {
            return changed;
        }
        let previous_binding = self.text_editing.focused_binding.take();
        // The outgoing target may already be truncated or unmounted, so the
        // `.focused` binding is captured while the target is emitted and
        // replayed from the slot — resolving it now would find nothing.
        let next_binding = focused
            .as_ref()
            .and_then(|key| self.text_editing.index_of(key))
            .and_then(|index| {
                self.text_editing.text_input_targets[index]
                    .focus_binding
                    .clone()
            });
        if let Some(binding) = previous_binding {
            binding.set(false);
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            previous_focus = ?previous,
            next_focus = ?focused,
            "text input focus changed"
        );
        let focused_something = focused.is_some();
        self.text_editing.store_focused_key(focused);
        self.text_editing.focused_binding.clone_from(&next_binding);
        if let Some(binding) = next_binding {
            binding.set(true);
        }
        if focused_something {
            // A field taking focus releases a surface's — the counterpart
            // of the rule that landing on a surface ends editing.
            let _ = self.hit_test.set_embedded_focus_index(None);
        }
        self.text_editing.active_text_selection_drag = None;
        self.text_editing.take_ime_preedit();
        if focused_something {
            self.reset_text_caret_animation(self.frame_instant());
        } else {
            self.clear_text_caret_animation();
            self.dismiss_active_text_context_menu();
            self.dismiss_active_popup_menu();
        }
        self.request_refresh();
        true
    }

    pub(crate) fn dismiss_active_text_context_menu(&mut self) {
        if let Some(menu) = self.text_editing.active_text_context_menu.take() {
            match menu {
                ActiveTextContextMenu::Overlay { overlay, .. } => {
                    overlay.menu_group.close_all();
                    if self
                        .popup_menu
                        .active_popup_menu_group
                        .as_ref()
                        .is_some_and(|group| Rc::ptr_eq(&group.0, &overlay.menu_group.0))
                    {
                        self.popup_menu.active_popup_menu_group = None;
                    }
                    self.request_refresh();
                }
                ActiveTextContextMenu::NativeWindow { group, .. } => {
                    group.close_all();
                    if self
                        .popup_menu
                        .active_popup_menu_group
                        .as_ref()
                        .is_some_and(|active| Rc::ptr_eq(&active.0, &group.0))
                    {
                        self.popup_menu.active_popup_menu_group = None;
                    }
                }
            }
        }
    }

    /// This frame's position of the input whose context menu is open, if that
    /// input is still emitted.
    pub(crate) fn active_text_context_menu_target(&self) -> Option<usize> {
        let key = match self.text_editing.active_text_context_menu.as_ref()? {
            ActiveTextContextMenu::Overlay { target, .. }
            | ActiveTextContextMenu::NativeWindow { target, .. } => target,
        };
        self.text_editing.index_of(key)
    }
}

impl HydrolysisRenderer {
    pub(crate) fn render_active_text_context_menu_overlay(
        &mut self,
        env: &Environment,
        transform: kurbo::Affine,
    ) {
        let Some(ActiveTextContextMenu::Overlay { overlay, .. }) =
            self.text_editing.active_text_context_menu.clone()
        else {
            return;
        };
        if overlay.dismiss_state.snapshot() == WindowState::Closed {
            self.dismiss_active_text_context_menu();
            return;
        }

        let theme = self.theme();
        let metrics = theme.text_context_menu_metrics();
        {
            self.scene.record_picture(transform, |draw| {
                theme.draw_text_context_menu_panel(&mut *draw, overlay.bounds);
            });
        }
        for (index, row) in overlay.rows.iter().enumerate() {
            let next_is_divider = overlay
                .rows
                .as_slice()
                .get(index + 1)
                .is_some_and(|next| matches!(next.node, PopupMenuNode::Divider));
            if index + 1 < overlay.rows.len()
                && !matches!(row.node, PopupMenuNode::Divider)
                && !next_is_divider
            {
                let separator = kurbo::Rect::new(
                    row.bounds.x0 + metrics.separator_horizontal_inset,
                    row.bounds.y1 - metrics.separator_thickness,
                    row.bounds.x1 - metrics.separator_horizontal_inset,
                    row.bounds.y1,
                );
                self.scene.record_picture(transform, |draw| {
                    theme.draw_text_context_menu_separator(&mut *draw, separator);
                });
            }

            match &row.node {
                PopupMenuNode::Command { plain_label, .. }
                | PopupMenuNode::Menu { plain_label, .. } => {
                    let text_rect = inset_rect(
                        row.bounds,
                        metrics.horizontal_padding,
                        metrics.vertical_padding,
                    );
                    let ctx = RenderContext {
                        transform,
                        hit_transform: kurbo::Affine::IDENTITY,
                        bounds: overlay.bounds,
                    }
                    .child(
                        kurbo::Affine::translate((text_rect.x0, text_rect.y0)),
                        kurbo::Rect::new(0.0, 0.0, text_rect.width(), text_rect.height()),
                    );
                    let (state, scene) = self.state_and_scene_mut();
                    Self::render_styled_text(
                        state,
                        scene,
                        ctx,
                        StyledStr::plain(plain_label.clone()),
                        HorizontalAlignment::Leading,
                        env,
                    );
                }
                PopupMenuNode::Divider => {
                    let separator = kurbo::Rect::new(
                        row.bounds.x0 + metrics.separator_horizontal_inset,
                        metrics
                            .separator_thickness
                            .mul_add(-0.5, f64::mul_add(row.bounds.height(), 0.5, row.bounds.y0)),
                        row.bounds.x1 - metrics.separator_horizontal_inset,
                        metrics
                            .separator_thickness
                            .mul_add(0.5, f64::mul_add(row.bounds.height(), 0.5, row.bounds.y0)),
                    );
                    self.scene.record_picture(transform, |draw| {
                        theme.draw_text_context_menu_separator(&mut *draw, separator);
                    });
                }
            }
        }
    }
}

impl SemanticCore {
    pub(crate) fn handle_text_context_menu_overlay_pointer_down(
        &mut self,
        point: kurbo::Point,
    ) -> bool {
        let Some(ActiveTextContextMenu::Overlay { overlay, .. }) =
            self.text_editing.active_text_context_menu.clone()
        else {
            return false;
        };
        if overlay.dismiss_state.snapshot() == WindowState::Closed {
            self.dismiss_active_text_context_menu();
            return false;
        }
        if !overlay.bounds.contains(point) {
            self.dismiss_active_text_context_menu();
            return false;
        }
        for row in &overlay.rows {
            if !row.bounds.contains(point) {
                continue;
            }
            match &row.node {
                PopupMenuNode::Command {
                    action, disabled, ..
                } => {
                    if disabled.snapshot() {
                        return false;
                    }
                    call_action_discarding_result(action, &overlay.env);
                    self.dismiss_active_text_context_menu();
                    return true;
                }
                PopupMenuNode::Divider => return false,
                PopupMenuNode::Menu { items, .. } => {
                    if items.is_empty() {
                        return false;
                    }
                    self.open_text_context_menu_submenu(&overlay, row.bounds, items.clone());
                    return true;
                }
            }
        }
        true
    }

    /// Opens `items` — a selection-menu row's nested `Menu` — as a submenu
    /// popup window anchored to the row's trailing edge, through the same
    /// [`popup_menu_window`] path a `.context_menu` submenu takes. The window
    /// joins the overlay's menu group at depth 1: the overlay's dismiss
    /// sentinel holds depth 0, standing in for the root window.
    fn open_text_context_menu_submenu(
        &mut self,
        overlay: &TextContextMenuOverlay,
        row_bounds: kurbo::Rect,
        items: Vec<PopupMenuNode>,
    ) {
        let env = &overlay.env;
        let theme = Rc::clone(&overlay.theme);
        let metrics = theme.text_context_menu_metrics();
        let text = self.popup_menu_text_metrics(&items, metrics, env, &theme);
        let origin = popup_window_origin(
            LayoutPoint::new(
                crate::num_cast::f64_as_f32(row_bounds.x1),
                crate::num_cast::f64_as_f32(row_bounds.y0),
            ),
            env,
        );
        let group = overlay.menu_group.clone();
        group.truncate(1);
        let (window, state) =
            popup_menu_window(items, origin, group.clone(), 1, metrics, text, &theme);
        group.push(state);
        env.get::<PopupWindowManager>()
            .expect("hydrolysis text selection menus require PopupWindowManager in environment")
            .show(window, env);
        self.popup_menu.active_popup_menu_group = Some(group);
        self.request_refresh();
    }

    pub(crate) fn focused_text_target_data(
        &mut self,
    ) -> Option<(usize, TextInputModel, Rc<RefCell<TextSelectionSlot>>)> {
        // Input handling runs between frames, when the target list is complete:
        // a focused identity that resolves to nothing here is genuinely gone.
        let Some(index) = self.text_editing.focused_index() else {
            self.set_focused_text_input_key(None);
            return None;
        };
        let target = &self.text_editing.text_input_targets[index];
        Some((index, target.model.clone(), Rc::clone(&target.selection)))
    }

    /// The focused text input's authoritative editing state for a platform
    /// editing session (Android's `InputConnection` mirror): committed text
    /// and selection in byte offsets, the live pre-edit, the field's input
    /// constraints and the geometry an IME draws around the caret.
    #[cfg_attr(
        not(any(target_os = "android", test)),
        allow(
            dead_code,
            reason = "read by the Android editing session and its tests"
        )
    )]
    pub(crate) fn focused_editor_snapshot(&self) -> Option<FocusedEditorSnapshot> {
        let index = self.text_editing.focused_index()?;
        let target = &self.text_editing.text_input_targets[index];
        let text = target.model.plain_text();
        let slot = target.selection.borrow();
        Some(FocusedEditorSnapshot {
            key: target.interaction_key.clone(),
            anchor: clamp_to_char_boundary(text.as_str(), slot.anchor),
            focus: clamp_to_char_boundary(text.as_str(), slot.focus),
            text,
            preedit: self
                .text_editing
                .ime_preedit
                .as_ref()
                .map(ToString::to_string),
            preedit_caret: self.text_editing.ime_preedit_caret,
            password: target.model.is_secure(),
            line_limit: target.model.line_limit(),
            has_submit: matches!(
                &target.model,
                TextInputModel::TextField {
                    on_submit: Some(_),
                    ..
                }
            ),
            bounds: target.bounds,
            cursor_area: target.cursor_area,
            text_bounds: target.text_bounds,
            display_text: target.display_text.to_string(),
            display_layout: std::sync::Arc::clone(&target.display_layout),
        })
    }

    /// Write an editing session's projection back into the focused model in
    /// one step: committed text, selection slot and pre-edit, in the order
    /// [`replace_model_selection`] publishes them (selection before text so a
    /// refresh sees the caret that belongs to the edited value). The pre-edit
    /// routes through [`Self::handle_ime_preedit`], so a password-purpose
    /// target still refuses marked text.
    #[cfg_attr(
        not(any(target_os = "android", test)),
        allow(
            dead_code,
            reason = "written by the Android editing session and its tests"
        )
    )]
    pub(crate) fn apply_editor_projection(
        &mut self,
        committed: &str,
        anchor: usize,
        focus: usize,
        preedit: Option<(&str, Option<usize>)>,
    ) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        // The projection goes through the same normalization the text-insert
        // path applies — carriage returns never reach a model, and a
        // single-line field never grows a newline — and the same line-limit
        // refusal, so a platform write cannot exceed what a keypress could.
        let committed = normalized_insert_text(committed, model.line_limit());
        let mut changed = false;
        {
            let mut slot = selection.borrow_mut();
            let anchor = clamp_to_char_boundary(committed.as_str(), anchor);
            let focus = clamp_to_char_boundary(committed.as_str(), focus);
            changed |= slot.anchor != anchor || slot.focus != focus || !slot.initialized;
            slot.anchor = anchor;
            slot.focus = focus;
            slot.initialized = true;
            if model.plain_text() != committed
                && !exceeds_line_limit(committed.as_str(), model.line_limit())
            {
                model.set_plain_text(committed);
                changed = true;
            }
        }
        let (preedit_text, preedit_caret) =
            preedit.map_or(("", None), |(text, caret)| (text, caret));
        changed |= self.handle_ime_preedit(preedit_text, preedit_caret);
        if changed {
            self.reset_text_caret_animation(self.frame_instant());
        }
        changed
    }

    /// A line-limited field's submit action, if its focused target declares
    /// one — the `on_submit` an editor action (Return, `IME_ACTION_DONE`)
    /// fires. Extracted from [`Self::handle_key`] so a platform editing
    /// session submits identically.
    pub(crate) fn perform_editor_submit(&self) -> bool {
        let submit = self
            .text_editing
            .focused_target()
            .and_then(|target| match &target.model {
                TextInputModel::TextField {
                    line_limit: Some(_),
                    on_submit: Some(action),
                    ..
                } => Some((action.clone(), target.env.clone())),
                _ => None,
            });
        let Some((action, env)) = submit else {
            return false;
        };
        action.call(&env);
        true
    }

    /// A context-menu editing action (select-all/cut/copy/paste) addressed at
    /// the focused target — the same primitives the rendered menu executes,
    /// for a platform `InputConnection`'s `performContextMenuAction`.
    #[cfg_attr(
        not(any(target_os = "android", test)),
        allow(
            dead_code,
            reason = "written by the Android editing session and its tests"
        )
    )]
    pub(crate) fn perform_focused_context_action(&mut self, action: TextContextMenuAction) -> bool {
        let Some((index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        // The rendered menu never offers copy/cut on a secure field; a
        // platform connection can send the action anyway, so the guard
        // belongs here too — a secret never reaches the clipboard.
        if model.is_secure()
            && matches!(
                action,
                TextContextMenuAction::Copy | TextContextMenuAction::Cut
            )
        {
            return false;
        }
        let env = self.text_editing.text_input_targets[index].env.clone();
        execute_text_context_menu_action(action, &model, &selection, &env)
    }

    pub(crate) fn text_selection_index_from_point(
        target: &TextInputTarget,
        point: kurbo::Point,
    ) -> usize {
        let local_x = crate::num_cast::f64_as_f32(point.x - target.text_bounds.x0);
        let local_y = crate::num_cast::f64_as_f32(point.y - target.text_bounds.y0);
        let selection =
            parley::Selection::from_point(&target.layout, local_x, local_y).refresh(&target.layout);
        target
            .model
            .plain_index_from_layout_index(selection.focus().index())
    }

    pub(crate) fn text_selection_range_from_point_with_click_count(
        target: &TextInputTarget,
        point: kurbo::Point,
        click_count: u8,
    ) -> (usize, usize) {
        let local_x = crate::num_cast::f64_as_f32(point.x - target.text_bounds.x0);
        let local_y = crate::num_cast::f64_as_f32(point.y - target.text_bounds.y0);
        let selection = match click_count {
            2 => parley::Selection::word_from_point(&target.layout, local_x, local_y),
            3.. => parley::Selection::line_from_point(&target.layout, local_x, local_y),
            _ => parley::Selection::from_point(&target.layout, local_x, local_y),
        }
        .refresh(&target.layout);
        (
            target
                .model
                .plain_index_from_layout_index(selection.anchor().index()),
            target
                .model
                .plain_index_from_layout_index(selection.focus().index()),
        )
    }

    /// Advance the double/triple-click streak for this frame's target at
    /// `target_index`. The streak is remembered by stable identity, so it
    /// continues across a reflow and never carries over to a different field
    /// that happens to land at the same position.
    pub(crate) fn next_text_selection_click_count(
        &mut self,
        target_index: usize,
        point: kurbo::Point,
        at: Instant,
    ) -> u8 {
        let target = self
            .text_editing
            .key_at(target_index)
            .expect("hydrolysis text selection click target must be emitted this frame")
            .clone();
        let count = match self.text_editing.last_text_selection_click.as_ref() {
            Some(previous)
                if previous.target == target
                    && at.saturating_duration_since(previous.at)
                        <= TEXT_SELECTION_MULTI_CLICK_INTERVAL
                    && previous.point.distance(point) <= TEXT_SELECTION_MULTI_CLICK_DISTANCE =>
            {
                previous.count.saturating_add(1).min(3)
            }
            _ => 1,
        };
        self.text_editing.last_text_selection_click = Some(TextSelectionClickState {
            target,
            point,
            at,
            count,
        });
        count
    }

    /// Apply the selection for a click of `click_count` at `point`. Returns the
    /// applied (anchor, focus) range plus whether the slot changed, so the
    /// caller can arm [`ActiveTextSelectionDrag`] with the same range as its
    /// anchor side.
    pub(crate) fn apply_text_selection_click_gesture(
        &mut self,
        index: usize,
        point: kurbo::Point,
        click_count: u8,
    ) -> Option<(usize, usize, bool)> {
        let Some(target) = self.text_editing.text_input_targets.as_slice().get(index) else {
            self.text_editing.active_text_selection_drag = None;
            return None;
        };
        let (anchor, focus) =
            Self::text_selection_range_from_point_with_click_count(target, point, click_count);
        let mut slot = target.selection.borrow_mut();
        let changed = slot.anchor != anchor || slot.focus != focus || !slot.initialized;
        slot.anchor = anchor;
        slot.focus = focus;
        slot.initialized = true;
        Some((anchor, focus, changed))
    }

    /// Extend an in-flight selection drag to `point`. The drag remembers the
    /// click streak that armed it: single clicks extend at caret granularity,
    /// while a double/triple-click drag keeps the word/line it snapped to as
    /// the anchor and extends by whole units — so the pointer release (or a
    /// sub-pixel jiggle inside the same word) cannot collapse the gesture's
    /// selection back to a caret. Mirrors parley's `Selection::extend_to_point`
    /// in plain-index space.
    pub(crate) fn update_text_selection_drag(&mut self, index: usize, point: kurbo::Point) -> bool {
        let Some(drag) = self.text_editing.active_text_selection_drag.clone() else {
            return false;
        };
        let Some(target) = self.text_editing.text_input_targets.as_slice().get(index) else {
            self.text_editing.active_text_selection_drag = None;
            return false;
        };
        let (anchor, focus) = if drag.click_count <= 1 {
            (
                drag.anchor,
                Self::text_selection_index_from_point(target, point),
            )
        } else {
            let (target_anchor, target_focus) =
                Self::text_selection_range_from_point_with_click_count(
                    target,
                    point,
                    drag.click_count,
                );
            // Same merge parley's `extend_selection` performs: union of the
            // hovered unit and the armed anchor range, with the anchor kept on
            // the side opposite the drag direction.
            let extending_right = target_anchor >= drag.anchor;
            let min = drag
                .anchor
                .min(drag.focus)
                .min(target_anchor.min(target_focus));
            let max = drag
                .anchor
                .max(drag.focus)
                .max(target_anchor.max(target_focus));
            if extending_right {
                (min, max)
            } else {
                (max, min)
            }
        };
        let mut slot = target.selection.borrow_mut();
        let changed = slot.anchor != anchor || slot.focus != focus || !slot.initialized;
        slot.anchor = anchor;
        slot.focus = focus;
        slot.initialized = true;
        changed
    }

    pub(crate) fn update_text_selection_from_pointer(
        &mut self,
        index: usize,
        point: kurbo::Point,
        extend: bool,
    ) -> bool {
        let Some(target) = self.text_editing.text_input_targets.as_slice().get(index) else {
            self.text_editing.active_text_selection_drag = None;
            return false;
        };
        let next_index = Self::text_selection_index_from_point(target, point);
        let mut slot = target.selection.borrow_mut();
        if !extend || !slot.initialized {
            let changed =
                slot.anchor != next_index || slot.focus != next_index || !slot.initialized;
            slot.anchor = next_index;
            slot.focus = next_index;
            slot.initialized = true;
            return changed;
        }
        let changed = slot.focus != next_index || !slot.initialized;
        slot.focus = next_index;
        slot.initialized = true;
        changed
    }

    pub(crate) fn insert_text_into_focused_target(&mut self, text: &str) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        replace_model_selection(&model, &mut slot, text)
    }

    pub(crate) fn delete_backward_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        delete_model_backward(&model, &mut slot)
    }

    pub(crate) fn delete_forward_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        delete_model_forward(&model, &mut slot)
    }

    pub(crate) fn select_all_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        select_all_model_text(&model, &mut slot)
    }

    pub(crate) fn copy_selection_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        if model.is_secure() {
            return false;
        }
        let slot = selection.borrow();
        let Some(selected) = selected_text_for_model(&model, &slot) else {
            return false;
        };
        write_clipboard_text(selected.as_str())
    }

    pub(crate) fn cut_selection_in_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        if model.is_secure() {
            return false;
        }
        let mut slot = selection.borrow_mut();
        let Some(selected) = selected_text_for_model(&model, &slot) else {
            return false;
        };
        if !write_clipboard_text(selected.as_str()) {
            return false;
        }
        delete_model_selection(&model, &mut slot)
    }

    pub(crate) fn paste_clipboard_into_focused_target(&mut self) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        spawn_clipboard_paste_task(model, selection);
        true
    }

    pub(crate) fn move_focused_caret_horizontal(&mut self, backward: bool, extend: bool) -> bool {
        let Some((index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let target = &self.text_editing.text_input_targets[index];
        let mut slot = selection.borrow_mut();
        let current = selection_for_target_layout(&model, &target.layout, &slot);
        let next = if backward {
            current.previous_visual(&target.layout, extend)
        } else {
            current.next_visual(&target.layout, extend)
        };
        let anchor = model.plain_index_from_layout_index(next.anchor().index());
        let focus = model.plain_index_from_layout_index(next.focus().index());
        let changed = slot.anchor != anchor || slot.focus != focus || !slot.initialized;
        slot.anchor = anchor;
        slot.focus = focus;
        slot.initialized = true;
        changed
    }

    pub(crate) fn move_focused_caret_to_boundary(&mut self, end: bool, extend: bool) -> bool {
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        let text = model.plain_text();
        let next_index = if end { text.len() } else { 0 };
        if extend {
            let next_index = clamp_to_char_boundary(text.as_str(), next_index);
            let changed = slot.focus != next_index || !slot.initialized;
            slot.focus = next_index;
            slot.initialized = true;
            return changed;
        }
        set_model_caret_position(&model, &mut slot, next_index)
    }

    /// The selection menu's rows as [`PopupMenuNode`]s: built-in editing
    /// commands become plain command rows, the field's custom
    /// `selection_menu` items go through the same [`popup_menu_node`]
    /// conversion `.context_menu` items take — a nested `Menu` keeps its
    /// structure and opens as a submenu rather than flattening or panicking.
    pub(crate) fn build_text_context_menu_nodes(
        target: &TextInputTarget,
        env: &Environment,
    ) -> Vec<PopupMenuNode> {
        let has_selection = {
            let slot = target.selection.borrow();
            selected_text_for_model(&target.model, &slot).is_some()
        };
        let has_text = !target.model.plain_text().is_empty();
        let mut nodes = Vec::new();
        let builtin = |key: &str, action| {
            text_context_menu_builtin_node(
                crate::localization::text(env, key),
                action,
                &target.model,
                &target.selection,
            )
        };
        if has_selection && !target.model.is_secure() {
            nodes.push(builtin("copy", TextContextMenuAction::Copy));
            nodes.push(builtin("cut", TextContextMenuAction::Cut));
        }
        nodes.push(builtin("paste", TextContextMenuAction::Paste));
        if has_text {
            nodes.push(builtin("select_all", TextContextMenuAction::SelectAll));
        }
        if has_selection {
            nodes.extend(
                target
                    .model
                    .custom_selection_menu_items()
                    .into_iter()
                    .map(crate::renderer::views::popup_menu_node),
            );
        }
        nodes
    }
}

impl HydrolysisRenderer {
    pub(crate) fn show_text_context_menu(
        &mut self,
        index: usize,
        point: kurbo::Point,
        env: &Environment,
    ) -> bool {
        let Some(target) = self
            .text_editing
            .text_input_targets
            .as_slice()
            .get(index)
            .cloned()
        else {
            return false;
        };
        let target_key = target.interaction_key.clone();
        // The menu opens in the registering view's environment layered over
        // this dispatch's, so `.state(&value)` overlays reach the item
        // actions (water-rs/hydrolysis#140).
        let menu_env = target.env.layered_on(env);
        let nodes = SemanticCore::build_text_context_menu_nodes(&target, &menu_env);
        if nodes.is_empty() {
            self.dismiss_active_text_context_menu();
            return false;
        }

        self.dismiss_active_text_context_menu();
        let mode = menu_env
            .get::<HydrolysisTextContextMenuMode>()
            .copied()
            .unwrap_or(HydrolysisTextContextMenuMode::NativeWindow);

        if mode == HydrolysisTextContextMenuMode::Overlay {
            let metrics = self.theme().text_context_menu_metrics();
            let bounds =
                text_context_menu_overlay_bounds(point, &nodes, self.window_bounds, metrics);
            let mut rows = Vec::with_capacity(nodes.len());
            for (index, node) in nodes.into_iter().enumerate() {
                let y0 = metrics
                    .row_height
                    .mul_add(crate::num_cast::usize_as_f64(index), bounds.y0);
                let row_bounds =
                    kurbo::Rect::new(bounds.x0, y0, bounds.x1, y0 + metrics.row_height);
                rows.push(TextContextMenuOverlayRow {
                    bounds: row_bounds,
                    node,
                });
            }
            let menu_group = PopupMenuStateGroup::new();
            let dismiss_state = nami::Binding::container(WindowState::Normal);
            menu_group.push(dismiss_state.clone());
            self.text_editing.active_text_context_menu = Some(ActiveTextContextMenu::Overlay {
                target: target_key,
                overlay: TextContextMenuOverlay {
                    bounds,
                    rows,
                    menu_group,
                    dismiss_state,
                    theme: self.theme(),
                    env: menu_env,
                },
            });
            self.request_refresh();
            return true;
        }

        // The windowed presentation mounts the nodes through the same popup
        // path a `.context_menu` takes: `show_popup_menu_nodes` sizes the
        // panel, builds the borderless window, and leaves submenu rows
        // wired to open deeper popups (water-rs/hydrolysis#317).
        let metrics = self.theme().text_context_menu_metrics();
        let theme = self.theme();
        self.show_popup_menu_nodes(
            nodes,
            LayoutPoint::new(
                crate::num_cast::f64_as_f32(point.x),
                crate::num_cast::f64_as_f32(point.y),
            ),
            metrics,
            &menu_env,
            &theme,
        );
        let group = self
            .popup_menu
            .active_popup_menu_group
            .clone()
            .expect("show_popup_menu_nodes leaves the opened menu's group registered");
        self.text_editing.active_text_context_menu = Some(ActiveTextContextMenu::NativeWindow {
            target: target_key,
            group,
        });
        true
    }
}

impl SemanticCore {
    pub fn handle_text_input(&mut self, text: &str) -> bool {
        let preedit_cleared = self.text_editing.take_ime_preedit().is_some();
        if text.is_empty() {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                focused = ?self.text_editing.focused_key(),
                preedit_cleared,
                "text input ignored empty payload"
            );
            return preedit_cleared;
        }
        let changed = self.insert_text_into_focused_target(text) || preedit_cleared;
        if changed {
            self.reset_text_caret_animation(self.frame_instant());
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            focused = ?self.text_editing.focused_key(),
            text = text,
            changed,
            "text input handled"
        );
        changed
    }

    pub fn handle_ime_preedit(&mut self, text: &str, caret: Option<usize>) -> bool {
        // A password-purpose field is not IME-allowed: the platform should
        // never mark one up, and a stray preedit must not draw extra mask
        // glyphs that leak the composition's length.
        let ime_allowed = self
            .text_editing
            .focused_target()
            .is_some_and(|target| target.purpose != TextInputPurpose::Password);
        if !ime_allowed {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                text = text,
                "ime preedit dropped without an ime-allowed focused text input"
            );
            return false;
        }
        let (next, next_caret) = if text.is_empty() {
            (None, None)
        } else {
            (Some(Str::from(text.to_owned())), caret)
        };
        if self.text_editing.ime_preedit == next
            && self.text_editing.ime_preedit_caret == next_caret
        {
            return false;
        }
        self.text_editing.ime_preedit = next;
        self.text_editing.ime_preedit_caret = next_caret;
        self.reset_text_caret_animation(self.frame_instant());
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            focused = ?self.text_editing.focused_key(),
            preedit = ?self.text_editing.ime_preedit,
            caret = ?self.text_editing.ime_preedit_caret,
            "ime preedit updated"
        );
        true
    }

    pub fn handle_ime_commit(&mut self, text: &str) -> bool {
        self.handle_text_input(text)
    }

    pub fn handle_ime_disabled(&mut self) -> bool {
        let changed = self.text_editing.take_ime_preedit().is_some();
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            changed,
            "ime disabled handled"
        );
        changed
    }

    pub fn handle_key_with_env(
        &mut self,
        key: &KeyCode,
        modifiers: Modifiers,
        env: &Environment,
    ) -> bool {
        // Callers without platform key data (tests, synthetic input) get a
        // `KeyPress` rebuilt from the `KeyCode` — the winit/semantic paths
        // carry real `logical_key`/`physical_code` and call
        // `handle_key_press` instead.
        let press = KeyPress {
            key: key.to_w3c_key(),
            code: keyboard_types::Code::Unidentified,
            modifiers: modifiers.into(),
            repeat: false,
        };
        self.handle_key_press(key, modifiers, env, &press) != KeyPressOutcome::Ignored
    }

    /// A key press with its full platform identity: the focused target's
    /// editing first, then the `OnKeyPress` bubble chain, then an enclosing
    /// embedded surface.
    ///
    /// The [`KeyPressOutcome`] distinguishes a press a handler consumed
    /// (whose paired `KeyText` is then suppressed at dispatch) from one
    /// forwarded to an embedded surface — delivery is not consumption: the
    /// surface owns its key+text pair, exactly as `SurfaceInputEvent`
    /// documents.
    pub(crate) fn handle_key_press(
        &mut self,
        key: &KeyCode,
        modifiers: Modifiers,
        env: &Environment,
        press: &KeyPress,
    ) -> KeyPressOutcome {
        if self.handle_keyboard_key_down(key, modifiers, env) {
            return KeyPressOutcome::Consumed;
        }
        if self.handle_key(key, modifiers) {
            return KeyPressOutcome::Consumed;
        }
        self.bubble_key_press(press)
    }

    /// Offers an unconsumed key to the focused node's `OnKeyPress`
    /// ancestors, nearest first, then to the topmost embedded surface
    /// enclosing it. [`KeyPressOutcome::Consumed`] once some scope reports
    /// [`KeyHandling::Handled`]; [`KeyPressOutcome::ForwardedToSurface`] when
    /// a surface takes the key.
    ///
    /// The focused node is whatever `hit_test.keyboard_focus` names — a text
    /// input, a focusable control's press slot, or an embedded surface; with
    /// no focus at all the key still bubbles through the scopes enclosing
    /// every registration of the frame.
    fn bubble_key_press(&mut self, press: &KeyPress) -> KeyPressOutcome {
        let focused_key = self.hit_test.keyboard_focus.clone();
        let mut scopes: Option<Option<Rc<KeyHandlerNode>>> = None;
        let mut bubble_center: Option<kurbo::Point> = None;
        if let Some(target) = self.text_editing.focused_target() {
            scopes = Some(target.key_handlers.clone());
            bubble_center = Some(target.bounds.center());
        } else if let Some(key) = focused_key.as_ref() {
            if let Some(target) = self.hit_test.pointer_targets.iter().find(|target| {
                target
                    .press_slot
                    .as_ref()
                    .is_some_and(|slot| &slot.key == key)
            }) {
                scopes = Some(target.key_handlers.clone());
                bubble_center = Some(target.bounds.center());
            } else if let Some(target) = self
                .hit_test
                .embedded_input_targets
                .iter()
                .find(|target| &target.interaction_key == key)
            {
                scopes = Some(target.key_handlers.clone());
                bubble_center = Some(target.to_window_rect(target.local_bounds).center());
            } else {
                // The semantic walk emits no targets for the key to resolve
                // against — the chain recorded at the focus link stands in.
                #[cfg(feature = "accessibility")]
                {
                    scopes = self.accessibility.focus_key_handlers.get(key).cloned();
                }
            }
        }
        // `scopes` wraps a chain head: a target with no scopes is
        // `Some(None)`; no resolved target at all falls back to the
        // every-registration chain.
        let mut node = scopes.unwrap_or_else(|| self.hit_test.root_key_handlers.clone());
        // `node` is moved into the while-let pattern each pass, so a fresh
        // clone rebinds it — `clone_from` needs a live target binding.
        #[allow(clippy::assigning_clones)]
        while let Some(link) = node {
            let handler = Rc::clone(&link.scope.handler);
            let env = link.scope.env.extending(press.clone());
            let result = handler.borrow_mut().handle(&env);
            if result == KeyHandling::Handled {
                tracing::trace!(
                    target: "waterui::hydrolysis::input",
                    key = ?press.key,
                    "key consumed by an on_key_press ancestor"
                );
                return KeyPressOutcome::Consumed;
            }
            node = link.parent.clone();
        }
        // Nothing above the focused node consumed it: the embedding surface
        // under it (e.g. a terminal under a search overlay) gets the key next.
        if let Some(center) = bubble_center
            && let Some((index, _)) = self.topmost_embedded_target_at(center)
        {
            let embedded = self.hit_test.embedded_input_targets[index].clone();
            embedded.sink.key(&KeyDelivery {
                pressed: true,
                logical: &press.key,
                code: press.code,
                repeat: press.repeat,
                modifiers: Modifiers::from(press.modifiers),
            });
            // The release belongs to the sink that saw the press, not to
            // whichever surface holds focus when it arrives.
            self.hit_test.bubbled_key_sinks.push(BubbledKeySink {
                logical: press.key.clone(),
                code: press.code,
                modifiers: Modifiers::from(press.modifiers),
                sink: embedded.sink,
            });
            // Forwarded, not consumed: the surface owns the key+text pair
            // and decides internally what the press meant, so its paired
            // `TextInput` is still delivered.
            return KeyPressOutcome::ForwardedToSurface;
        }
        KeyPressOutcome::Ignored
    }

    pub fn handle_key_release_with_env(&mut self, key: &KeyCode, env: &Environment) -> bool {
        self.handle_keyboard_key_up(key, env)
    }

    pub fn handle_key(&mut self, key: &KeyCode, modifiers: Modifiers) -> bool {
        if !self.text_editing.has_focus() {
            return false;
        }

        if modifiers.alt {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                key = ?key,
                modifiers = ?modifiers,
                "key ignored due command modifiers"
            );
            return false;
        }

        let command_modifier = modifiers.control || modifiers.super_key;
        let changed = if command_modifier {
            match key {
                KeyCode::Character(value) => match value.to_ascii_lowercase().as_str() {
                    "a" => self.select_all_in_focused_target(),
                    "c" => self.copy_selection_in_focused_target(),
                    "x" => self.cut_selection_in_focused_target(),
                    "v" => self.paste_clipboard_into_focused_target(),
                    _ => false,
                },
                KeyCode::Named(value) if value == "Escape" => {
                    let changed = self.text_editing.active_text_context_menu.is_some()
                        || self.active_popup_menu_visible();
                    self.dismiss_active_text_context_menu();
                    self.dismiss_active_popup_menu();
                    changed
                }
                _ => false,
            }
        } else {
            match key {
                KeyCode::Named(value) if value == "Backspace" => {
                    if self.text_editing.take_ime_preedit().is_some() {
                        true
                    } else {
                        self.delete_backward_in_focused_target()
                    }
                }
                KeyCode::Named(value) if value == "Delete" => {
                    if self.text_editing.take_ime_preedit().is_some() {
                        true
                    } else {
                        self.delete_forward_in_focused_target()
                    }
                }
                KeyCode::Named(value) if value == "ArrowLeft" => {
                    self.move_focused_caret_horizontal(true, modifiers.shift)
                }
                KeyCode::Named(value) if value == "ArrowRight" => {
                    self.move_focused_caret_horizontal(false, modifiers.shift)
                }
                KeyCode::Named(value) if value == "Home" => {
                    self.move_focused_caret_to_boundary(false, modifiers.shift)
                }
                KeyCode::Named(value) if value == "End" => {
                    self.move_focused_caret_to_boundary(true, modifiers.shift)
                }
                KeyCode::Named(value) if value == "Escape" => {
                    let changed = self.text_editing.active_text_context_menu.is_some()
                        || self.active_popup_menu_visible();
                    self.dismiss_active_text_context_menu();
                    self.dismiss_active_popup_menu();
                    changed
                }
                KeyCode::Named(value) if value == "Enter" => {
                    if self.text_editing.ime_preedit.is_some() {
                        false
                    } else if self.perform_editor_submit() {
                        true
                    } else {
                        // Enter inserts a newline like any other text. The
                        // model's line limit is what decides whether it
                        // survives: a single-line field strips it (and the
                        // edit reports no change, so the key bubbles), a
                        // capped field refuses the edit that would exceed
                        // the limit, and an unlimited field accepts it.
                        self.insert_text_into_focused_target("\n")
                    }
                }
                KeyCode::Character(text) => {
                    if self.text_editing.ime_preedit.is_some() || text.is_empty() {
                        false
                    } else {
                        self.insert_text_into_focused_target(text.as_str())
                    }
                }
                KeyCode::Named(_) | KeyCode::Unidentified => false,
            }
        };
        if changed {
            self.reset_text_caret_animation(self.frame_instant());
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            key = ?key,
            modifiers = ?modifiers,
            focused = ?self.text_editing.focused_key(),
            changed,
            "key handled"
        );
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_selection_menu() -> nami::Computed<Vec<ResolvedMenuItem>> {
        nami::Computed::new(Vec::new())
    }

    fn text_field_model(value: &str, line_limit: Option<usize>) -> TextInputModel {
        TextInputModel::TextField {
            value: Binding::container(StyledStr::plain(value.to_owned())),
            line_limit,
            on_submit: None,
            selection_menu: empty_selection_menu(),
        }
    }

    fn secure_field_model(value: &str) -> TextInputModel {
        let mut secure = FormSecure::default();
        secure.set(value.to_owned());
        TextInputModel::SecureField {
            value: Binding::container(secure),
        }
    }

    #[test]
    fn normalized_insert_text_strips_newlines_for_single_line_models() {
        assert_eq!(normalized_insert_text("a\r\nb\nc", Some(1)), "abc");
        assert_eq!(normalized_insert_text("a\r\nb\nc", Some(2)), "a\nb\nc");
    }

    #[test]
    fn replace_text_selection_rejects_line_limit_overflow() {
        let mut text = String::from("hello\nthere");
        let (mut anchor, mut focus) = (text.len(), text.len());
        assert!(!replace_text_selection(
            &mut text,
            &mut anchor,
            &mut focus,
            "\nworld",
            Some(2),
        ));
        assert_eq!(text, "hello\nthere");
        assert_eq!((anchor, focus), (11, 11));
    }

    #[test]
    fn replace_text_selection_updates_caret_at_char_boundary() {
        let mut text = String::from("a界c");
        let (mut anchor, mut focus) = (1, 4);
        assert!(replace_text_selection(
            &mut text,
            &mut anchor,
            &mut focus,
            "🙂",
            None,
        ));
        assert_eq!(text, "a🙂c");
        assert_eq!(anchor, 1 + "🙂".len());
        assert_eq!(focus, anchor);
    }

    #[test]
    fn delete_backward_in_selection_removes_selection_or_previous_grapheme_boundary() {
        let mut selected = String::from("abcdef");
        let (mut anchor, mut focus) = (2, 5);
        assert!(delete_backward_in_selection(
            &mut selected,
            &mut anchor,
            &mut focus,
        ));
        assert_eq!(selected, "abf");
        assert_eq!((anchor, focus), (2, 2));

        let mut collapsed = String::from("a界c");
        let (mut anchor, mut focus) = (4, 4);
        assert!(delete_backward_in_selection(
            &mut collapsed,
            &mut anchor,
            &mut focus,
        ));
        assert_eq!(collapsed, "ac");
        assert_eq!((anchor, focus), (1, 1));

        let mut joined = String::from("a👨‍👩‍👧‍👦e\u{301}");
        let (mut anchor, mut focus) = (joined.len(), joined.len());
        assert!(delete_backward_in_selection(
            &mut joined,
            &mut anchor,
            &mut focus,
        ));
        assert_eq!(joined, "a👨‍👩‍👧‍👦");
        assert!(delete_backward_in_selection(
            &mut joined,
            &mut anchor,
            &mut focus,
        ));
        assert_eq!(joined, "a");
        assert_eq!((anchor, focus), (1, 1));
    }

    #[test]
    fn delete_forward_in_selection_removes_one_extended_grapheme() {
        let mut text = String::from("e\u{301}👩🏽‍💻z");
        let (mut anchor, mut focus) = (0, 0);
        assert!(delete_forward_in_selection(
            &mut text,
            &mut anchor,
            &mut focus,
        ));
        assert_eq!(text, "👩🏽‍💻z");
        assert!(delete_forward_in_selection(
            &mut text,
            &mut anchor,
            &mut focus,
        ));
        assert_eq!(text, "z");
    }

    #[test]
    fn replace_model_selection_enforces_text_field_line_limit() {
        let model = text_field_model("hello\nthere", Some(2));
        let mut slot = TextSelectionSlot {
            anchor: 11,
            focus: 11,
            initialized: true,
        };

        assert!(!replace_model_selection(&model, &mut slot, "\nworld"));
        assert_eq!(model.plain_text(), "hello\nthere");
        assert_eq!((slot.anchor, slot.focus), (11, 11));
    }

    #[test]
    fn stale_transient_layout_does_not_rewind_the_model_selection() {
        let model = text_field_model("l", Some(1));
        let layout = parley::Layout::new();
        let slot = TextSelectionSlot {
            anchor: 1,
            focus: 1,
            initialized: true,
        };

        let display_selection = selection_for_target_layout(&model, &layout, &slot);

        assert_eq!(display_selection.focus().index(), 0);
        assert_eq!((slot.anchor, slot.focus), (1, 1));
    }

    #[test]
    fn secure_model_forces_single_line_and_masks_layout_indices_by_character() {
        let model = secure_field_model("a界c");
        let mut slot = TextSelectionSlot {
            anchor: 1,
            focus: 1,
            initialized: true,
        };

        assert_eq!(model.line_limit(), Some(1));
        assert!(model.is_secure());
        assert_eq!(model.layout_index_from_plain_index(1), 1);
        assert_eq!(model.layout_index_from_plain_index(4), 2);
        assert_eq!(model.plain_index_from_layout_index(2), 4);

        assert!(replace_model_selection(&model, &mut slot, "\n🙂"));
        assert_eq!(model.plain_text(), "a🙂界c");
        assert_eq!((slot.anchor, slot.focus), (1 + "🙂".len(), 1 + "🙂".len()));
    }
}
