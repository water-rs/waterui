//! Generated Hydrolysis preview binding for `{{ crate_name_ident }}`.

{% if expression_mode %}
pub(crate) fn load_preview_view() -> waterui::AnyView {
    use {{ crate_name_ident }}::*;
    use waterui::prelude::*;
    use waterui::prelude::picker::picker;
    use waterui as waterui;
    use waterui_core::binding;

    let view = { {{ preview_expression }} };
    waterui::AnyView::new(view)
}
{% else %}
fn ensure_preview_crate_is_linked() {
    let _ = {{ crate_name_ident }}::app as fn(waterui::env::Environment) -> waterui::app::App;
}

unsafe extern "C" {
    #[link_name = "{{ preview_symbol }}"]
    fn waterui_hydrolysis_preview_entry() -> *mut ();
}

pub(crate) fn load_preview_view() -> waterui::AnyView {
    ensure_preview_crate_is_linked();
    let ptr = unsafe { waterui_hydrolysis_preview_entry() };
    let boxed: Box<waterui::AnyView> = unsafe { Box::from_raw(ptr.cast()) };
    *boxed
}
{% endif %}

/// The style the preview runtimes are constructed with — the value `main`
/// hands to `hydrolysis::run(app, style)`.
pub(crate) fn preview_style() -> impl hydrolysis::Style {
    {{ preview_theme_style }}
}

/// The environment previews resolve under.
///
/// This goes through the application's own composition root — `app(env)` is
/// what installs the realizations and options the application configures
/// (`waterui_map_gpu::install`, provider options, fonts) — so a preview sees
/// the same environment a run would.
pub(crate) fn app_environment() -> waterui::env::Environment {
    let env = waterui::configure_environment!(waterui::env::Environment::new());
    {{ crate_name_ident }}::app(env).env
}
{% if include_automation %}

pub(crate) fn run_semantic_automation(app: &mut waterui_testing::SemanticApp) {
    use waterui::prelude::*;
    use waterui_testing::*;

    {{ semantic_automation_body }}
}
{% endif %}
