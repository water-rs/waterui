//! Compile-level coverage for `asset!` expansions.
//!
//! `asset!` generates paths into the `waterui` facade (`media::Photo`,
//! `video::video`, root asset handles). Nothing else in the workspace invokes
//! the macro, so without this test a facade re-export moving out from under an
//! expansion breaks only downstream users. Every `AssetKind` arm is
//! instantiated here; the values are never loaded, so the referenced files do
//! not need to exist.
#![cfg(feature = "assets")]

use waterui::asset;

// `include_bundle!` expands to a `web` mount module whose `BUNDLE` and
// accessors are staged by the CLI from the emitted `waterui_meta_bundle_web`
// static. `CARGO_MANIFEST_DIR` is the `waterui` package root (`src/`).
waterui::include_bundle!("tests/fixtures/web", as = web);

#[test]
fn include_bundle_expands_mount_module() {
    let _: waterui::Bundle = web::BUNDLE;
    let _: waterui::DataAsset = web::hello();
}

#[test]
fn asset_macro_expands_for_every_kind() {
    // Packaged paths are relative to ResourceContext's assets root.
    let _local_image: waterui::ImageAsset = asset!("logo.png");
    let _remote_image = asset!("https://waterui.dev/logo.png");
    let _local_video: waterui::VideoAsset = asset!("intro.mp4");
    let _remote_video = asset!("https://waterui.dev/intro.mp4");
    // Font, audio, data, and large-model handles live at the facade root.
    let _font: waterui::FontAsset = asset!("body.ttf");
    let _audio: waterui::AudioAsset = asset!("chime.mp3");
    let _data: waterui::DataAsset = asset!("config.json");
    let _remote_data = asset!("https://waterui.dev/config.json");
    let _model: waterui::LargeFileAsset = asset!("classifier.onnx");
}
