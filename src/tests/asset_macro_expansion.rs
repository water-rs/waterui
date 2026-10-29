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

/// The mount static is a metadata-directory record: on non-Apple targets
/// and Apple debug builds the linked test binary's `__wmeta`/`DIR_SECTION`
/// section carries it, decoded exactly the way the CLI reads it — matching
/// the name, cutting the payload. An Apple release build parks the record
/// in `__DWARF`, which `ld` strips from the final image: there the record
/// is asserted absent, the same way a shipped binary reports.
#[test]
fn bundle_record_obeys_linked_profile() {
    use object::{Object as _, ObjectSection as _};
    use waterui_meta::{DIR_SECTION, dir_records};

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
    present_or_absent(found);
}

/// The non-Apple and Apple-debug profile: the runtime reference keeps the
/// record in the image, and decoding the linked record answers the mount.
#[cfg(any(not(target_vendor = "apple"), debug_assertions))]
fn present_or_absent(record: Option<waterui_meta::DirRecord<'_>>) {
    use waterui_assets_planner::BundleMountMeta;
    use waterui_meta::dir_records;

    let emitted = &web::waterui_meta_bundle_web;
    let static_record = dir_records(emitted)
        .next()
        .expect("the static holds one record");
    assert_eq!(static_record.name, b"waterui_meta_bundle_web");

    let record = record.expect("no `waterui_meta_bundle_web` record in the section");
    let meta = BundleMountMeta::from_record(record).expect("decode the record");
    assert_eq!(meta.mount(), "web");
    assert!(
        meta.path().ends_with("tests/fixtures/web"),
        "{}",
        meta.path().display()
    );
}

/// The Apple release profile: the record is parked in `__DWARF` for `ld`
/// to strip, so the linked image carries none — `web::waterui_meta_bundle_web`
/// is never referenced, because the record data is discarded with the section.
#[cfg(all(target_vendor = "apple", not(debug_assertions)))]
fn present_or_absent(record: Option<waterui_meta::DirRecord<'_>>) {
    assert!(
        record.is_none(),
        "the release linker stripped the __wmeta record"
    );
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
