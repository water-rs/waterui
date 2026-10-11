//! Generated Apple in-process preview entry for `{{ crate_name_ident }}`.

mod preview_target;

fn main() -> Result<(), waterui_apple::preview::PreviewError> {
    let resources = waterui::ResourceContext::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/resources/waterui_assets"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/resources/fonts"),
    );
    let config = waterui_preview_protocol::run::PreviewRunConfig::load_from_env()
        .expect("the CLI writes the run configuration before exec");
    waterui_apple::preview::run(
        |env| {{ crate_name_ident }}::app(waterui::configure_environment!(env)).env,
        preview_target::load_preview_view,
        resources,
        config,
    )
}
