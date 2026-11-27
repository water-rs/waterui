//! Generated Apple in-process preview entry for `{{ crate_name_ident }}`.

#[cfg(feature = "dev")]
use waterui_dylib as _;

mod preview_target;

// Embeds the preview `Info.plist` on iOS Simulator (`UIDeviceFamily` 1 so
// `UIKit` runs under the iPhone idiom). Expands to nothing on macOS.
waterui_apple::preview_info_plist!();

fn main() -> Result<(), waterui_apple::preview::PreviewError> {
    let resources = waterui::ResourceContext::new(
        concat!(env!("CARGO_MANIFEST_DIR"), "/resources/waterui_assets"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/resources/fonts"),
    );
    let config = waterui_preview_protocol::run::PreviewRunConfig::load_from_env()
        .expect("the CLI writes the run configuration before exec");
    waterui_apple::preview::run(
        |env| {{ crate_name_ident }}::app(waterui::configure_environment!(env)).into_parts().env,
        preview_target::load_preview_view,
        resources,
        config,
    )
}
