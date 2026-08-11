use std::cell::{Cell, RefCell};
use std::rc::Rc;

use waterui_browser_cef::{
    CefInputModifiers, CefKeyInput, CefPageHandle, CefPointerButton, CefViewport, gpu_view,
};
use waterui_core::Environment;
use waterui_graphics::gpu_surface::GpuSurface;

use crate::platform::{KeyCode, Modifiers, NativeKey, PointerButton};
use crate::renderer::{
    BrowserInputHandler, EmbeddedGpuSurfaceRuntime, GpuSurfaceSource, HydrolysisRenderer,
    transformed_rect,
};

const CEF_WHEEL_DELTA: f64 = 120.0;

struct CefInputHandler {
    page: CefPageHandle,
    modifiers: Cell<Modifiers>,
    buttons: Cell<(bool, bool, bool)>,
    wheel_remainder: Cell<(f64, f64)>,
    pending_text: RefCell<Option<String>>,
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacEditShortcut {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
}

#[cfg(target_os = "macos")]
impl MacEditShortcut {
    fn from_input(key: &KeyCode, modifiers: CefInputModifiers) -> Option<Self> {
        if !modifiers.command || modifiers.control || modifiers.alt {
            return None;
        }
        let KeyCode::Character(value) = key else {
            return None;
        };
        let character = single_cef_character(value)?.to_ascii_lowercase();
        Some(match (character, modifiers.shift) {
            ('z', false) => Self::Undo,
            ('z', true) => Self::Redo,
            ('x', false) => Self::Cut,
            ('c', false) => Self::Copy,
            ('v', false) => Self::Paste,
            ('a', false) => Self::SelectAll,
            _ => return None,
        })
    }

    fn execute(self, page: &CefPageHandle) {
        match self {
            Self::Undo => page.undo(),
            Self::Redo => page.redo(),
            Self::Cut => page.cut(),
            Self::Copy => page.copy(),
            Self::Paste => page.paste(),
            Self::SelectAll => page.select_all(),
        }
    }
}

impl CefInputHandler {
    fn new(page: CefPageHandle) -> Self {
        Self {
            page,
            modifiers: Cell::new(Modifiers::default()),
            buttons: Cell::new((false, false, false)),
            wheel_remainder: Cell::new((0.0, 0.0)),
            pending_text: RefCell::new(None),
        }
    }

    fn input_modifiers(&self) -> CefInputModifiers {
        let modifiers = self.modifiers.get();
        let (primary_button, middle_button, secondary_button) = self.buttons.get();
        CefInputModifiers {
            shift: modifiers.shift,
            control: modifiers.control,
            alt: modifiers.alt,
            command: modifiers.super_key,
            primary_button,
            middle_button,
            secondary_button,
        }
    }

    fn pointer_button(button: PointerButton) -> Option<CefPointerButton> {
        match button {
            PointerButton::Primary => Some(CefPointerButton::Primary),
            PointerButton::Middle => Some(CefPointerButton::Middle),
            PointerButton::Secondary => Some(CefPointerButton::Secondary),
            PointerButton::Back | PointerButton::Forward => None,
            PointerButton::Other(value) => {
                panic!("CEF does not support pointer button {value}")
            }
        }
    }

    fn update_button(&self, button: CefPointerButton, pressed: bool) {
        let (mut primary, mut middle, mut secondary) = self.buttons.get();
        match button {
            CefPointerButton::Primary => primary = pressed,
            CefPointerButton::Middle => middle = pressed,
            CefPointerButton::Secondary => secondary = pressed,
        }
        self.buttons.set((primary, middle, secondary));
    }

    fn key_input(key: &KeyCode, native: Option<NativeKey>) -> CefKeyInput {
        let keyval = native
            .map(|native| native.keyval)
            .filter(|keyval| *keyval != 0)
            .unwrap_or_else(|| portable_keyval(key));
        CefKeyInput {
            native_keycode: native.map_or(0, |native| native.keycode),
            keyval,
            character: key_character(key),
        }
    }
}

impl BrowserInputHandler for CefInputHandler {
    fn set_focus(&self, focused: bool) {
        self.page.set_focus(focused);
    }

    fn set_modifiers(&self, modifiers: Modifiers) {
        self.modifiers.set(modifiers);
    }

    fn pointer_move(&self, position: vello::kurbo::Point) {
        self.page
            .pointer_move(position.x, position.y, self.input_modifiers());
    }

    fn pointer_button(&self, pressed: bool, button: PointerButton, position: vello::kurbo::Point) {
        let Some(button) = Self::pointer_button(button) else {
            if pressed {
                match button {
                    PointerButton::Back => self.page.go_back(),
                    PointerButton::Forward => self.page.go_forward(),
                    _ => unreachable!("only navigation buttons omit a CEF pointer button"),
                }
            }
            return;
        };
        self.update_button(button, pressed);
        self.page.pointer_button(
            pressed,
            button,
            position.x,
            position.y,
            self.input_modifiers(),
        );
    }

    fn scroll(
        &self,
        position: vello::kurbo::Point,
        delta_x: f32,
        delta_y: f32,
        is_line_delta: bool,
    ) {
        let multiplier = if is_line_delta { CEF_WHEEL_DELTA } else { 1.0 };
        let remainder = self.wheel_remainder.get();
        let delta_x = f64::from(delta_x).mul_add(multiplier, remainder.0);
        let delta_y = f64::from(delta_y).mul_add(multiplier, remainder.1);
        let integral_x = delta_x.round();
        let integral_y = delta_y.round();
        self.wheel_remainder
            .set((delta_x - integral_x, delta_y - integral_y));
        self.page.scroll(
            position.x,
            position.y,
            integral_x,
            integral_y,
            self.input_modifiers(),
        );
    }

    fn key(&self, pressed: bool, key: &KeyCode, native: Option<NativeKey>) {
        let text = pressed
            .then(|| self.pending_text.borrow_mut().take())
            .flatten();
        let character = text.as_deref().and_then(single_cef_character);
        let input = with_text_character(Self::key_input(key, native), character);
        let modifiers = self.input_modifiers();
        self.page.key(pressed, input, modifiers);
        #[cfg(target_os = "macos")]
        if pressed && let Some(command) = MacEditShortcut::from_input(key, modifiers) {
            command.execute(&self.page);
        }
        if character.is_none()
            && let Some(text) = text
        {
            self.page.commit_text(&text, None);
        }
    }

    fn text_input(&self, text: &str) {
        let previous = self.pending_text.borrow_mut().replace(text.to_owned());
        assert!(
            previous.is_none(),
            "CEF received consecutive text input without the corresponding key event"
        );
    }

    fn commit_text(&self, text: &str) {
        self.page.commit_text(text, None);
    }
}

fn single_cef_character(text: &str) -> Option<char> {
    let mut characters = text.chars();
    let character = characters.next()?;
    (characters.next().is_none() && character.len_utf16() == 1).then_some(character)
}

fn with_text_character(mut input: CefKeyInput, character: Option<char>) -> CefKeyInput {
    if let Some(character) = character {
        input.character = Some(character);
    }
    input
}

fn key_character(key: &KeyCode) -> Option<char> {
    match key {
        KeyCode::Character(value) => single_cef_character(value),
        KeyCode::Named(value) => match value.as_str() {
            "Backspace" => Some('\u{7f}'),
            "Tab" => Some('\t'),
            "Enter" => Some('\r'),
            "Escape" => Some('\u{1b}'),
            "Space" => Some(' '),
            _ => None,
        },
        KeyCode::Unidentified => None,
    }
}

fn portable_keyval(key: &KeyCode) -> u32 {
    match key {
        KeyCode::Character(value) => value
            .chars()
            .next()
            .map_or(0, |character| character.to_ascii_uppercase().into()),
        KeyCode::Named(value) => match value.as_str() {
            "Backspace" => 0x08,
            "Tab" => 0x09,
            "Enter" => 0x0d,
            "Shift" => 0x10,
            "Control" => 0x11,
            "Alt" => 0x12,
            "Escape" => 0x1b,
            "Space" => 0x20,
            "PageUp" => 0x21,
            "PageDown" => 0x22,
            "End" => 0x23,
            "Home" => 0x24,
            "ArrowLeft" => 0x25,
            "ArrowUp" => 0x26,
            "ArrowRight" => 0x27,
            "ArrowDown" => 0x28,
            "Insert" => 0x2d,
            "Delete" => 0x2e,
            "F1" => 0x70,
            "F2" => 0x71,
            "F3" => 0x72,
            "F4" => 0x73,
            "F5" => 0x74,
            "F6" => 0x75,
            "F7" => 0x76,
            "F8" => 0x77,
            "F9" => 0x78,
            "F10" => 0x79,
            "F11" => 0x7a,
            "F12" => 0x7b,
            _ => 0,
        },
        KeyCode::Unidentified => 0,
    }
}

pub(crate) struct CefSurfaceRenderState {
    gpu: Rc<RefCell<EmbeddedGpuSurfaceRuntime>>,
    viewport: CefViewport,
    input: Rc<CefInputHandler>,
}

#[cfg(not(test))]
pub(crate) fn install_runtime(env: &mut Environment) {
    let runtime = env
        .get::<waterui_browser_cef::CefRuntime>()
        .cloned()
        .unwrap_or_else(|| {
            let runtime = waterui_browser_cef::CefRuntime::initialize(
                waterui_browser_cef::CefRuntimeConfiguration::packaged(),
            );
            env.insert(runtime.clone());
            runtime
        });
    #[cfg(hydrolysis_cef_webview)]
    env.insert(runtime.webview_controller());
    #[cfg(feature = "chromium")]
    env.insert(runtime.chromium_controller());
}

impl CefSurfaceRenderState {
    pub(crate) fn new(page: CefPageHandle, env: &Environment) -> Self {
        let viewport = CefViewport::new();
        let surface = GpuSurface::new(gpu_view(page.clone(), viewport.clone()));
        Self {
            gpu: Rc::new(RefCell::new(EmbeddedGpuSurfaceRuntime::new(surface, env))),
            viewport,
            input: Rc::new(CefInputHandler::new(page)),
        }
    }

    pub(crate) fn prebuild(&self, renderer: &mut HydrolysisRenderer) {
        renderer.register_node_gpu_surface(Rc::clone(&self.gpu));
    }

    pub(crate) fn render(
        &self,
        renderer: &mut HydrolysisRenderer,
        bounds: vello::kurbo::Rect,
        transform: vello::kurbo::Affine,
        hit_transform: vello::kurbo::Affine,
    ) {
        self.viewport
            .set_scale(transform.determinant().abs().sqrt());
        let input: Rc<dyn BrowserInputHandler> = self.input.clone();
        renderer.register_browser_input_target(bounds, hit_transform, input);
        renderer.push_gpu_surface_layer(
            GpuSurfaceSource::Owned(Rc::clone(&self.gpu)),
            transform,
            bounds,
            transformed_rect(hit_transform, bounds),
        );
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::MacEditShortcut;
    use super::{CefInputHandler, key_character, single_cef_character, with_text_character};
    use crate::platform::{KeyCode, NativeKey};
    use waterui_browser_cef::{CefInputModifiers, CefKeyInput};

    #[test]
    fn native_hardware_code_keeps_the_portable_virtual_key() {
        let input = CefInputHandler::key_input(
            &KeyCode::Character("a".into()),
            Some(NativeKey {
                keycode: 0,
                keyval: 0,
            }),
        );

        assert_eq!(input.native_keycode, 0);
        assert_eq!(input.keyval, u32::from('A'));
        assert_eq!(input.character, Some('a'));
    }

    #[test]
    fn only_one_bmp_character_uses_the_key_character_path() {
        assert_eq!(single_cef_character("W"), Some('W'));
        assert_eq!(single_cef_character(""), None);
        assert_eq!(single_cef_character("UI"), None);
        assert_eq!(single_cef_character("🚀"), None);
    }

    #[test]
    fn macos_editing_keys_preserve_their_character_payloads() {
        assert_eq!(key_character(&KeyCode::Character("a".into())), Some('a'));
        assert_eq!(
            key_character(&KeyCode::Named(String::from("Backspace"))),
            Some('\u{7f}')
        );
        assert_eq!(
            key_character(&KeyCode::Named(String::from("ArrowLeft"))),
            None
        );
    }

    #[test]
    fn absent_text_does_not_erase_shortcut_or_editing_characters() {
        let command = with_text_character(
            CefKeyInput {
                native_keycode: 0,
                keyval: u32::from('A'),
                character: Some('a'),
            },
            None,
        );
        let text = with_text_character(
            CefKeyInput {
                native_keycode: 0,
                keyval: 0,
                character: None,
            },
            Some('W'),
        );

        assert_eq!(command.character, Some('a'));
        assert_eq!(text.character, Some('W'));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_standard_edit_shortcuts_map_to_cef_frame_commands() {
        let command = CefInputModifiers {
            command: true,
            ..Default::default()
        };
        let shifted_command = CefInputModifiers {
            shift: true,
            ..command
        };

        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("a".into()), command),
            Some(MacEditShortcut::SelectAll)
        );
        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("c".into()), command),
            Some(MacEditShortcut::Copy)
        );
        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("x".into()), command),
            Some(MacEditShortcut::Cut)
        );
        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("v".into()), command),
            Some(MacEditShortcut::Paste)
        );
        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("z".into()), command),
            Some(MacEditShortcut::Undo)
        );
        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("Z".into()), shifted_command),
            Some(MacEditShortcut::Redo)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_edit_shortcuts_reject_nonstandard_modifier_combinations() {
        let command_control = CefInputModifiers {
            command: true,
            control: true,
            ..Default::default()
        };
        let shifted_command = CefInputModifiers {
            command: true,
            shift: true,
            ..Default::default()
        };

        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("a".into()), command_control),
            None
        );
        assert_eq!(
            MacEditShortcut::from_input(&KeyCode::Character("a".into()), shifted_command),
            None
        );
        assert_eq!(
            MacEditShortcut::from_input(
                &KeyCode::Character("a".into()),
                CefInputModifiers::default()
            ),
            None
        );
    }
}
