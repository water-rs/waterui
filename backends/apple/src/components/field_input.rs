//! Shared input helpers for [`super::text_field`] and [`super::secure_field`]:
//! reactive styled-text pushing, prompt + alignment wiring, and
//! `ContentType` / `SubmitLabel` mapping onto `cocoa-ui`.

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;

use cocoa_ui::{PlatformView, Retained, view};
use nami::Signal;
use objc2_foundation::NSAttributedString;
use waterui_backend_core::Environment;
use waterui_controls::input::ContentType;
#[cfg(target_os = "ios")]
use waterui_controls::input::SubmitLabel;
use waterui_core::Str;
use waterui_core::layout::HorizontalAlignment;
use waterui_graphics::color::WorkingColor;
use waterui_text::StyledStr;
use waterui_text::font::ResolvedFont;

use super::font::platform_font;
use crate::contract::{NativeLeaf, RenderContext};

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::colors;
    pub(super) use cocoa_ui::objc2_app_kit::{NSColor, NSTextAlignment};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::objc2_ui_kit::{NSTextAlignment, UIColor};
    pub(super) use cocoa_ui::uikit::colors;
    pub(super) use cocoa_ui::uikit::text_input::ReturnKey;
}

/// The platform color a [`StyledPush`]'s default foreground draws in.
#[cfg(target_os = "macos")]
pub(super) type PlatformColor = platform::NSColor;
/// The platform color a [`StyledPush`]'s default foreground draws in.
#[cfg(target_os = "ios")]
pub(super) type PlatformColor = platform::UIColor;

/// A styled chunk resolved into its current values, as in `text`.
struct Chunk {
    text: Str,
    font: ResolvedFont,
    foreground: Option<WorkingColor>,
    background: Option<WorkingColor>,
    italic: bool,
    underline: bool,
    strikethrough: bool,
}

/// A `StyledStr` a field renders into an attributed string and pushes to a
/// kit setter: the field's `value` (theme `Foreground` default) or its
/// `prompt` (placeholder-color default).
pub(super) struct StyledPush {
    mtm: cocoa_ui::MainThreadMarker,
    env: Environment,
    pub(super) default_foreground: Retained<PlatformColor>,
    chunks: Vec<Chunk>,
    signal_guards: Vec<Box<dyn Any>>,
    push: Box<dyn Fn(&Self)>,
}

impl core::fmt::Debug for StyledPush {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StyledPush").finish_non_exhaustive()
    }
}

/// A `WorkingColor` as the platform's extended linear Display-P3 color object.
pub(super) fn platform_color(color: &WorkingColor) -> Retained<PlatformColor> {
    let [red, green, blue, alpha] = color.components;
    platform::colors::extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// The attributed string the current chunks render into — `text`'s builder
/// run against this state's default foreground.
pub(super) fn render_attributed(state: &StyledPush) -> Retained<NSAttributedString> {
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
            letter_spacing: f64::from(chunk.font.letter_spacing()),
            line_height: chunk.font.line_height().map_or(0.0, f64::from),
        })
        .collect();
    cocoa_ui::text::build(state.mtm, &runs).into_super()
}

/// Rebuilds the attributed string and pushes it through the state's setter.
pub(super) fn push(state: &StyledPush) {
    (state.push)(state);
}

/// A live `StyledPush` holding `styled`'s resolved chunks and signal
/// watches, already pushed once.
pub(super) fn styled_push(
    mtm: cocoa_ui::MainThreadMarker,
    env: &Environment,
    default_foreground: Retained<PlatformColor>,
    styled: &StyledStr,
    push_fn: impl Fn(&StyledPush) + 'static,
) -> Rc<RefCell<StyledPush>> {
    let state = Rc::new(RefCell::new(StyledPush {
        mtm,
        env: env.clone(),
        default_foreground,
        chunks: Vec::new(),
        signal_guards: Vec::new(),
        push: Box::new(push_fn),
    }));
    apply_styled(&state, styled);
    push(&state.borrow());
    state
}

/// Resolves `styled` into chunk state plus the per-chunk signal watches
/// that re-push a rebuild.
pub(super) fn apply_styled(state: &Rc<RefCell<StyledPush>>, styled: &StyledStr) {
    let env = state.borrow().env.clone();
    let mut chunks = Vec::with_capacity(styled.chunks().len());
    let mut guards: Vec<Box<dyn Any>> = Vec::new();
    for (index, (text, style)) in styled.chunks().iter().enumerate() {
        let font = style.get_font().resolve(&env);
        let foreground = style
            .get_foreground()
            .as_ref()
            .map(|color| color.resolve(&env));
        let background = style
            .get_background()
            .as_ref()
            .map(|color| color.resolve(&env));
        chunks.push(Chunk {
            text: text.clone(),
            font: font.snapshot(),
            foreground: foreground.as_ref().map(Signal::snapshot),
            background: background.as_ref().map(Signal::snapshot),
            italic: style.get_italic(),
            underline: style.get_underline(),
            strikethrough: style.get_strikethrough(),
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

/// The watcher a new `StyledStr` value installs on its push state:
/// re-resolve every chunk signal, then push the rebuild.
pub(super) fn restyle(
    state: &Rc<RefCell<StyledPush>>,
) -> impl Fn(nami::watcher::Context<StyledStr>) + 'static + use<> {
    let state = Rc::clone(state);
    move |ctx| {
        apply_styled(&state, ctx.value());
        push(&state.borrow());
    }
}

/// Maps `HorizontalAlignment` to `NSTextAlignment`, flipping `Trailing` with
/// the field's reading direction.
pub(super) fn resolve_alignment(
    field_view: &PlatformView,
    alignment: HorizontalAlignment,
) -> platform::NSTextAlignment {
    use platform::NSTextAlignment;
    if alignment == HorizontalAlignment::Center {
        NSTextAlignment::Center
    } else if alignment == HorizontalAlignment::Trailing {
        if view::is_right_to_left(field_view) {
            NSTextAlignment::Left
        } else {
            NSTextAlignment::Right
        }
    } else {
        NSTextAlignment::Natural
    }
}

/// Wires a field's styled placeholder and paragraph alignment through the
/// shared [`StyledPush`] pipeline.
pub(super) fn wire_prompt(
    mtm: cocoa_ui::MainThreadMarker,
    leaf: &mut NativeLeaf,
    ctx: &RenderContext<'_>,
    prompt: &waterui_text::TextConfig,
    set_placeholder: impl Fn(&NSAttributedString) + 'static,
    set_alignment: impl Fn(HorizontalAlignment) + 'static,
) {
    let content = prompt.content();
    let state = styled_push(
        mtm,
        ctx.env(),
        platform::colors::placeholder_text(),
        &content.snapshot(),
        move |state| {
            crate::measure_memo::invalidate();
            set_placeholder(&render_attributed(state));
        },
    );
    leaf.watch(&content, restyle(&state));
    leaf.bind(prompt.paragraph_alignment(), set_alignment);
    leaf.keep(state);
}

/// The kit `ContentType` a `WaterUI` `ContentType` asks for —
/// `UITextContentType`/`NSTextContentType` share the vocabulary, so one
/// table covers both platforms.
pub(super) const fn content_type(kind: ContentType) -> cocoa_ui::ContentType {
    match kind {
        ContentType::Username => cocoa_ui::ContentType::Username,
        ContentType::Password => cocoa_ui::ContentType::Password,
        ContentType::NewPassword => cocoa_ui::ContentType::NewPassword,
        ContentType::EmailAddress => cocoa_ui::ContentType::EmailAddress,
        ContentType::PhoneNumber => cocoa_ui::ContentType::PhoneNumber,
        ContentType::OneTimeCode => cocoa_ui::ContentType::OneTimeCode,
        ContentType::PersonName => cocoa_ui::ContentType::PersonName,
        ContentType::PostalAddress => cocoa_ui::ContentType::PostalAddress,
        ContentType::PostalCode => cocoa_ui::ContentType::PostalCode,
        ContentType::CreditCardNumber => cocoa_ui::ContentType::CreditCardNumber,
    }
}

/// The `UIKit` return-key type a `SubmitLabel` asks for.
#[cfg(target_os = "ios")]
pub(super) fn return_key(label: SubmitLabel) -> platform::ReturnKey {
    match label {
        SubmitLabel::Return => platform::ReturnKey::Default,
        SubmitLabel::Done => platform::ReturnKey::Done,
        SubmitLabel::Go => platform::ReturnKey::Go,
        SubmitLabel::Next => platform::ReturnKey::Next,
        SubmitLabel::Search => platform::ReturnKey::Search,
        SubmitLabel::Send => platform::ReturnKey::Send,
        SubmitLabel::Join => platform::ReturnKey::Join,
        SubmitLabel::Continue => platform::ReturnKey::Continue,
        _ => {
            waterui_core::contract_violation!(
                "unsupported WaterUI submit label; the default return key is used"
            );
            platform::ReturnKey::Default
        }
    }
}
