//! Generated Hydrolysis preview test binding for `{{ crate_name_ident }}`.

pub(crate) const PREVIEW_THEME: &str = "{{ preview_theme }}";
pub(crate) const PREVIEW_TEST_MODE: &str = "{{ test_mode }}";
pub(crate) const PREVIEW_WIDTH_ENV: &str = "{{ preview_width_env }}";
pub(crate) const PREVIEW_HEIGHT_ENV: &str = "{{ preview_height_env }}";
pub(crate) const PERF_WARMUPS_ENV: &str = "{{ perf_warmups_env }}";
pub(crate) const PERF_SAMPLES_ENV: &str = "{{ perf_samples_env }}";
pub(crate) const FLAMEGRAPH_ENV: &str = "{{ flamegraph_env }}";
pub(crate) const FLAMEGRAPH_FREQUENCY_ENV: &str = "{{ flamegraph_frequency_env }}";

{% if expression_mode %}
pub(crate) fn load_preview_view() -> waterui::AnyView {
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

pub(crate) fn run_semantic_automation(app: &mut waterui_testing::SemanticApp) {
    use waterui::prelude::*;
    use waterui_testing::*;

    {{ semantic_automation_body }}
}

pub(crate) fn run_perf_automation<T, F, V>(perf: &mut waterui_testing::PerfApp<T, F, V>)
where
    T: waterui_testing::ThemeInstaller,
    F: Fn() -> V + Clone + 'static,
    V: waterui::View + 'static,
{
    use waterui::prelude::*;
    use waterui_testing::*;

    {{ perf_automation_body }}
}
