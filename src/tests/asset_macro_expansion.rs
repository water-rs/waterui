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

// `include_bundle!` must parse its own documented syntax; the expansion is
// deliberately inert, so this only asserts the parser accepts `as = <mount>`.
waterui::include_bundle!("assets/web", as = web);

#[test]
fn asset_macro_expands_for_every_kind() {
    // Image: local and remote resolve to `waterui::media::Photo`.
    let _local_image = asset!("assets/logo.png");
    let _remote_image = asset!("https://waterui.dev/logo.png");
    // Video: local paths route through `waterui::Url`.
    let _local_video = asset!("assets/intro.mp4");
    let _remote_video = asset!("https://waterui.dev/intro.mp4");
    // Font, audio, data, and large-model handles live at the facade root.
    let _font = asset!("assets/body.ttf");
    let _audio = asset!("assets/chime.mp3");
    let _data = asset!("assets/config.json");
    let _remote_data = asset!("https://waterui.dev/config.json");
    let _model = asset!("assets/classifier.onnx");
}
