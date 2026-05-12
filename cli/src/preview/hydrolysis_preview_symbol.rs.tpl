pub(crate) const PREVIEW_OUTPUT_ENV: &str = "{{ preview_output_env }}";
pub(crate) const PREVIEW_WIDTH_ENV: &str = "{{ preview_width_env }}";
pub(crate) const PREVIEW_HEIGHT_ENV: &str = "{{ preview_height_env }}";
pub(crate) const PREVIEW_THEME: &str = "{{ preview_theme }}";

{% if expression_mode %}
pub(crate) fn load_preview_view() -> waterui::AnyView {
    use waterui::prelude::*;
    use waterui as waterui;

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
