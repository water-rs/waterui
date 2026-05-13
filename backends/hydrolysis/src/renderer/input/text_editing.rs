use super::*;

#[derive(Clone)]
pub(crate) enum TextInputModel {
    TextField {
        value: nami::Binding<StyledStr>,
        line_limit: Option<usize>,
        selection_menu: nami::Computed<Vec<ResolvedMenuItem>>,
    },
    SecureField {
        value: nami::Binding<FormSecure>,
    },
}

#[derive(Default)]
pub(crate) struct TextEditingState {
    pub(crate) text_input_targets: Vec<TextInputTarget>,
    pub(crate) text_selection_slots: Vec<Rc<RefCell<TextSelectionSlot>>>,
    pub(crate) text_selection_cursor: usize,
    pub(crate) active_text_selection_drag: Option<usize>,
    pub(crate) last_text_selection_click: Option<TextSelectionClickState>,
    pub(crate) active_text_context_menu: Option<ActiveTextContextMenu>,
    pub(crate) focused_text_input: Cell<Option<usize>>,
    pub(crate) ime_preedit: Option<Str>,
    pub(crate) text_caret_fade_started_at: Option<Instant>,
    pub(crate) text_caret_next_frame_at: Option<Instant>,
    pub(crate) text_caret_motion: Option<TextCaretMotion>,
}

#[derive(Debug, Default)]
pub(crate) struct TextSelectionSlot {
    pub(crate) anchor: usize,
    pub(crate) focus: usize,
    pub(crate) initialized: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TextSelectionClickState {
    pub(crate) target_index: usize,
    pub(crate) point: vello::kurbo::Point,
    pub(crate) at: Instant,
    pub(crate) count: u8,
}

#[derive(Clone)]
pub(crate) struct TextInputTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) cursor_area: vello::kurbo::Rect,
    pub(crate) text_bounds: vello::kurbo::Rect,
    pub(crate) layout: parley::Layout<[u8; 4]>,
    pub(crate) purpose: TextInputPurpose,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) model: TextInputModel,
    pub(crate) selection: Rc<RefCell<TextSelectionSlot>>,
    pub(crate) focus_binding: Option<Binding<bool>>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_node_id: Option<AccessibilityNodeId>,
}

pub(crate) struct TextInputTargetRegistration {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) cursor_area: vello::kurbo::Rect,
    pub(crate) text_bounds: vello::kurbo::Rect,
    pub(crate) layout: parley::Layout<[u8; 4]>,
    pub(crate) purpose: TextInputPurpose,
    pub(crate) model: TextInputModel,
    pub(crate) selection: Rc<RefCell<TextSelectionSlot>>,
}

pub(crate) struct TextInputTargetData {
    pub(crate) target: TextInputTargetRegistration,
    pub(crate) depth: usize,
    pub(crate) focus_binding: Option<Binding<bool>>,
    #[cfg(feature = "accessibility")]
    pub(crate) accessibility_node_id: Option<AccessibilityNodeId>,
}

#[derive(Clone)]
pub(crate) enum TextContextMenuAction {
    Copy,
    Cut,
    Paste,
    SelectAll,
    Custom(ResolvedCommand),
}

#[derive(Clone)]
pub(crate) enum TextContextMenuEntry {
    Command {
        label: String,
        action: Box<TextContextMenuAction>,
    },
    Divider,
}

#[derive(Clone)]
pub(crate) struct TextContextMenuOverlayRow {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) entry: TextContextMenuEntry,
}

#[derive(Clone)]
pub(crate) struct TextContextMenuOverlay {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) rows: Vec<TextContextMenuOverlayRow>,
    pub(crate) model: TextInputModel,
    pub(crate) selection: Rc<RefCell<TextSelectionSlot>>,
    pub(crate) env: Environment,
}

#[derive(Clone)]
pub(crate) enum ActiveTextContextMenu {
    Overlay {
        index: usize,
        overlay: TextContextMenuOverlay,
    },
    NativeWindow {
        index: usize,
        state: nami::Binding<WindowState>,
    },
}

impl TextInputModel {
    pub(crate) fn plain_text(&self) -> String {
        match self {
            Self::TextField { value, .. } => value.get().to_plain().to_string(),
            Self::SecureField { value } => value.get().expose().to_owned(),
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

    pub(crate) fn line_limit(&self) -> Option<usize> {
        match self {
            Self::TextField { line_limit, .. } => *line_limit,
            Self::SecureField { .. } => Some(1),
        }
    }

    pub(crate) fn is_secure(&self) -> bool {
        matches!(self, Self::SecureField { .. })
    }

    pub(crate) fn custom_selection_menu_items(&self) -> Vec<ResolvedMenuItem> {
        match self {
            Self::TextField { selection_menu, .. } => selection_menu.get(),
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
}

pub(crate) fn normalized_insert_text(inserted: &str, max_lines: Option<usize>) -> String {
    if max_lines == Some(1) {
        inserted
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r')
            .collect()
    } else {
        inserted.chars().filter(|ch| *ch != '\r').collect()
    }
}

pub(crate) fn line_count(value: &str) -> usize {
    value.chars().filter(|ch| *ch == '\n').count() + 1
}

pub(crate) fn exceeds_line_limit(value: &str, max_lines: Option<usize>) -> bool {
    max_lines.is_some_and(|max| line_count(value) > max)
}

pub(crate) fn apply_text_insert(
    buffer: &mut String,
    inserted: &str,
    max_lines: Option<usize>,
) -> bool {
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

pub(crate) fn clamp_to_char_boundary(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        index = text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub(crate) fn previous_char_boundary(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    if clamped == 0 {
        return 0;
    }
    text[..clamped]
        .char_indices()
        .next_back()
        .map_or(0, |(value, _)| value)
}

pub(crate) fn next_char_boundary(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    if clamped >= text.len() {
        return text.len();
    }
    let mut chars = text[clamped..].chars();
    let Some(ch) = chars.next() else {
        return text.len();
    };
    clamped + ch.len_utf8()
}

pub(crate) fn byte_index_to_char_offset(text: &str, index: usize) -> usize {
    let clamped = clamp_to_char_boundary(text, index);
    text[..clamped].chars().count()
}

pub(crate) fn char_offset_to_byte_index(text: &str, char_offset: usize) -> usize {
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

pub(crate) fn normalized_selection_range(anchor: usize, focus: usize) -> std::ops::Range<usize> {
    anchor.min(focus)..anchor.max(focus)
}

pub(crate) fn replace_text_selection(
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

pub(crate) fn delete_backward_in_selection(
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
    let previous = text[..start]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0);
    text.replace_range(previous..start, "");
    *anchor = previous;
    *focus = previous;
    true
}

pub(crate) fn delete_forward_in_selection(
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
    let mut iter = text[start..].char_indices();
    let Some((_, ch)) = iter.next() else {
        return false;
    };
    let next = start + ch.len_utf8();
    text.replace_range(start..next, "");
    *anchor = start;
    *focus = start;
    true
}

pub(crate) fn selection_slot_range_for_text(
    slot: &TextSelectionSlot,
    text: &str,
) -> std::ops::Range<usize> {
    let anchor = clamp_to_char_boundary(text, slot.anchor);
    let focus = clamp_to_char_boundary(text, slot.focus);
    normalized_selection_range(anchor, focus)
}

pub(crate) fn selected_text_for_model(
    model: &TextInputModel,
    slot: &TextSelectionSlot,
) -> Option<String> {
    let text = model.plain_text();
    let range = selection_slot_range_for_text(slot, text.as_str());
    if range.is_empty() {
        return None;
    }
    text.as_str().get(range).map(str::to_owned)
}

pub(crate) fn replace_model_selection(
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
    model.set_plain_text(text);
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    true
}

pub(crate) fn delete_model_selection(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    let range = normalized_selection_range(anchor, focus);
    if range.is_empty() {
        return false;
    }
    text.replace_range(range.clone(), "");
    model.set_plain_text(text);
    slot.anchor = range.start;
    slot.focus = range.start;
    slot.initialized = true;
    true
}

pub(crate) fn delete_model_backward(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !delete_backward_in_selection(&mut text, &mut anchor, &mut focus) {
        return false;
    }
    model.set_plain_text(text);
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    true
}

pub(crate) fn delete_model_forward(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
    let mut text = model.plain_text();
    let mut anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let mut focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    if !delete_forward_in_selection(&mut text, &mut anchor, &mut focus) {
        return false;
    }
    model.set_plain_text(text);
    slot.anchor = anchor;
    slot.focus = focus;
    slot.initialized = true;
    true
}

pub(crate) fn set_model_caret_position(
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

pub(crate) fn move_model_caret_horizontal(
    model: &TextInputModel,
    slot: &mut TextSelectionSlot,
    backward: bool,
    extend: bool,
) -> bool {
    let text = model.plain_text();
    let anchor = clamp_to_char_boundary(text.as_str(), slot.anchor);
    let focus = clamp_to_char_boundary(text.as_str(), slot.focus);
    let base = if !extend && anchor != focus {
        if backward {
            anchor.min(focus)
        } else {
            anchor.max(focus)
        }
    } else {
        focus
    };
    let next = if backward {
        previous_char_boundary(text.as_str(), base)
    } else {
        next_char_boundary(text.as_str(), base)
    };
    if extend {
        let changed = slot.focus != next || !slot.initialized;
        slot.anchor = anchor;
        slot.focus = next;
        slot.initialized = true;
        return changed;
    }
    set_model_caret_position(model, slot, next)
}

pub(crate) async fn read_clipboard_text_async() -> Option<String> {
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

pub(crate) fn spawn_clipboard_paste_task(
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

pub(crate) fn select_all_model_text(model: &TextInputModel, slot: &mut TextSelectionSlot) -> bool {
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

pub(crate) fn write_clipboard_text(text: &str) -> bool {
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

pub(crate) fn selection_range_contains_index(
    model: &TextInputModel,
    slot: &TextSelectionSlot,
    index: usize,
) -> bool {
    let text = model.plain_text();
    let range = selection_slot_range_for_text(slot, text.as_str());
    !range.is_empty() && range.contains(&index)
}

pub(crate) fn text_context_menu_size(
    entries: &[TextContextMenuEntry],
    metrics: TextContextMenuMetrics,
) -> (f64, f64) {
    let max_label_chars = entries
        .iter()
        .filter_map(|entry| match entry {
            TextContextMenuEntry::Command { label, .. } => Some(label.chars().count()),
            TextContextMenuEntry::Divider => None,
        })
        .max()
        .unwrap_or(0) as f64;
    let width = (metrics.horizontal_padding * 2.0 + max_label_chars * metrics.width_per_char)
        .clamp(metrics.min_width, metrics.max_width);
    let height = (entries.len() as f64 * metrics.row_height).max(metrics.row_height);
    (width, height)
}

pub(crate) fn text_context_menu_overlay_bounds(
    anchor: vello::kurbo::Point,
    entries: &[TextContextMenuEntry],
    window_bounds: vello::kurbo::Rect,
    metrics: TextContextMenuMetrics,
) -> vello::kurbo::Rect {
    let (width, height) = text_context_menu_size(entries, metrics);
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
    vello::kurbo::Rect::new(x0, y0, x0 + width, y0 + height)
}

pub(crate) fn execute_text_context_menu_action(
    action: &TextContextMenuAction,
    model: &TextInputModel,
    selection: &Rc<RefCell<TextSelectionSlot>>,
    env: &Environment,
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
        TextContextMenuAction::Custom(command) => {
            call_action_discarding_result(&command.action, env);
            true
        }
    }
}

impl HydrolysisRenderer {
    pub(crate) fn set_text_caret_motion(&mut self, motion: TextCaretMotion) {
        self.text_editing.text_caret_motion = Some(motion);
    }

    fn text_caret_motion(&self) -> TextCaretMotion {
        self.text_editing
            .text_caret_motion
            .expect("hydrolysis text input render must install text caret motion before focus")
    }

    pub(crate) fn reset_text_caret_animation(&mut self, now: Instant) {
        let motion = self.text_caret_motion();
        self.text_editing.text_caret_fade_started_at = Some(now);
        self.text_editing.text_caret_next_frame_at = Some(
            now.checked_add(motion.frame_interval)
                .expect("hydrolysis text caret frame timestamp overflow"),
        );
    }

    pub(crate) fn clear_text_caret_animation(&mut self) {
        self.text_editing.text_caret_fade_started_at = None;
        self.text_editing.text_caret_next_frame_at = None;
    }

    pub(crate) fn advance_text_caret_animation(&mut self, now: Instant) -> bool {
        if self.text_editing.focused_text_input.get().is_none() {
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
        if self.text_editing.focused_text_input.get().is_none() {
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
        let wave = ((core::f32::consts::TAU * phase).cos() + 1.0) * 0.5;
        motion.min_opacity + (1.0 - motion.min_opacity) * wave
    }

    pub(crate) fn set_focused_text_input(&mut self, focused: Option<usize>) -> bool {
        let previous = self.text_editing.focused_text_input.get();
        if previous == focused {
            return false;
        }
        let previous_binding = previous
            .and_then(|index| self.text_editing.text_input_targets.as_slice().get(index))
            .and_then(|target| target.focus_binding.clone());
        let next_binding = focused
            .and_then(|index| self.text_editing.text_input_targets.as_slice().get(index))
            .and_then(|target| target.focus_binding.clone());
        if let Some(binding) = previous_binding {
            binding.set(false);
        }
        self.text_editing.focused_text_input.set(focused);
        if let Some(binding) = next_binding {
            binding.set(true);
        }
        self.text_editing.active_text_selection_drag = None;
        self.text_editing.ime_preedit = None;
        if focused.is_some() {
            self.reset_text_caret_animation(self.frame_instant());
        } else {
            self.clear_text_caret_animation();
            self.dismiss_active_text_context_menu();
            self.dismiss_active_popup_menu();
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            previous_focus = ?previous,
            next_focus = ?focused,
            "text input focus changed"
        );
        true
    }

    pub(crate) fn dismiss_active_text_context_menu(&mut self) {
        if let Some(menu) = self.text_editing.active_text_context_menu.take()
            && let ActiveTextContextMenu::NativeWindow { state, .. } = menu
        {
            state.set(WindowState::Closed);
        }
    }

    pub(crate) fn active_text_context_menu_target(&self) -> Option<usize> {
        self.text_editing
            .active_text_context_menu
            .as_ref()
            .map(|menu| match menu {
                ActiveTextContextMenu::Overlay { index, .. }
                | ActiveTextContextMenu::NativeWindow { index, .. } => *index,
            })
    }

    pub(crate) fn render_active_text_context_menu_overlay(
        &mut self,
        env: &Environment,
        transform: vello::kurbo::Affine,
    ) {
        let Some(ActiveTextContextMenu::Overlay { overlay, .. }) =
            self.text_editing.active_text_context_menu.clone()
        else {
            return;
        };

        let theme = widget_theme(env);
        let metrics = theme.text_context_menu_metrics();
        {
            let mut draw = VelloDrawContext::with_root_transform(&mut self.scene, transform);
            theme.draw_text_context_menu_panel(&mut draw, overlay.bounds);
        }
        for (index, row) in overlay.rows.iter().enumerate() {
            let next_is_divider = overlay
                .rows
                .as_slice()
                .get(index + 1)
                .is_some_and(|next| matches!(next.entry, TextContextMenuEntry::Divider));
            if index + 1 < overlay.rows.len()
                && !matches!(row.entry, TextContextMenuEntry::Divider)
                && !next_is_divider
            {
                let separator = vello::kurbo::Rect::new(
                    row.bounds.x0 + metrics.separator_horizontal_inset,
                    row.bounds.y1 - metrics.separator_thickness,
                    row.bounds.x1 - metrics.separator_horizontal_inset,
                    row.bounds.y1,
                );
                let mut draw = VelloDrawContext::with_root_transform(&mut self.scene, transform);
                theme.draw_text_context_menu_separator(&mut draw, separator);
            }

            match &row.entry {
                TextContextMenuEntry::Command { label, .. } => {
                    let text_rect = inset_rect(
                        row.bounds,
                        metrics.horizontal_padding,
                        metrics.vertical_padding,
                    );
                    let ctx = RenderContext {
                        transform,
                        hit_transform: vello::kurbo::Affine::IDENTITY,
                        bounds: overlay.bounds,
                    }
                    .child(
                        vello::kurbo::Affine::translate((text_rect.x0, text_rect.y0)),
                        vello::kurbo::Rect::new(0.0, 0.0, text_rect.width(), text_rect.height()),
                    );
                    let (state, scene) = self.state_and_scene_mut();
                    Self::render_styled_text(
                        state,
                        scene,
                        ctx,
                        StyledStr::plain(label.clone()),
                        HorizontalAlignment::Leading,
                        env,
                    );
                }
                TextContextMenuEntry::Divider => {
                    let separator = vello::kurbo::Rect::new(
                        row.bounds.x0 + metrics.separator_horizontal_inset,
                        row.bounds.y0 + row.bounds.height() * 0.5
                            - metrics.separator_thickness * 0.5,
                        row.bounds.x1 - metrics.separator_horizontal_inset,
                        row.bounds.y0
                            + row.bounds.height() * 0.5
                            + metrics.separator_thickness * 0.5,
                    );
                    let mut draw =
                        VelloDrawContext::with_root_transform(&mut self.scene, transform);
                    theme.draw_text_context_menu_separator(&mut draw, separator);
                }
            }
        }
    }

    pub(crate) fn handle_text_context_menu_overlay_pointer_down(
        &mut self,
        point: vello::kurbo::Point,
    ) -> bool {
        let Some(ActiveTextContextMenu::Overlay { overlay, .. }) =
            self.text_editing.active_text_context_menu.clone()
        else {
            return false;
        };
        if !overlay.bounds.contains(point) {
            self.dismiss_active_text_context_menu();
            return false;
        }
        for row in &overlay.rows {
            if !row.bounds.contains(point) {
                continue;
            }
            match &row.entry {
                TextContextMenuEntry::Command { action, .. } => {
                    let changed = execute_text_context_menu_action(
                        action,
                        &overlay.model,
                        &overlay.selection,
                        &overlay.env,
                    );
                    self.dismiss_active_text_context_menu();
                    return changed;
                }
                TextContextMenuEntry::Divider => return false,
            }
        }
        true
    }

    pub(crate) fn focused_text_target_data(
        &mut self,
    ) -> Option<(usize, TextInputModel, Rc<RefCell<TextSelectionSlot>>)> {
        let index = self.text_editing.focused_text_input.get()?;
        let Some(target) = self.text_editing.text_input_targets.as_slice().get(index) else {
            self.set_focused_text_input(None);
            return None;
        };
        Some((index, target.model.clone(), Rc::clone(&target.selection)))
    }

    pub(crate) fn text_selection_index_from_point(
        target: &TextInputTarget,
        point: vello::kurbo::Point,
    ) -> usize {
        let local_x = (point.x - target.text_bounds.x0) as f32;
        let local_y = (point.y - target.text_bounds.y0) as f32;
        let selection =
            parley::Selection::from_point(&target.layout, local_x, local_y).refresh(&target.layout);
        target
            .model
            .plain_index_from_layout_index(selection.focus().index())
    }

    pub(crate) fn text_selection_range_from_point_with_click_count(
        target: &TextInputTarget,
        point: vello::kurbo::Point,
        click_count: u8,
    ) -> (usize, usize) {
        let local_x = (point.x - target.text_bounds.x0) as f32;
        let local_y = (point.y - target.text_bounds.y0) as f32;
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

    pub(crate) fn next_text_selection_click_count(
        &mut self,
        target_index: usize,
        point: vello::kurbo::Point,
        at: Instant,
    ) -> u8 {
        let count = if let Some(previous) = self.text_editing.last_text_selection_click {
            if previous.target_index == target_index
                && at.saturating_duration_since(previous.at) <= TEXT_SELECTION_MULTI_CLICK_INTERVAL
                && previous.point.distance(point) <= TEXT_SELECTION_MULTI_CLICK_DISTANCE
            {
                previous.count.saturating_add(1).min(3)
            } else {
                1
            }
        } else {
            1
        };
        self.text_editing.last_text_selection_click = Some(TextSelectionClickState {
            target_index,
            point,
            at,
            count,
        });
        count
    }

    pub(crate) fn apply_text_selection_click_gesture(
        &mut self,
        index: usize,
        point: vello::kurbo::Point,
        click_count: u8,
    ) -> bool {
        let Some(target) = self.text_editing.text_input_targets.as_slice().get(index) else {
            self.text_editing.active_text_selection_drag = None;
            return false;
        };
        let (anchor, focus) =
            Self::text_selection_range_from_point_with_click_count(target, point, click_count);
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
        point: vello::kurbo::Point,
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
        let Some((_index, model, selection)) = self.focused_text_target_data() else {
            return false;
        };
        let mut slot = selection.borrow_mut();
        move_model_caret_horizontal(&model, &mut slot, backward, extend)
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

    pub(crate) fn build_text_context_menu_entries(
        target: &TextInputTarget,
    ) -> Vec<TextContextMenuEntry> {
        let has_selection = {
            let slot = target.selection.borrow();
            selected_text_for_model(&target.model, &slot).is_some()
        };
        let has_text = !target.model.plain_text().is_empty();
        let mut entries = Vec::new();
        if has_selection && !target.model.is_secure() {
            entries.push(TextContextMenuEntry::Command {
                label: "Copy".to_owned(),
                action: Box::new(TextContextMenuAction::Copy),
            });
            entries.push(TextContextMenuEntry::Command {
                label: "Cut".to_owned(),
                action: Box::new(TextContextMenuAction::Cut),
            });
        }
        entries.push(TextContextMenuEntry::Command {
            label: "Paste".to_owned(),
            action: Box::new(TextContextMenuAction::Paste),
        });
        if has_text {
            entries.push(TextContextMenuEntry::Command {
                label: "Select All".to_owned(),
                action: Box::new(TextContextMenuAction::SelectAll),
            });
        }
        if has_selection {
            for item in target.model.custom_selection_menu_items() {
                match item {
                    ResolvedMenuItem::Command(command) => {
                        entries.push(TextContextMenuEntry::Command {
                            label: command.label.content.get().to_plain().to_string(),
                            action: Box::new(TextContextMenuAction::Custom(command)),
                        });
                    }
                    ResolvedMenuItem::Divider => entries.push(TextContextMenuEntry::Divider),
                    ResolvedMenuItem::Menu(_) => {
                        panic!("hydrolysis text selection menus do not support nested menus yet")
                    }
                }
            }
        }
        entries
    }

    pub(crate) fn show_text_context_menu(
        &mut self,
        index: usize,
        point: vello::kurbo::Point,
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
        let entries = Self::build_text_context_menu_entries(&target);
        if entries.is_empty() {
            self.dismiss_active_text_context_menu();
            return false;
        }

        self.dismiss_active_text_context_menu();
        let mode = env
            .get::<HydrolysisTextContextMenuMode>()
            .copied()
            .unwrap_or(HydrolysisTextContextMenuMode::NativeWindow);

        if mode == HydrolysisTextContextMenuMode::Overlay {
            let metrics = widget_theme(env).text_context_menu_metrics();
            let bounds =
                text_context_menu_overlay_bounds(point, &entries, self.window_bounds, metrics);
            let mut rows = Vec::with_capacity(entries.len());
            for (index, entry) in entries.into_iter().enumerate() {
                let y0 = bounds.y0 + metrics.row_height * index as f64;
                let row_bounds =
                    vello::kurbo::Rect::new(bounds.x0, y0, bounds.x1, y0 + metrics.row_height);
                rows.push(TextContextMenuOverlayRow {
                    bounds: row_bounds,
                    entry,
                });
            }
            self.text_editing.active_text_context_menu = Some(ActiveTextContextMenu::Overlay {
                index,
                overlay: TextContextMenuOverlay {
                    bounds,
                    rows,
                    model: target.model,
                    selection: target.selection,
                    env: env.clone(),
                },
            });
            return true;
        }

        let menu_state = nami::Binding::container(WindowState::Normal);
        let metrics = widget_theme(env).text_context_menu_metrics();
        let (width, height) = text_context_menu_size(&entries, metrics);
        let origin = env
            .get::<HydrolysisWindowOrigin>()
            .copied()
            .expect("hydrolysis text context menu requires HydrolysisWindowOrigin in environment");

        let entries_for_popup = entries.clone();
        let model = target.model.clone();
        let selection = Rc::clone(&target.selection);
        let action_env = env.clone();
        let menu_state_for_content = menu_state.clone();
        let popup_content = move || {
            let mut rows = Vec::with_capacity(entries_for_popup.len());
            for entry in entries_for_popup.clone() {
                let state_binding = menu_state_for_content.clone();
                let model = model.clone();
                let selection = Rc::clone(&selection);
                let action_env = action_env.clone();
                match entry {
                    TextContextMenuEntry::Command { label, action } => {
                        let button =
                            Button::new(label)
                                .style(ButtonStyle::Borderless)
                                .action(move || {
                                    state_binding.set(WindowState::Closed);
                                    let _ = execute_text_context_menu_action(
                                        &action,
                                        &model,
                                        &selection,
                                        &action_env,
                                    );
                                });
                        rows.push(AnyView::new(button));
                    }
                    TextContextMenuEntry::Divider => rows.push(AnyView::new(Divider)),
                }
            }
            let menu_content: waterui_layout::stack::VStack<(Vec<AnyView>,)> =
                rows.into_iter().collect();
            AnyView::new(
                menu_content
                    .alignment(HorizontalAlignment::Leading)
                    .spacing(0.0),
            )
        };
        let mut popup = Window::new(
            TEXT_CONTEXT_MENU_WINDOW_TITLE,
            menu_state.clone(),
            popup_content,
        )
        .style(WindowStyle::Borderless)
        .resizable(false);
        popup.closable = false;
        popup.frame.set(LayoutRect::new(
            LayoutPoint::new(origin.x + point.x as f32, origin.y + point.y as f32),
            LayoutSize::new(width as f32, height as f32),
        ));
        popup.show(env);
        self.text_editing.active_text_context_menu = Some(ActiveTextContextMenu::NativeWindow {
            index,
            state: menu_state,
        });
        true
    }

    pub fn handle_text_input(&mut self, text: &str) -> bool {
        let preedit_cleared = self.text_editing.ime_preedit.take().is_some();
        if text.is_empty() {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                focused = ?self.text_editing.focused_text_input.get(),
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
            focused = ?self.text_editing.focused_text_input.get(),
            text = text,
            changed,
            "text input handled"
        );
        changed
    }

    pub fn handle_ime_preedit(&mut self, text: &str) -> bool {
        if self.text_editing.focused_text_input.get().is_none() {
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                text = text,
                "ime preedit dropped without focused text input"
            );
            return false;
        }
        let next = if text.is_empty() {
            None
        } else {
            Some(Str::from(text.to_owned()))
        };
        if self.text_editing.ime_preedit == next {
            return false;
        }
        self.text_editing.ime_preedit = next;
        self.reset_text_caret_animation(self.frame_instant());
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            focused = ?self.text_editing.focused_text_input.get(),
            preedit = ?self.text_editing.ime_preedit,
            "ime preedit updated"
        );
        true
    }

    pub fn handle_ime_commit(&mut self, text: &str) -> bool {
        self.handle_text_input(text)
    }

    pub fn handle_ime_disabled(&mut self) -> bool {
        let changed = self.text_editing.ime_preedit.take().is_some();
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            changed,
            "ime disabled handled"
        );
        changed
    }

    pub fn handle_key(&mut self, key: &KeyCode, modifiers: Modifiers) -> bool {
        if self.text_editing.focused_text_input.get().is_none() {
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
                        || self.popup_menu.active_popup_menu_group.is_some();
                    self.dismiss_active_text_context_menu();
                    self.dismiss_active_popup_menu();
                    changed
                }
                _ => false,
            }
        } else {
            match key {
                KeyCode::Named(value) if value == "Backspace" => {
                    if self.text_editing.ime_preedit.take().is_some() {
                        true
                    } else {
                        self.delete_backward_in_focused_target()
                    }
                }
                KeyCode::Named(value) if value == "Delete" => {
                    if self.text_editing.ime_preedit.take().is_some() {
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
                        || self.popup_menu.active_popup_menu_group.is_some();
                    self.dismiss_active_text_context_menu();
                    self.dismiss_active_popup_menu();
                    changed
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
            focused = ?self.text_editing.focused_text_input.get(),
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
    }

    #[test]
    fn move_model_caret_horizontal_collapses_selection_before_moving() {
        let model = text_field_model("hello", None);
        let mut slot = TextSelectionSlot {
            anchor: 1,
            focus: 4,
            initialized: true,
        };

        assert!(move_model_caret_horizontal(&model, &mut slot, true, false));
        assert_eq!((slot.anchor, slot.focus), (0, 0));

        assert!(move_model_caret_horizontal(&model, &mut slot, false, true));
        assert_eq!((slot.anchor, slot.focus), (0, 1));
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
