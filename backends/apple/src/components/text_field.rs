//! The `text_field` leaf: `Native<ResolvedTextFieldConfig>` rendered
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
//! over the field with 4pt of spacing, height always at least intrinsic.
//!
//! The field is also this subtree's focus anchor — the kit marks it so
//! `Metadata<Focused>` resolves it the way `installWuiFocusTarget` marked
//! it in Swift. `WuiSecureField` and the selection menu stay in the
//! Swift fallback until their own ports land.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;

use cocoa_ui::{PlatformView, Rect, Retained, focus, view};
use waterui::Str;
#[cfg(target_os = "ios")]
use waterui::component::text_field::KeyboardType;
use waterui::component::text_field::ResolvedTextFieldConfig;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::{Binding, Signal};
use waterui::resolve::Resolvable;
use waterui::text::StyledStr;
use waterui::text::font::{FontDesign, FontWeight, ResolvedFont};
use waterui::theme::color::Foreground;
use waterui_backend_core::Environment;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{
    HorizontalAlignment, ProposalSize, Size, StretchAxis, SubView, ViewDimensions,
};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::colors;
    pub(super) use cocoa_ui::appkit::{HostView, TextField};
    pub(super) use cocoa_ui::objc2_app_kit::{NSColor, NSTextAlignment};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::{NSTextAlignment, UIColor};
    pub(super) use cocoa_ui::uikit::colors;
    pub(super) use cocoa_ui::uikit::text_field::Keyboard;
    pub(super) use cocoa_ui::uikit::{HostView, TextField};
}

use platform::{HostView, TextField};

/// The platform color a [`StyledPush`]'s default foreground draws in.
#[cfg(target_os = "macos")]
type PlatformColor = platform::NSColor;
/// The platform color a [`StyledPush`]'s default foreground draws in.
#[cfg(target_os = "ios")]
type PlatformColor = platform::UIColor;

/// The gap between the label and the field — `WuiTextField.verticalSpacing`.
const VERTICAL_SPACING: f64 = 4.0;
/// The narrowest width the leaf reports — `WuiTextField`'s 100pt floor.
const MIN_WIDTH: f64 = 100.0;

/// A styled chunk resolved into its current values, as in `text`.
struct Chunk {
    /// The chunk's characters.
    text: Str,
    /// Its latest resolved font.
    font: ResolvedFont,
    /// Its latest foreground — `None` draws the state's default.
    foreground: Option<WorkingColor>,
    /// Its latest background.
    background: Option<WorkingColor>,
    /// Whether the chunk is italicized.
    italic: bool,
    /// Whether the chunk is underlined.
    underline: bool,
    /// Whether the chunk is struck through.
    strikethrough: bool,
}

/// A `StyledStr` the leaf renders into an attributed string and pushes to a
/// kit setter: the field's `value` (theme `Foreground` default) or its
/// `prompt` (placeholder-color default). Same model as `text`'s `TextState`
/// — per-chunk signal watches re-push the rebuilt string.
struct StyledPush {
    /// Proof of the main thread for font construction inside watchers.
    mtm: cocoa_ui::MainThreadMarker,
    /// The environment chunks resolve through.
    env: Environment,
    /// The color chunks without their own foreground draw in.
    default_foreground: Retained<PlatformColor>,
    /// The resolved chunks of the current `StyledStr`.
    chunks: Vec<Chunk>,
    /// Watcher guards of the current `StyledStr`'s per-chunk signals,
    /// replaced wholesale when the value changes.
    signal_guards: Vec<Box<dyn Any>>,
    /// Where the rebuilt attributed string goes.
    push: Box<dyn Fn(&Self)>,
}

impl core::fmt::Debug for StyledPush {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StyledPush").finish_non_exhaustive()
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
#[cfg(target_os = "ios")]
fn platform_color(color: &WorkingColor) -> Retained<PlatformColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object, with HDR
/// headroom applied as a content-headroom multiplier — the `AppKit` variant.
#[cfg(target_os = "macos")]
fn platform_color(color: &WorkingColor) -> Retained<PlatformColor> {
    {
        let [red, green, blue, alpha] = color.components;
        platform::colors::extended_linear_display_p3(
            f64::from(red),
            f64::from(green),
            f64::from(blue),
            f64::from(alpha),
        )
    }
}

/// The platform weight of a `FontWeight` on the `UIFont`/`NSFont` scale.
const fn platform_weight(weight: FontWeight) -> f64 {
    use cocoa_ui::font::weight;
    match weight {
        FontWeight::Thin => weight::THIN,
        FontWeight::UltraLight => weight::ULTRA_LIGHT,
        FontWeight::Light => weight::LIGHT,
        FontWeight::Normal => weight::REGULAR,
        FontWeight::Medium => weight::MEDIUM,
        FontWeight::SemiBold => weight::SEMI_BOLD,
        FontWeight::Bold => weight::BOLD,
        FontWeight::UltraBold => weight::HEAVY,
        FontWeight::Black => weight::BLACK,
    }
}

/// The comma-separated candidates of a CSS-style family list — as in `text`.
fn family_candidates(family: &str) -> impl Iterator<Item = &str> {
    family
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
}

/// The platform face a resolved font names — the same rule as `text`.
///
/// # Panics
///
/// When the family names no installed font and no generic — the same
/// `fatalError` the Swift port raises.
fn platform_font(
    mtm: cocoa_ui::MainThreadMarker,
    resolved: &ResolvedFont,
    italic: bool,
) -> Retained<cocoa_ui::Font> {
    let size = f64::from(resolved.size);
    let weight = platform_weight(resolved.weight);
    let mut font = match resolved.family.as_deref() {
        Some(family) if !family.is_empty() => {
            let mut resolved_font = None;
            for candidate in family_candidates(family) {
                resolved_font = match candidate {
                    "system" | "sans-serif" => Some(cocoa_ui::font::system(mtm, size, weight)),
                    _ => cocoa_ui::font::named(candidate, size),
                };
                if resolved_font.is_some() {
                    break;
                }
            }
            resolved_font.unwrap_or_else(|| {
                panic!(
                    "WaterUI: font family '{family}' not found. Ensure the font is bundled and registered."
                )
            })
        }
        _ => match resolved.design {
            FontDesign::Default => cocoa_ui::font::system(mtm, size, weight),
            FontDesign::Monospaced => cocoa_ui::font::monospaced(mtm, size, weight),
        },
    };
    if italic && let Some(italic_font) = cocoa_ui::font::italic_variant(&font) {
        font = italic_font;
    }
    font
}

/// The attributed string the current chunks render into — `text`'s builder
/// run against this state's default foreground.
fn render_attributed(state: &StyledPush) -> Retained<objc2_foundation::NSAttributedString> {
    let fonts: Vec<Retained<cocoa_ui::Font>> = state
        .chunks
        .iter()
        .map(|chunk| platform_font(state.mtm, &chunk.font, chunk.italic))
        .collect();
    let foregrounds: Vec<Option<Retained<PlatformColor>>> = state
        .chunks
        .iter()
        .map(|chunk| {
            Some(
                chunk
                    .foreground
                    .as_ref()
                    .map_or_else(|| state.default_foreground.clone(), platform_color),
            )
        })
        .collect();
    let backgrounds: Vec<Option<Retained<PlatformColor>>> = state
        .chunks
        .iter()
        .map(|chunk| chunk.background.as_ref().map(platform_color))
        .collect();
    let runs: Vec<cocoa_ui::text::TextRun<'_>> = state
        .chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| cocoa_ui::text::TextRun {
            text: chunk.text.as_str(),
            font: &fonts[i],
            foreground: foregrounds[i].as_deref(),
            background: backgrounds[i].as_deref(),
            underline: chunk.underline,
            strikethrough: chunk.strikethrough,
            letter_spacing: f64::from(chunk.font.letter_spacing),
            line_height: chunk.font.line_height.map_or(0.0, f64::from),
        })
        .collect();
    cocoa_ui::text::build(state.mtm, &runs).into_super()
}

/// Rebuilds the attributed string and pushes it through the state's setter.
fn push(state: &StyledPush) {
    (state.push)(state);
}

/// A live `StyledPush` holding `styled`'s resolved chunks and signal
/// watches, already pushed once — the leaf then watches the value signal
/// and re-runs [`apply_styled`] on it.
fn styled_push(
    mtm: cocoa_ui::MainThreadMarker,
    env: &Environment,
    default_foreground: Retained<PlatformColor>,
    styled: &StyledStr,
    push: impl Fn(&StyledPush) + 'static,
) -> Rc<RefCell<StyledPush>> {
    let state = Rc::new(RefCell::new(StyledPush {
        mtm,
        env: env.clone(),
        default_foreground,
        chunks: Vec::new(),
        signal_guards: Vec::new(),
        push: Box::new(push),
    }));
    apply_styled(&state, styled);
    self::push(&state.borrow());
    state
}

/// Resolves `styled` into chunk state plus the per-chunk signal watches
/// that re-push a rebuild — the work `text`'s `apply_styled` performs.
fn apply_styled(state: &Rc<RefCell<StyledPush>>, styled: &StyledStr) {
    let env = state.borrow().env.clone();
    let mut chunks = Vec::with_capacity(styled.chunks().len());
    let mut guards: Vec<Box<dyn Any>> = Vec::new();
    for (index, (text, style)) in styled.chunks().iter().enumerate() {
        let font = style.font.resolve(&env);
        let foreground = style.foreground.as_ref().map(|color| color.resolve(&env));
        let background = style.background.as_ref().map(|color| color.resolve(&env));
        chunks.push(Chunk {
            text: text.clone(),
            font: font.snapshot(),
            foreground: foreground.as_ref().map(Signal::snapshot),
            background: background.as_ref().map(Signal::snapshot),
            italic: style.italic,
            underline: style.underline,
            strikethrough: style.strikethrough,
        });

        guards.push(Box::new(font.watch({
            let state = Rc::clone(state);
            move |ctx| {
                let mut state = state.borrow_mut();
                state.chunks[index].font = ctx.into_value();
                push(&state);
            }
        })));
        if let Some(signal) = foreground {
            guards.push(Box::new(signal.watch({
                let state = Rc::clone(state);
                move |ctx| {
                    let mut state = state.borrow_mut();
                    state.chunks[index].foreground = Some(ctx.into_value());
                    push(&state);
                }
            })));
        }
        if let Some(signal) = background {
            guards.push(Box::new(signal.watch({
                let state = Rc::clone(state);
                move |ctx| {
                    let mut state = state.borrow_mut();
                    state.chunks[index].background = Some(ctx.into_value());
                    push(&state);
                }
            })));
        }
    }
    let mut state = state.borrow_mut();
    state.chunks = chunks;
    state.signal_guards = guards;
}

/// The `HorizontalAlignment` mapping `WuiTextField.applyPromptAlignment`
/// applies: `Center` centers, `Trailing` flips with the reading direction,
/// anything else stays natural.
fn apply_alignment(field: &TextField, alignment: HorizontalAlignment) {
    use platform::NSTextAlignment;
    let field_view: &PlatformView = field;
    let alignment = if alignment == HorizontalAlignment::Center {
        NSTextAlignment::Center
    } else if alignment == HorizontalAlignment::Trailing {
        if view::is_right_to_left(field_view) {
            NSTextAlignment::Left
        } else {
            NSTextAlignment::Right
        }
    } else {
        NSTextAlignment::Natural
    };
    field.set_text_alignment(alignment);
}

/// The `Keyboard` a `KeyboardType` asks for — `WuiKeyboardType.uiKeyboardType`'s
/// table. `AppKit` has no on-screen keyboard; there the hint is unused.
#[cfg(target_os = "ios")]
const fn keyboard(kind: &KeyboardType) -> platform::Keyboard {
    match kind {
        KeyboardType::Text => platform::Keyboard::Text,
        KeyboardType::Email => platform::Keyboard::Email,
        KeyboardType::URL => platform::Keyboard::Url,
        KeyboardType::Number => platform::Keyboard::Number,
        KeyboardType::PhoneNumber => platform::Keyboard::PhoneNumber,
        _ => panic!("unsupported WaterUI keyboard type"),
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

/// The watcher a new `StyledStr` value installs on its push state:
/// re-resolve every chunk signal, then push the rebuild.
fn restyle(
    state: &Rc<RefCell<StyledPush>>,
) -> impl Fn(waterui::reactive::watcher::Context<StyledStr>) + 'static + use<> {
    let state = Rc::clone(state);
    move |ctx| {
        apply_styled(&state, ctx.value());
        push(&state.borrow());
    }
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
    leaf.watch(value, restyle(value_state));
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
        move |field| binding.set(StyledStr::plain(field.string()))
    }));
    #[cfg(target_os = "macos")]
    field.on_change({
        let binding = value.clone();
        move |field| binding.set(StyledStr::plain(field.string()))
    });
}

/// The leaf's live state: the platform field and the mounted label child.
struct FieldState {
    field: Retained<TextField>,
    label: Mounted,
}

/// Lays out the children inside `view`'s bounds: the label at top-leading,
/// then the field spanning the full width below it — `WuiTextField`'s
/// constraints as manual frames. `UIKit` centers the input inside the
/// region between the label and the container's bottom, the way the
/// field's `inputRegion` layout guide does.
fn layout_children(view: &PlatformView, state: &FieldState) {
    let bounds = view::bounds(view);
    let label = state.label.layout().measure(ProposalSize::UNSPECIFIED).size;
    let has_label = label.height > 0.0;
    view::set_hidden(state.label.view(), !has_label);

    // The label pins top+leading and keeps its intrinsic width; the measure
    // reports bare text bounds, which is narrower than the cell's drawing
    // frame, so the full region width is what keeps it unclipped.
    view::set_frame(
        state.label.view(),
        Rect::new(0.0, 0.0, bounds.size.width, f64::from(label.height)),
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
    view::set_frame(field_view, field_frame);
}

/// The container's layout face: reports `WuiTextField.sizeThatFits`'s
/// answer — the proposed width floored at `max(label, 100pt)` and height
/// never below intrinsic — stretching horizontally at priority 0.
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
        let height = proposal
            .height
            .map_or(intrinsic_height, |h| f64::from(h).max(intrinsic_height));
        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `text_field` handler on the dispatcher:
/// `Native<ResolvedTextFieldConfig>` maps to a container with the platform
/// field and the mounted label child; binding, prompt, disabled state and
/// accessibility stay live through watchers.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ResolvedTextFieldConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);

        #[cfg(target_os = "ios")]
        let field = {
            let field = TextField::new(mtm);
            field.set_keyboard(keyboard(&config.keyboard));
            field
        };
        #[cfg(target_os = "macos")]
        let field = TextField::new(mtm, config.line_limit.map(core::num::NonZero::get));

        let field_view: &PlatformView = &field;
        host.add_subview(field_view);

        // The label renders above the field; its semantic text names the
        // control while the visual label leaves the accessibility tree —
        // `WuiControlAccessibility`'s split.
        let accessibility_label = config.label.accessibility_label();
        let host_view: &PlatformView = &host;
        let mounted = ctx
            .render(waterui_backend_core::AnyView::new(config.label))
            .mount(host_view);
        view::hide_from_accessibility(mounted.view());

        let state = Rc::new(FieldState {
            field: field.clone(),
            label: mounted,
        });
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state)
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
            &config.value.snapshot(),
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

        wire_value(&mut leaf, &config.value, ctx, &field, &value_state);

        // Return submits through the configured action.
        if let Some(on_submit) = config.on_submit {
            let env = ctx.env().clone();
            #[cfg(target_os = "ios")]
            leaf.keep(field.install_submit_handler(move |_| on_submit.call(&env)));
            #[cfg(target_os = "macos")]
            field.on_submit(move |_| on_submit.call(&env));
        }

        // The prompt renders into the placeholder under the placeholder
        // color — `WuiStyledStrRenderer`'s `defaultForegroundColor`.
        let prompt_state = styled_push(
            mtm,
            ctx.env(),
            platform::colors::placeholder_text(),
            &config.prompt.content.snapshot(),
            {
                let field = field.clone();
                move |state| {
                    crate::measure_memo::invalidate();
                    field.set_placeholder(&render_attributed(state));
                }
            },
        );
        leaf.watch(&config.prompt.content, restyle(&prompt_state));
        leaf.bind(&config.prompt.paragraph_alignment, {
            let field = field.clone();
            move |alignment| apply_alignment(&field, alignment)
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

        leaf.keep(state);
        leaf.keep(value_state);
        leaf.keep(prompt_state);
        leaf
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
            (KeyboardType::PhoneNumber, platform::Keyboard::PhoneNumber),
        ];
        for (source, expected) in table {
            assert_eq!(keyboard(&source), expected);
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
