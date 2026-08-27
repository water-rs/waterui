use super::*;
use crate::platform::NativeKey;
use waterui_graphics::input::ScrollUnit;

/// Backend-owned input adapter for one embedded browser surface.
///
/// The engine bridges (CEF, WPE) speak this vocabulary. It is the narrower,
/// older half of [`EmbeddedInputSink`]: no physical key code, no composition
/// session, no scroll-gesture end. New embedded surfaces implement the neutral
/// sink directly; these engines move over when they are ported onto
/// [`SurfaceInputEvent`](waterui_graphics::input::SurfaceInputEvent).
pub(crate) trait BrowserInputHandler {
    fn set_focus(&self, focused: bool);
    fn set_modifiers(&self, modifiers: Modifiers);
    fn pointer_move(&self, position: vello::kurbo::Point);
    fn pointer_button(&self, pressed: bool, button: PointerButton, position: vello::kurbo::Point);
    fn scroll(
        &self,
        position: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        is_line_delta: bool,
    );
    fn key(&self, pressed: bool, key: &KeyCode, native: Option<NativeKey>);
    fn text_input(&self, text: &str);
    fn commit_text(&self, text: &str);
}

/// Presents a [`BrowserInputHandler`] as an [`EmbeddedInputSink`], so browsers
/// and GPU surfaces share one target list, one hit-test arbitration and one
/// focus/capture state machine.
pub(crate) struct BrowserInputSink {
    handler: Rc<dyn BrowserInputHandler>,
}

impl EmbeddedInputSink for BrowserInputSink {
    fn identity(&self) -> *const () {
        Rc::as_ptr(&self.handler) as *const ()
    }

    fn set_focus(&self, focused: bool) {
        self.handler.set_focus(focused);
    }

    fn set_modifiers(&self, modifiers: Modifiers) {
        self.handler.set_modifiers(modifiers);
    }

    fn pointer_move(&self, position: vello::kurbo::Point) {
        self.handler.pointer_move(position);
    }

    fn pointer_button(&self, pressed: bool, button: PointerButton, position: vello::kurbo::Point) {
        self.handler.pointer_button(pressed, button, position);
    }

    fn scroll(
        &self,
        position: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        unit: ScrollUnit,
        _finished: bool,
    ) {
        self.handler
            .scroll(position, delta_x, delta_y, matches!(unit, ScrollUnit::Line));
    }

    fn key(&self, delivery: &KeyDelivery<'_>) {
        // The engines read the modifier state off the handler rather than off
        // the key event, so it is published first, exactly as before.
        self.handler.set_modifiers(delivery.modifiers);
        self.handler
            .key(delivery.pressed, delivery.key, delivery.native);
    }

    fn text_input(&self, text: &str) {
        self.handler.text_input(text);
    }

    fn composition_start(&self) {}

    fn composition_update(&self, _text: &str, _caret: Option<usize>) {
        // The engines render their own pre-edit through the platform IME; an
        // in-progress composition is not forwarded to them.
    }

    fn composition_commit(&self, text: &str) {
        self.handler.commit_text(text);
    }

    fn composition_cancel(&self) {}

    fn ime_caret(&self) -> Option<vello::kurbo::Rect> {
        None
    }
}

impl HydrolysisRenderer {
    pub(crate) fn register_browser_input_target(
        &mut self,
        local_bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        handler: Rc<dyn BrowserInputHandler>,
    ) {
        self.register_embedded_input_target(
            local_bounds,
            transform,
            Rc::new(BrowserInputSink { handler }),
        );
    }
}
