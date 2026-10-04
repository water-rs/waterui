//! The runners' theme-token assembly must order framework defaults <
//! `Style::install_tokens` < the application's own environment
//! (water-rs/hydrolysis#203): a `Theme` the application installs is never
//! replaced by the runtime, a style installing its tokens sees the
//! application's colour scheme, and a reactive scheme keeps the resolved
//! tokens live.

use std::cell::RefCell;
use std::rc::Rc;

use hydrolysis::{FontFamilyResolution, HeadlessRuntime, SemanticRuntime};
use hydrolysis_m3::{Material3, MaterialColorScheme};
use waterui::app::App;
use waterui::color::WorkingColor;
use waterui::graphics::color::Srgb;
use waterui::reactive::binding;
use waterui::theme::color::{Background, Foreground};
use waterui::theme::{ColorScheme, Theme, installed_color_scheme, installed_color_signal};
use waterui::{Computed, Signal};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment, View};

/// What a [`TokenProbe`] read out of the environment it was built under.
#[derive(Default)]
struct CapturedTokens {
    scheme: Option<Computed<ColorScheme>>,
    background: Option<Computed<WorkingColor>>,
    foreground: Option<Computed<WorkingColor>>,
}

/// Records the token signals its environment resolves at build time — a
/// probe into the token layer a runner assembled.
#[derive(Clone)]
struct TokenProbe(Rc<RefCell<CapturedTokens>>);

impl View for TokenProbe {
    fn body(self, env: &Environment) -> impl View {
        let mut captured = self.0.borrow_mut();
        captured.scheme = installed_color_scheme(env);
        captured.background = installed_color_signal::<Background>(env);
        captured.foreground = installed_color_signal::<Foreground>(env);
        AnyView::new(())
    }
}

fn token_probe() -> (TokenProbe, Rc<RefCell<CapturedTokens>>) {
    let captured = Rc::new(RefCell::new(CapturedTokens::default()));
    (TokenProbe(Rc::clone(&captured)), captured)
}

/// The application's environment as every runner receives it —
/// `App::into_parts`. Building the `App` also runs the framework's own
/// composition-root installs, so `env` carries the same entries a real
/// application's environment does.
fn app_environment(env: Environment) -> Environment {
    App::new(|| (), env).into_parts().env
}

/// Mounts a probe under the styled headless runtime — the same environment
/// assembly `run` performs — and builds the tree once.
fn headless_probe(
    env: Environment,
    style: impl hydrolysis::Style,
) -> (HeadlessRuntime, Rc<RefCell<CapturedTokens>>) {
    let (probe, captured) = token_probe();
    let content = AnyViewBuilder::new(move || AnyView::new(probe.clone()));
    let mut runtime = HeadlessRuntime::new_for_tests(env, content, 64, 64, style);
    let _ = runtime.pump(false);
    (runtime, captured)
}

/// Mounts a probe under the semantic runtime — the unstyled assembly the
/// other runners share before a style's tokens land.
fn semantic_probe(env: Environment) -> Rc<RefCell<CapturedTokens>> {
    let (probe, captured) = token_probe();
    let content = AnyViewBuilder::new(move || AnyView::new(probe.clone()));
    let mut runtime = SemanticRuntime::new(env, content, 64, 64, FontFamilyResolution::Strict);
    let _ = runtime.pump();
    captured
}

/// `WorkingColor` does not compare for equality; compare channel bits.
fn assert_resolved_eq(actual: WorkingColor, expected: WorkingColor, message: &str) {
    let bits = |color: WorkingColor| color.components.map(f32::to_bits);
    assert_eq!(bits(actual), bits(expected), "{message}");
}

/// The application's `Theme` outranks both the framework defaults and the
/// style's tokens: `Material3`'s dynamic colours must bind the application's
/// installed scheme, so the resolved tokens are the dark Material baseline.
#[test]
fn application_theme_wins_over_framework_defaults() {
    let mut app_env = Environment::new();
    app_env.install(Theme::new().color_scheme(ColorScheme::Dark));
    let (_runtime, captured) = headless_probe(app_environment(app_env), Material3::defaults());
    let captured = captured.borrow();

    assert_eq!(
        captured.scheme.as_ref().map(Signal::snapshot),
        Some(ColorScheme::Dark),
        "the application's installed scheme must survive environment assembly"
    );
    assert_resolved_eq(
        captured
            .background
            .as_ref()
            .expect("a `Background` token must be installed")
            .snapshot(),
        MaterialColorScheme::baseline_dark().background.working(),
        "`Background` must resolve to the dark Material colour",
    );
}

/// An application that installs no `Theme` resolves the framework defaults:
/// the Light scheme and the framework's own colour baseline.
#[test]
fn bare_application_resolves_framework_defaults() {
    let captured = semantic_probe(app_environment(Environment::new()));
    let captured = captured.borrow();

    assert_eq!(
        captured.scheme.as_ref().map(Signal::snapshot),
        Some(ColorScheme::Light),
        "an application that installs nothing resolves the Light scheme"
    );
    assert_resolved_eq(
        captured
            .background
            .as_ref()
            .expect("a `Background` token must be installed")
            .snapshot(),
        Srgb::from_u32(0xFF_FF_FF).resolve(),
        "`Background` must be the framework default",
    );
    assert_resolved_eq(
        captured
            .foreground
            .as_ref()
            .expect("a `Foreground` token must be installed")
            .snapshot(),
        Srgb::from_u32(0x11_18_27).resolve(),
        "`Foreground` must be the framework default",
    );
}

/// A `Binding`-backed scheme keeps the token layer live: the style's
/// projected tokens re-resolve when the application's binding flips.
#[test]
fn reactive_color_scheme_rethemes_resolved_tokens() {
    let scheme = binding(ColorScheme::Light);
    let mut app_env = Environment::new();
    app_env.install(Theme::new().color_scheme(scheme.clone()));
    let (mut runtime, captured) = headless_probe(app_environment(app_env), Material3::defaults());

    {
        let captured = captured.borrow();
        assert_eq!(
            captured.scheme.as_ref().map(Signal::snapshot),
            Some(ColorScheme::Light),
            "the scheme must start from the application's binding"
        );
        assert_resolved_eq(
            captured
                .background
                .as_ref()
                .expect("a `Background` token must be installed")
                .snapshot(),
            MaterialColorScheme::baseline_light().background.working(),
            "`Background` must start at the light Material colour",
        );
    }

    scheme.set(ColorScheme::Dark);
    let _ = runtime.pump(false);

    let captured = captured.borrow();
    assert_eq!(
        captured.scheme.as_ref().map(Signal::snapshot),
        Some(ColorScheme::Dark),
        "the scheme must follow the application's binding"
    );
    assert_resolved_eq(
        captured
            .background
            .as_ref()
            .expect("a `Background` token must be installed")
            .snapshot(),
        MaterialColorScheme::baseline_dark().background.working(),
        "`Background` must follow the binding to the dark Material colour",
    );
}
