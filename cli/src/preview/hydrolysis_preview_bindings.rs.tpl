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

pub(crate) fn install_preview_theme(env: &mut waterui::env::Environment) {
    {{ preview_theme_installer }}(env);
}
{% if include_automation %}

pub(crate) fn run_semantic_automation(app: &mut waterui_testing::SemanticApp) {
    use waterui::prelude::*;
    use waterui_testing::*;

    {{ semantic_automation_body }}
}
{% endif %}
