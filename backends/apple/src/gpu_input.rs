//! Translates cocoa-ui's platform input vocabulary into
//! `waterui-graphics`' `SurfaceInputEvent` — what `WuiGpuSurfaceInput.swift`
//! did for the Swift surface.

use waterui_graphics::input::SurfaceInputEvent;

/// Converts a [`cocoa_ui::input::SurfaceEvent`] into the event a
/// [`GpuContentView`](waterui_graphics::gpu::GpuContentView) consumes.
///
/// The kit vocabulary mirrors the platform events it translates; the mapping
/// is one-to-one except pointer buttons with no W3C meaning, which the kit
/// already declined to deliver.
#[allow(clippy::too_many_lines)]
pub fn translate(event: &cocoa_ui::input::SurfaceEvent) -> SurfaceInputEvent {
    use cocoa_ui::input::SurfaceEvent as E;
    use waterui_graphics::input::ScrollUnit;
    match event {
        E::Focus(focused) => SurfaceInputEvent::Focus(*focused),
        E::Modifiers(modifiers) => SurfaceInputEvent::Modifiers(*modifiers),
        E::PointerMove { position } => SurfaceInputEvent::PointerMove {
            position: kurbo::Point::new(position.x, position.y),
        },
        E::PointerButton {
            pressed,
            button,
            position,
        } => SurfaceInputEvent::PointerButton {
            pressed: *pressed,
            button: pointer_button(*button),
            position: kurbo::Point::new(position.x, position.y),
        },
        E::Scroll {
            position,
            delta_x,
            delta_y,
            unit,
            finished,
        } => SurfaceInputEvent::Scroll {
            position: kurbo::Point::new(position.x, position.y),
            delta_x: *delta_x,
            delta_y: *delta_y,
            unit: match unit {
                cocoa_ui::input::ScrollUnit::Line => ScrollUnit::Line,
                cocoa_ui::input::ScrollUnit::Pixel => ScrollUnit::Pixel,
            },
            finished: *finished,
        },
        E::Key {
            pressed,
            key,
            code,
            modifiers,
            repeat,
        } => SurfaceInputEvent::Key {
            pressed: *pressed,
            key: key.clone(),
            code: *code,
            modifiers: *modifiers,
            repeat: *repeat,
        },
        E::TextInput(text) => SurfaceInputEvent::TextInput(text.clone().into()),
        E::CompositionStart => SurfaceInputEvent::CompositionStart,
        E::CompositionUpdate { text, caret } => SurfaceInputEvent::CompositionUpdate {
            text: text.clone().into(),
            caret: *caret,
        },
        E::CompositionCommit(text) => SurfaceInputEvent::CompositionCommit(text.clone().into()),
        E::CompositionCancel => SurfaceInputEvent::CompositionCancel,
    }
}

const fn pointer_button(
    button: cocoa_ui::input::PointerButton,
) -> waterui_graphics::input::SurfacePointerButton {
    use waterui_graphics::input::SurfacePointerButton;
    match button {
        cocoa_ui::input::PointerButton::Primary => SurfacePointerButton::Primary,
        cocoa_ui::input::PointerButton::Secondary => SurfacePointerButton::Secondary,
        cocoa_ui::input::PointerButton::Middle => SurfacePointerButton::Middle,
        cocoa_ui::input::PointerButton::Back => SurfacePointerButton::Back,
        cocoa_ui::input::PointerButton::Forward => SurfacePointerButton::Forward,
    }
}
