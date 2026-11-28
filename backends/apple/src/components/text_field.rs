//! The `text_field` leaf: `Native<TextFieldConfig>` rendered
//! through the kit's editable text field inside a container that also
//! holds the label child.
//!
//! Mirrors `WuiTextField`: the `value` `Binding<StyledStr>` is two-way —
//! watchers push resolved attributed text onto the field and user edits
//! write back as plain text — the `prompt` `Text` renders into the
//! placeholder under the placeholder color, `Disabled` pushes
//! `set_enabled`, the label mounts above the field and names it to
//! assistive technology, and `on_submit` fires on Return. The measure is
//! `WuiTextField.sizeThatFits`: a 100pt-width floor, the label stacked
//! over the field with 4pt of spacing, always the intrinsic height.
//!
//! The field is also this subtree's focus anchor — the kit marks it so
//! `Metadata<Focused>` resolves it the way `installWuiFocusTarget` marked
//! it in Swift. `WuiSecureField` and the selection menu stay in the
//! Swift fallback until their own ports land.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
#[cfg(target_os = "macos")]
use core::num::NonZeroUsize;
use core::ops::Range;

use cocoa_ui::{PlatformView, Rect, Retained, focus, view};
use nami::Computed;
use nami::{Binding, Signal};
use objc2::rc::Weak;
use objc2_foundation::{NSRange, NSString};
use waterui_backend_core::Environment;
use waterui_controls::ControlSize;
#[cfg(target_os = "ios")]
use waterui_controls::input::{Capitalization, KeyboardType};
use waterui_controls::menu::ResolvedMenuItem;
use waterui_controls::text_field::TextFieldConfig;
use waterui_controls::text_selection::{CustomTextSelection, TextSelection};
use waterui_controls::theme::color::Foreground;
use waterui_core::State;
use waterui_core::Str;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect as LayoutRect, ResolvedExtent, Size, StretchAxis, SubView,
    ViewDimensions,
};
use waterui_core::resolve::Resolvable;
use waterui_interaction::interaction::Disabled;
use waterui_text::StyledStr;
#[cfg(target_os = "macos")]
use waterui_text::font::Body;

use super::field_input::{
    self, StyledPush, apply_styled, platform_color, push, render_attributed, resolve_alignment,
    styled_push,
};
#[cfg(target_os = "macos")]
use super::font::platform_font;
#[cfg(target_os = "macos")]
use crate::components::control_size::control_font;
use crate::components::control_size::platform_control_size;
use crate::contract::{Held, Mounted, NativeLeaf};
use crate::proposal;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HostView, TextField};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::text_field::Keyboard;
    pub(super) use cocoa_ui::uikit::text_input::{Autocapitalization, Autocorrection};
    pub(super) use cocoa_ui::uikit::{HostView, TextField};
}

use crate::contract::RenderContext;
use platform::{HostView, TextField};

/// The gap between the label and the field — `WuiTextField.verticalSpacing`.
const VERTICAL_SPACING: f64 = 4.0;
/// The narrowest width the leaf reports — `WuiTextField`'s 100pt floor.
const MIN_WIDTH: f64 = 100.0;

/// The `Keyboard` a `KeyboardType` asks for — `WuiKeyboardType.uiKeyboardType`'s
/// table. `AppKit` has no on-screen keyboard; there the hint is unused.
#[cfg(target_os = "ios")]
/// A kind the backend does not know is a contract violation of the
/// framework above; the plain text keyboard is used.
fn keyboard(kind: KeyboardType) -> platform::Keyboard {
    match kind {
        KeyboardType::Text => platform::Keyboard::Text,
        KeyboardType::Email => platform::Keyboard::Email,
        KeyboardType::URL => platform::Keyboard::Url,
        KeyboardType::Number => platform::Keyboard::Number,
        KeyboardType::Decimal => platform::Keyboard::Decimal,
        KeyboardType::PhoneNumber => platform::Keyboard::PhoneNumber,
        _ => {
            waterui_core::contract_violation!(
                "unsupported WaterUI keyboard type; the text keyboard is used"
            );
            platform::Keyboard::Text
        }
    }
}

/// The `UIKit` autocapitalization mode a `Capitalization` asks for.
#[cfg(target_os = "ios")]
fn autocapitalization(kind: Capitalization) -> platform::Autocapitalization {
    match kind {
        Capitalization::None => platform::Autocapitalization::None,
        Capitalization::Words => platform::Autocapitalization::Words,
        Capitalization::Sentences => platform::Autocapitalization::Sentences,
        Capitalization::Characters => platform::Autocapitalization::AllCharacters,
        _ => {
            waterui_core::contract_violation!(
                "unsupported WaterUI capitalization mode; sentences is used"
            );
            platform::Autocapitalization::Sentences
        }
    }
}

/// The `UIKit` autocorrection setting for a resolved `autocorrect` boolean.
#[cfg(target_os = "ios")]
const fn autocorrection(enabled: bool) -> platform::Autocorrection {
    if enabled {
        platform::Autocorrection::Enabled
    } else {
        platform::Autocorrection::Disabled
    }
}

/// The field's share of a measure — the text height `WuiTextField` reports.
/// `AppKit` reads `intrinsicContentSize`; `UIKit` floors `sizeThatFits` at
/// intrinsic so the field never reports shorter than its font needs.
#[cfg(target_os = "macos")]
fn field_height(field: &TextField, _width: f64) -> f64 {
    field.measured_height()
}

/// The field's share of a measure — see the `AppKit` variant.
#[cfg(target_os = "ios")]
fn field_height(field: &TextField, width: f64) -> f64 {
    field.measured_height(width)
}

/// The value side of the leaf: binding watches pushing the resolved text
/// to the field, the theme-`Foreground` watch behind unstyled chunks, and
/// the field's edits writing plain text back into the binding.
fn wire_value(
    leaf: &mut NativeLeaf,
    value: &Binding<StyledStr>,
    ctx: &crate::contract::RenderContext<'_>,
    field: &TextField,
    value_state: &Rc<RefCell<StyledPush>>,
) {
    // An edit's `binding.set` re-runs the `value` watchers, which would push
    // the rebuilt attributed string right back onto the control — resetting
    // the live edit's caret and undo. `write_back` holds the exact value that
    // set propagates so the watcher can skip only that echo's push.
    let write_back = Rc::new(RefCell::new(Option::<Str>::None));
    leaf.watch(value, {
        let value_state = Rc::clone(value_state);
        let write_back = Rc::clone(&write_back);
        move |ctx| {
            apply_styled(&value_state, ctx.value());
            // Only the plain-text value the field's own edit just wrote is
            // the echo — a `set` propagating a different text or a styled
            // value inside the same window still pushes.
            let is_echo = ctx.value().is_plain()
                && write_back.borrow().as_ref() == Some(&ctx.value().to_plain());
            if !is_echo {
                push(&value_state.borrow());
            }
        }
    });
    // The theme default re-renders chunks without their own foreground.
    let default_foreground = Foreground.resolve(ctx.env());
    leaf.watch(&default_foreground, {
        let value_state = Rc::clone(value_state);
        move |ctx| {
            let mut state = value_state.borrow_mut();
            state.default_foreground = platform_color(ctx.value());
            push(&state);
        }
    });

    // Field → binding: user edits write back as plain text.
    #[cfg(target_os = "ios")]
    leaf.keep(field.install_change_handler({
        let binding = value.clone();
        move |field| {
            let echo = Str::from(field.string());
            *write_back.borrow_mut() = Some(echo.clone());
            binding.set(StyledStr::plain(echo));
            write_back.borrow_mut().take();
        }
    }));
    #[cfg(target_os = "macos")]
    field.on_change({
        let binding = value.clone();
        move |field| {
            let echo = Str::from(field.string());
            *write_back.borrow_mut() = Some(echo.clone());
            binding.set(StyledStr::plain(echo));
            write_back.borrow_mut().take();
        }
    });
}

/// The prompt's share of the leaf: the styled placeholder and its
/// alignment binding.
fn wire_prompt(
    mtm: cocoa_ui::MainThreadMarker,
    leaf: &mut NativeLeaf,
    ctx: &crate::contract::RenderContext<'_>,
    field: &Retained<TextField>,
    prompt: &waterui_text::TextConfig,
) {
    let placeholder_field = field.clone();
    let alignment_field = field.clone();
    field_input::wire_prompt(
        mtm,
        leaf,
        ctx,
        prompt,
        move |attributed| placeholder_field.set_placeholder(attributed),
        move |alignment| {
            let view: &PlatformView = &alignment_field;
            alignment_field.set_text_alignment(resolve_alignment(view, alignment));
        },
    );
}

/// The leaf's live state: the platform field and the mounted label child.
struct FieldState {
    field: Retained<TextField>,
    label: Mounted,
}

/// Lays out the children inside the `response` rectangle: the label at
/// top-leading, then the field spanning the full width below it —
/// `WuiTextField`'s constraints as manual frames. `UIKit` centers the input
/// inside the region between the label and the container's bottom, the way
/// the field's `inputRegion` layout guide does.
fn layout_children(view: &PlatformView, response: Rect, state: &FieldState) {
    let bounds = response;
    let label = state.label.layout().measure(ProposalSize::UNSPECIFIED).size;
    let has_label = label.height > 0.0;
    view::set_hidden(state.label.view(), !has_label);

    // The label pins top+leading and keeps its intrinsic width; the measure
    // reports bare text bounds, which is narrower than the cell's drawing
    // frame, so the full region width is its allocation and its response
    // is anchored at the leading edge of it.
    proposal::place_leading(
        state.label.view(),
        Rect::new(0.0, 0.0, bounds.size.width, f64::from(label.height)).translated(response.origin),
        ProposalSize::UNSPECIFIED,
        cocoa_ui::view::is_right_to_left(view),
    );

    let region_top = if has_label {
        f64::from(label.height) + VERTICAL_SPACING
    } else {
        0.0
    };
    let region_height = (bounds.size.height - region_top).max(0.0);
    let field_height = field_height(&state.field, bounds.size.width).min(region_height);
    #[cfg(target_os = "macos")]
    let field_frame = Rect::new(0.0, region_top, bounds.size.width, field_height);
    #[cfg(target_os = "ios")]
    let field_frame = Rect::new(
        0.0,
        region_top + (region_height - field_height) / 2.0,
        bounds.size.width,
        field_height,
    );
    let field_view: &PlatformView = &state.field;
    view::set_frame(field_view, field_frame.translated(response.origin));
}

/// The container's layout face: reports `WuiTextField.sizeThatFits`'s
/// answer — the proposed width floored at `max(label, 100pt)` and always
/// the intrinsic height — stretching horizontally at priority 0.
struct TextFieldSubView {
    state: Rc<FieldState>,
}

impl core::fmt::Debug for TextFieldSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextFieldSubView").finish_non_exhaustive()
    }
}

/// The width the field's own measure is taken at: the proposed width under
/// `WuiTextField`'s 100pt floor, and never narrower than the label.
#[cfg(target_os = "ios")]
fn measure_width(proposal: ProposalSize, label_width: f64) -> f64 {
    proposal
        .width
        .map_or(MIN_WIDTH, |w| f64::from(w).max(MIN_WIDTH))
        .max(label_width)
}

/// `AppKit` measures the field at its intrinsic height regardless of the
/// offer; the proposal is ignored the way `WuiTextField` ignores it.
#[cfg(target_os = "macos")]
const fn measure_width(_proposal: ProposalSize, _label_width: f64) -> f64 {
    0.0
}

impl SubView for TextFieldSubView {
    fn resolve_extent(
        &self,
        bounds: LayoutRect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    // `measure` speaks f32; the geometry math runs in f64 — the narrowing is
    // the layout contract, as in `text`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = &self.state;
        let label = state.label.layout().measure(ProposalSize::UNSPECIFIED).size;
        let has_label = label.height > 0.0;

        let text_height = field_height(
            &state.field,
            measure_width(proposal, f64::from(label.width)),
        );
        let intrinsic_height =
            f64::from(label.height) + if has_label { VERTICAL_SPACING } else { 0.0 } + text_height;
        let min_width = f64::from(label.width).max(MIN_WIDTH);
        let width = proposal
            .width
            .map_or(min_width, |w| f64::from(w).max(min_width));
        ViewDimensions::new(Size::new(width as f32, intrinsic_height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Applies the field's resolved input-trait configuration (`keyboard`,
/// `autocapitalization`, `autocorrect`, `submit_label`, `content_type`) onto
/// the platform text field.
fn configure_input_traits(field: &TextField, config: &TextFieldConfig<Held>) {
    #[cfg(target_os = "ios")]
    {
        field.set_keyboard(keyboard(config.keyboard()));
        field.set_autocapitalization(autocapitalization(config.autocapitalization()));
        field.set_autocorrection(autocorrection(config.autocorrect()));
        field.set_return_key(field_input::return_key(config.submit_label()));
    }
    #[cfg(target_os = "macos")]
    field.set_autocorrect(config.autocorrect());

    // The declared content type is the whole one-time-code feature on
    // both kits — `UITextContentType`/`NSTextContentType` is what the
    // system offers an SMS code for.
    if let Some(declared) = config.content_type() {
        field.set_content_type(field_input::content_type(declared));
    }
}

/// The `text_field` realization:
/// `Native<TextFieldConfig>` maps to a container with the platform
/// field and the mounted label child; binding, prompt, disabled state and
/// accessibility stay live through watchers.
pub fn realize_text_field_config(
    config: TextFieldConfig<Held>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let mtm = ctx.mtm();
    let host = HostView::new(mtm, Rect::ZERO);

    #[cfg(target_os = "ios")]
    let field = TextField::new(mtm);
    #[cfg(target_os = "macos")]
    let field = TextField::new(mtm, config.line_limit().snapshot().map(NonZeroUsize::get));

    let control_size = platform_control_size(config.size(), ControlSize::Small);
    field.set_control_size(control_size);
    configure_input_traits(&field, &config);

    let field_view: &PlatformView = &field;
    host.add_subview(field_view);

    // The label renders above the field; its semantic text names the
    // control while the visual label leaves the accessibility tree —
    // `WuiControlAccessibility`'s split.
    let (label, value, prompt, _, _, _, _, _, selection_menu, line_limit, on_submit) =
        config.into_parts();
    #[cfg(not(target_os = "macos"))]
    let _ = line_limit;
    let accessibility_label = label.accessibility_label();
    let host_view: &PlatformView = &host;
    let mounted = ctx.realize(label.build()).mount(host_view);
    view::hide_from_accessibility(mounted.view());

    let state = Rc::new(FieldState {
        field: field.clone(),
        label: mounted,
    });
    host.set_layout_handler({
        let state = Rc::clone(&state);
        move |view| layout_children(view, proposal::response(view), &state)
    });

    let control = view::retain_base(field_view);
    let mut leaf = NativeLeaf::new(
        host_view,
        TextFieldSubView {
            state: Rc::clone(&state),
        },
    );

    // Signal → field: the resolved attributed string for the current
    // binding value, re-pushed by every per-chunk style signal too.
    let value_state = styled_push(
        mtm,
        ctx.env(),
        platform_color(&Foreground.resolve(ctx.env()).snapshot()),
        &value.snapshot(),
        {
            let field = field.clone();
            move |state| {
                crate::measure_memo::invalidate();
                let attributed = render_attributed(state);
                #[cfg(target_os = "ios")]
                field.set_attributed_text(&attributed);
                #[cfg(target_os = "macos")]
                field.set_attributed_string(&attributed);
            }
        },
    );

    wire_value(&mut leaf, &value, ctx, &field, &value_state);

    #[cfg(target_os = "macos")]
    leaf.watch(&line_limit, {
        let field = field.clone();
        move |ctx| {
            field.set_line_limit(ctx.value().map(NonZeroUsize::get));
            crate::measure_memo::invalidate();
        }
    });

    // Return submits through the configured action.
    if let Some(on_submit) = on_submit {
        let env = ctx.env().clone();
        #[cfg(target_os = "ios")]
        leaf.keep(field.install_submit_handler(move |_| on_submit.run(&env)));
        #[cfg(target_os = "macos")]
        field.on_submit(move |_| on_submit.run(&env));
    }

    // The prompt renders into the placeholder under the placeholder
    // color — `WuiStyledStrRenderer`'s `defaultForegroundColor`.
    wire_prompt(mtm, &mut leaf, ctx, &field, &prompt);

    // The theme `Body` font is the field's own font, as in the secure
    // field: a single-line `NSTextField` cell positions its line by that
    // font, not by the attributed value or placeholder, and typing into
    // an empty field starts in it.
    #[cfg(target_os = "macos")]
    leaf.bind(&Body.resolve(ctx.env()), {
        let field = field.clone();
        move |font| {
            crate::measure_memo::invalidate();
            let font = control_font(font, control_size);
            field.set_font(&platform_font(mtm, &font, false));
        }
    });

    // Announce the label's semantic text on the field.
    leaf.bind(&accessibility_label, {
        move |styled| {
            let plain = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
            view::set_accessibility_label(&control, &plain);
        }
    });

    // A disabled subtree must not respond to input.
    let disabled = Disabled::resolve(ctx.env(), false);
    leaf.bind(&disabled, {
        let field = field.clone();
        move |is_disabled| field.set_enabled(!is_disabled)
    });

    // The field is this subtree's focus anchor: `Metadata<Focused>`
    // resolves it by walking the view hierarchy, as
    // `installWuiFocusTarget` marked it in Swift.
    leaf.keep(focus::install(mtm, field_view));

    // `selection_menu` rows ride the platform edit menu, on an
    // environment carrying the field's live `TextSelection`.
    install_selection_menu(mtm, ctx.env(), &selection_menu, &field, &value);

    leaf.keep(state);
    leaf.keep(value_state);
    leaf
}

/// A `TextSelection`'s byte range as the UTF-16 units the kit's selection
/// APIs count. An edge off a char boundary is a contract violation of the
/// caller; there is no range (`None`) and the command is ignored.
fn utf16_range(text: &str, range: &Range<usize>) -> Option<NSRange> {
    let edge = |byte: usize| {
        let edge = cocoa_ui::text::utf16_offset_at_byte(text, byte);
        if edge.is_none() {
            waterui_core::contract_violation!(
                "selection edge {byte} is not a UTF-8 boundary of the field's text; \
                 the selection command is ignored"
            );
        }
        edge
    };
    let location = edge(range.start)?;
    Some(NSRange::new(location, edge(range.end)? - location))
}

/// The field editor's UTF-16 range as `TextSelection`'s byte range. A
/// platform edge inside a surrogate pair or past the text is logged and
/// answers `None`.
fn utf16_byte_range(text: &str, range: NSRange) -> Option<Range<usize>> {
    let edge = |unit: usize| {
        let edge = cocoa_ui::text::byte_offset_at_utf16(text, unit);
        if edge.is_none() {
            tracing::error!(
                "the platform reported a selection edge {unit} inside a surrogate pair or past the text"
            );
        }
        edge
    };
    Some(edge(range.location)?..edge(range.location + range.length)?)
}

/// The [`CustomTextSelection`] a `selection_menu` command's `TextSelection`
/// delegates to: the field, reached weakly so the
/// handler the kit retains never keeps the leaf alive, plus the `value`
/// binding `styled_value` hands out. Every read answers at call time.
struct FieldSelection {
    field: Weak<TextField>,
    value: Binding<StyledStr>,
}

impl Clone for FieldSelection {
    fn clone(&self) -> Self {
        Self {
            field: self.field.clone(),
            value: self.value.clone(),
        }
    }
}

impl FieldSelection {
    /// The field with a live editing session, beginning one through the
    /// platform's own first-responder path when the selection is written
    /// outside an open session. `None` when the field is gone or cannot begin
    /// a session: the command is ignored.
    fn editable_field(&self) -> Option<Retained<TextField>> {
        let field = self.live_field()?;
        if field.selected_range().is_none() && !field.begin_editing() {
            tracing::error!(
                "the field cannot begin an editing session for the selection command; \
                 the command is ignored"
            );
            return None;
        }
        Some(field)
    }

    /// The field, or `None` after it was released: a selection command that
    /// outlives its field is a contract violation of the app holding the
    /// `TextSelection`, and answers as an empty selection.
    fn live_field(&self) -> Option<Retained<TextField>> {
        let field = self.field.load();
        if field.is_none() {
            waterui_core::contract_violation!(
                "a selection command fired after its field was released; \
                 the command answers as an empty selection"
            );
        }
        field
    }
}

impl CustomTextSelection for FieldSelection {
    fn text(&self) -> Str {
        self.live_field()
            .map_or_else(Str::default, |field| Str::from(field.string()))
    }

    fn range(&self) -> Range<usize> {
        let Some(field) = self.live_field() else {
            return 0..0;
        };
        let text = field.string();
        let Some(utf16) = field.selected_range() else {
            waterui_core::contract_violation!(
                "a selection command fired on a field with no editing session; \
                 the selection reads as empty"
            );
            return 0..0;
        };
        utf16_byte_range(&text, utf16).unwrap_or(0..0)
    }

    fn replace_range(&self, range: Range<usize>, inserted: &str) {
        let Some(field) = self.editable_field() else {
            return;
        };
        if let Some(utf16) = utf16_range(&field.string(), &range) {
            field.replace_range(utf16, &NSString::from_str(inserted));
        }
    }

    fn select(&self, range: Range<usize>) {
        let Some(field) = self.editable_field() else {
            return;
        };
        if let Some(utf16) = utf16_range(&field.string(), &range) {
            field.set_selected_range(utf16);
        }
    }

    fn styled_value(&self) -> Option<Binding<StyledStr>> {
        Some(self.value.clone())
    }
}

/// Rebuilds the `selection_menu` rows each time the field asks for its
/// edit menu: the item signal is snapshotted only then, and the
/// environment the rows resolve against carries the field's live
/// `TextSelection`, the same seat Hydrolysis installs it in.
fn install_selection_menu(
    #[cfg_attr(
        target_os = "macos",
        expect(unused_variables, reason = "only the UIKit path arms key commands")
    )]
    mtm: cocoa_ui::MainThreadMarker,
    env: &Environment,
    selection_menu: &Computed<Vec<ResolvedMenuItem<Held>>>,
    field: &TextField,
    value: &Binding<StyledStr>,
) {
    let driver = FieldSelection {
        field: Weak::new(field),
        value: value.clone(),
    };
    #[cfg(target_os = "macos")]
    appkit_edit_menu(field, driver, env.clone(), selection_menu.clone());
    #[cfg(target_os = "ios")]
    uikit_edit_menu(mtm, field, driver, env.clone(), selection_menu.clone());
}

/// The environment a selection menu resolves on: the render environment
/// plus the live `TextSelection`, so `Command` extraction and validation
/// see the selection.
fn menu_env(env: &Environment, driver: &FieldSelection) -> Environment {
    env.extending(State(TextSelection::new(driver.clone())))
}

/// `AppKit`: the handler appends the resolved rows to the field editor's
/// standard `NSMenu`, separated from the platform's own.
#[cfg(target_os = "macos")]
fn appkit_edit_menu(
    field: &TextField,
    driver: FieldSelection,
    env: Environment,
    items: Computed<Vec<ResolvedMenuItem<Held>>>,
) {
    field.install_edit_menu_handler(move |_, _, menu| {
        let nodes =
            crate::components::menu_items::tree_nodes(&items.snapshot(), &menu_env(&env, &driver));
        cocoa_ui::appkit::append_edit_menu(menu, &nodes);
    });
}

/// `UIKit`: the handler answers `textField:editMenuForCharactersInRanges:`
/// with the suggested system actions plus the resolved rows as an inline
/// group — `None` leaves the platform's default menu alone.
#[cfg(target_os = "ios")]
fn uikit_edit_menu(
    mtm: cocoa_ui::MainThreadMarker,
    field: &TextField,
    driver: FieldSelection,
    env: Environment,
    items: Computed<Vec<ResolvedMenuItem<Held>>>,
) {
    // The scope arming the current menu's shortcut commands, retired with
    // the next presentation — the lifespan `KeyCommands` documents.
    let scopes = Rc::new(RefCell::new(Option::<cocoa_ui::uikit::KeyCommands>::None));
    field.install_edit_menu_handler(move |field, suggested| {
        let nodes =
            crate::components::menu_items::tree_nodes(&items.snapshot(), &menu_env(&env, &driver));
        if nodes.is_empty() {
            // Retire the previous presentation's scope too — an empty
            // menu arms nothing, not the last menu's commands.
            scopes.borrow_mut().take();
            return None;
        }
        let scope = field.edit_menu_commands();
        let menu = cocoa_ui::uikit::edit_menu(mtm, &scope, suggested, &nodes);
        scopes.borrow_mut().replace(scope);
        Some(menu)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measure constants `WuiTextField` fixes: a 4pt gap and a 100pt
    /// floor.
    #[test]
    fn measure_constants_match_the_swift_values() {
        assert_eq!(VERTICAL_SPACING.to_bits(), 4.0_f64.to_bits());
        assert_eq!(MIN_WIDTH.to_bits(), 100.0_f64.to_bits());
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn keyboard_maps_the_swift_table() {
        let table = [
            (KeyboardType::Text, platform::Keyboard::Text),
            (KeyboardType::Email, platform::Keyboard::Email),
            (KeyboardType::URL, platform::Keyboard::Url),
            (KeyboardType::Number, platform::Keyboard::Number),
            (KeyboardType::Decimal, platform::Keyboard::Decimal),
            (KeyboardType::PhoneNumber, platform::Keyboard::PhoneNumber),
        ];
        for (source, expected) in table {
            assert_eq!(keyboard(source), expected);
        }
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn capitalization_autocorrection_and_return_key_map_every_variant() {
        assert_eq!(
            autocapitalization(Capitalization::None),
            platform::Autocapitalization::None
        );
        assert_eq!(
            autocapitalization(Capitalization::Words),
            platform::Autocapitalization::Words
        );
        assert_eq!(
            autocapitalization(Capitalization::Sentences),
            platform::Autocapitalization::Sentences
        );
        assert_eq!(
            autocapitalization(Capitalization::Characters),
            platform::Autocapitalization::AllCharacters
        );

        assert_eq!(autocorrection(true), platform::Autocorrection::Enabled);
        assert_eq!(autocorrection(false), platform::Autocorrection::Disabled);

        let labels = [
            (
                waterui_controls::input::SubmitLabel::Return,
                cocoa_ui::uikit::text_input::ReturnKey::Default,
            ),
            (
                waterui_controls::input::SubmitLabel::Done,
                cocoa_ui::uikit::text_input::ReturnKey::Done,
            ),
            (
                waterui_controls::input::SubmitLabel::Go,
                cocoa_ui::uikit::text_input::ReturnKey::Go,
            ),
            (
                waterui_controls::input::SubmitLabel::Next,
                cocoa_ui::uikit::text_input::ReturnKey::Next,
            ),
            (
                waterui_controls::input::SubmitLabel::Search,
                cocoa_ui::uikit::text_input::ReturnKey::Search,
            ),
            (
                waterui_controls::input::SubmitLabel::Send,
                cocoa_ui::uikit::text_input::ReturnKey::Send,
            ),
            (
                waterui_controls::input::SubmitLabel::Join,
                cocoa_ui::uikit::text_input::ReturnKey::Join,
            ),
            (
                waterui_controls::input::SubmitLabel::Continue,
                cocoa_ui::uikit::text_input::ReturnKey::Continue,
            ),
        ];
        for (source, expected) in labels {
            assert_eq!(field_input::return_key(source), expected);
        }
    }

    /// `WuiKeyboardType`'s table is exercised on `UIKit`; on `AppKit` there
    /// is no hint to map, so the test only anchors the width floor.
    #[cfg(target_os = "macos")]
    #[test]
    fn measure_width_ignores_the_proposal() {
        let proposal = ProposalSize::new(Some(42.0), None);
        assert_eq!(measure_width(proposal, 60.0).to_bits(), 0.0_f64.to_bits());
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn measure_width_floors_at_100_and_the_label() {
        let proposal = ProposalSize::new(Some(42.0), None);
        assert_eq!(measure_width(proposal, 0.0).to_bits(), 100.0_f64.to_bits());
        assert_eq!(
            measure_width(proposal, 250.0).to_bits(),
            250.0_f64.to_bits()
        );
        let unspecified = ProposalSize::UNSPECIFIED;
        assert_eq!(
            measure_width(unspecified, 0.0).to_bits(),
            100.0_f64.to_bits()
        );
    }
}
