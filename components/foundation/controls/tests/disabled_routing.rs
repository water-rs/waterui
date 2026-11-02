//! Pointer routing around disabled controls: a pointer event that lands on a
//! disabled control's bounds dispatches into the window but reaches no
//! action. Moved from `backends/hydrolysis/tests/controls.rs` under
//! water-rs/waterui#1673; the assertions are style-free (a binding either
//! moved or it did not), so the mount runs under `TestStyle` — the minimal
//! style — instead of the Material 3 theme the origin used.

mod support;

use support::control_shell;
use waterui::Signal as _;
use waterui::{Binding, ViewExt as _};
use waterui_controls::{ToggleStyle, button, slider::slider, toggle};
use waterui_testing::{Role, Styled, TestStyle, UiBuilder};

/// The role a default-style toggle reports on this host — the platform's
/// default style decides it, so queries for one hold on every host.
const DEFAULT_TOGGLE: Role = Role::toggle(ToggleStyle::Automatic);

/// The pointer event dispatches into the window but must not hit the
/// disabled control: the binding stays unchanged.
#[waterui::test(theme = TestStyle::new(), viewport = (320, 240))]
fn disabled_toggle_ignores_pointer_input(ui: UiBuilder<Styled<TestStyle>>) {
    let enabled = Binding::bool(false);
    let enabled_for_view = enabled.clone();

    let mut app = ui
        .mount_offscreen(move || control_shell(toggle("Wi-Fi", &enabled_for_view).disabled(true)));

    app.query()
        .role(DEFAULT_TOGGLE)
        .label("Wi-Fi")
        .tap_at(0.5, 0.5);
    assert!(
        !enabled.snapshot(),
        "disabled-toggle: binding must stay unchanged"
    );
}

/// The pointer drag dispatches into the window but must not hit the
/// disabled control: the value stays unchanged.
#[waterui::test(theme = TestStyle::new(), viewport = (320, 240))]
fn disabled_slider_ignores_pointer_drags(ui: UiBuilder<Styled<TestStyle>>) {
    let value = Binding::f64(0.5);
    let value_for_view = value.clone();

    let mut app =
        ui.mount_offscreen(move || control_shell(slider("Volume", &value_for_view).disabled(true)));

    app.query()
        .role(Role::SLIDER)
        .label("Volume")
        .drag_by(60.0, 0.0);
    assert!(
        (value.snapshot() - 0.5).abs() <= 0.0001,
        "disabled-slider: value must stay unchanged, got {}",
        value.snapshot()
    );
}

/// The pointer event dispatches into the window but must not hit the
/// disabled control: the action never runs.
#[waterui::test(theme = TestStyle::new(), viewport = (320, 240))]
fn disabled_button_ignores_pointer_input(ui: UiBuilder<Styled<TestStyle>>) {
    let count = Binding::i32(0);
    let count_for_view = count.clone();

    let mut app = ui.mount_offscreen(move || {
        control_shell(
            button("Submit")
                .action(|waterui::State(count): waterui::State<Binding<i32>>| {
                    *count.get_mut() += 1;
                })
                .disabled(true)
                .state(&count_for_view),
        )
    });

    app.query()
        .role(Role::BUTTON)
        .label("Submit")
        .tap_at(0.5, 0.5);
    assert_eq!(count.snapshot(), 0, "disabled-button: action must not run");
}
