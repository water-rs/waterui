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
// record. `CARGO_MANIFEST_DIR` is the `waterui` package root (`src/`).
waterui::include_bundle!("tests/fixtures/web", as = web);

#[test]
fn include_bundle_expands_mount_module() {
    let _: waterui::Bundle = web::BUNDLE;
    let _: waterui::DataAsset = web::hello();
}

/// The mount static is a metadata-directory record in the `.wmeta` section:
/// reading `waterui_meta_bundle_web` back from this linked test binary
/// decodes the mount exactly the way the CLI reads it — matching the name,
/// cutting the payload.
#[test]
fn bundle_record_survives_linking() {
    use object::{Object as _, ObjectSection as _};
    use waterui_assets_planner::BundleMountMeta;
    use waterui_meta::{DIR_SECTION, dir_records};

    // The mount static carries no `#[used]` — a shipped binary dead-strips
    // it. Reading it back here takes the reference a consumer's expansion
    // would leave in the image anyway.
    let emitted = &web::waterui_meta_bundle_web;
    let record = dir_records(emitted)
        .next()
        .expect("the static holds one record");
    assert_eq!(record.name, b"waterui_meta_bundle_web");

    let exe = std::env::current_exe().expect("current_exe resolves");
    let bytes = std::fs::read(exe).expect("read the test binary");
    let file = object::File::parse(&*bytes).expect("parse the test binary");
    let mut found = None;
    for section in file.sections() {
        let Ok(name) = section.name() else {
            continue;
        };
        if !matches!(name, DIR_SECTION | "__wmeta") {
            continue;
        }
        let Ok(data) = section.data() else {
            continue;
        };
        for record in dir_records(data) {
            if record.name == b"waterui_meta_bundle_web" {
                assert!(found.is_none(), "duplicate bundle record");
                found = Some(record);
            }
        }
    }
    let record = found.expect("no `waterui_meta_bundle_web` record in the section");
    let meta = BundleMountMeta::from_record(record).expect("decode the record");
    assert_eq!(meta.mount, "web");
    assert!(meta.path.ends_with("tests/fixtures/web"), "{:?}", meta.path);
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
