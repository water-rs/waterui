//! The two hosts that mount a `tsx!` view outside an application — a
//! `waterui-testing` session and the `water preview` support app — with the
//! bundle `WATERUI_TS_BUNDLE` names.
//!
//! Every test here configures the variable first, through `support/bundle.rs`,
//! which does for this binary what `water test` does for a crate. The targets
//! beside this one — `bundle_var_unset.rs`, `bundle_var_missing.rs`,
//! `bundle_var_relative.rs` — each hold one process environment the variable
//! can be in, because a process has one environment and nextest gives each
//! test its own process.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nami::Binding;
use waterui::app::App;
use waterui::tsx;
use waterui_core::{AnyView, Environment};
use waterui_preview::with_configured_runtime;
use waterui_testing::{UiBuilder, install_default_theme, ui};
use waterui_ts::schema::TsProps;

#[path = "support/bundle.rs"]
mod bundle;

/// The props `fixtures/promo.tsx` is typed against.
#[derive(TsProps)]
struct PromoProps {
    headline: String,
    unread: Binding<u32>,
    #[ts(rename = "onDismiss")]
    on_dismiss: Box<dyn Fn()>,
}

/// The headline every test mounts the module with.
const HEADLINE: &str = "Welcome back";

/// The module, mounted the way an application's view mounts it, with
/// `dismissed` set when its button is tapped.
fn promo(unread: Binding<u32>, dismissed: &Rc<Cell<bool>>) -> impl waterui_core::View + use<> {
    let dismissed = Rc::clone(dismissed);
    tsx!(
        "fixtures/promo.tsx",
        PromoProps {
            headline: String::from(HEADLINE),
            unread,
            on_dismiss: Box::new(move || dismissed.set(true)),
        }
    )
}

#[waterui::test(viewport = (320, 240))]
fn ts_a_test_session_mounts_a_tsx_view(ui: UiBuilder) {
    // `ui.mount` installs the runtime loaded from the bundle the variable
    // names, so the view's `Mount` finds one. Without that installation the
    // mount panics with the missing-runtime message — which is what
    // `bundle_var_unset.rs` asserts — so a tree with the module's labels in
    // it is a runtime the session installed, not one the test did.
    bundle::configure(PromoProps::CONTRACT_HASH);
    let unread = Binding::container(3_u32);
    let dismissed = Rc::new(Cell::new(false));
    let mut app = {
        let unread = unread.clone();
        let dismissed = Rc::clone(&dismissed);
        ui.mount(move || promo(unread.clone(), &dismissed))
    };

    app.query().label(HEADLINE).assert_exists();
    app.query().label("3 unread").assert_exists();

    // The mount's exports outlive the mount: the binding keeps pushing into
    // the module's signal for as long as the session holds the view.
    unread.set(11);
    app.settle();
    app.query().label("11 unread").assert_exists();
    app.query().label("3 unread").assert_not_exists();

    app.query().label("Dismiss").tap();
    assert!(
        dismissed.get(),
        "tapping the module's button ran the callback prop"
    );
}

#[test]
fn ts_a_preview_render_installs_the_runtime_for_the_view() {
    // The preview support app has a view it loaded from a dylib and the
    // environment its backend gave it, and nothing else: the runtime rides in
    // the view's own subtree environment. The view is mounted through
    // `mount_app`, which runs the application's environment verbatim and
    // installs nothing of its own — `mount` would install the runtime the
    // variable names into the session and hide an unwrapped view — so the
    // labels in the tree come from the runtime the subtree carries and from
    // nothing else.
    bundle::configure(PromoProps::CONTRACT_HASH);
    let dismissed = Rc::new(Cell::new(false));
    // The support app's environment carries its backend's theme, which the
    // module reads its colours from; the session's default theme stands in.
    let mut environment = Environment::new();
    install_default_theme(&mut environment);
    let view = with_configured_runtime(
        &environment,
        AnyView::new(promo(Binding::container(3), &dismissed)),
    )
    .unwrap_or_else(|error| panic!("installing the runtime for the preview: {error}"));

    // A view crosses once, so the builder hands it over exactly once.
    let view = RefCell::new(Some(view));
    let app = App::new(
        move || {
            view.borrow_mut()
                .take()
                .expect("the application realizes its root once")
        },
        environment,
    );
    let mut app = ui().viewport(320, 240).mount_app(app);

    app.query().label(HEADLINE).assert_exists();
    app.query().label("3 unread").assert_exists();
    app.query().label("Dismiss").tap();
    assert!(dismissed.get(), "the preview's view is live, not a picture");
}
