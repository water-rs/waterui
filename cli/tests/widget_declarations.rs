//! `#[widget]` end-to-end: the fixture crate's declarations land as
//! `waterui_widget_*` exports paired with `waterui_meta_widget_*` records,
//! and `apple::widgets::declared_widgets` pairs them back into the
//! declarations the extension build consumes.

#[cfg(target_os = "macos")]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cargo_metadata::Message;
use waterui_cli::apple::widgets::declared_widgets;
use waterui_cli::artifact_symbols::ArtifactSymbols;
use waterui_cli::toolchain::Host;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/widget_app")
}

struct WidgetFixture {
    artifact: PathBuf,
    #[cfg(target_os = "macos")]
    target_dir: PathBuf,
    #[cfg(target_os = "macos")]
    native_links: Vec<OsString>,
}

fn build_fixture() -> WidgetFixture {
    smol::block_on(async {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("the CLI is a workspace member");
        let host = Host::current().with_cwd(workspace);
        let metadata = host
            .output_within(
                env!("CARGO"),
                ["metadata", "--no-deps", "--format-version=1", "--offline"],
                Duration::from_secs(30),
            )
            .await
            .expect("read the workspace target directory");
        assert!(metadata.status.success(), "{metadata:?}");
        let metadata: cargo_metadata::Metadata =
            serde_json::from_slice(&metadata.stdout).expect("Cargo metadata decodes");
        let target_dir = metadata.target_directory.into_std_path_buf();
        let output = host
            .with_cwd(fixture_dir())
            .with_env("CARGO_TARGET_DIR", &target_dir)
            .output_within(
                env!("CARGO"),
                [
                    "rustc",
                    "--lib",
                    "--offline",
                    "--locked",
                    "--message-format=json-render-diagnostics",
                    "--",
                    "--print=native-static-libs",
                ],
                Duration::from_secs(600),
            )
            .await
            .expect("build the widget fixture");
        assert!(output.status.success(), "{output:?}");
        let mut artifact = None;
        #[cfg(target_os = "macos")]
        let mut native_links = None;
        for message in Message::parse_stream(output.stdout.as_slice()) {
            match message.expect("Cargo emits valid messages") {
                Message::CompilerArtifact(message) if message.target.name == "widget_app" => {
                    artifact = message
                        .filenames
                        .into_iter()
                        .find(|path| matches!(path.extension(), Some("a" | "lib")))
                        .map(cargo_metadata::camino::Utf8PathBuf::into_std_path_buf);
                }
                #[cfg(target_os = "macos")]
                Message::CompilerMessage(message) => {
                    if let Some(flags) =
                        message.message.message.strip_prefix("native-static-libs: ")
                    {
                        native_links = Some(flags.split_whitespace().map(OsString::from).collect());
                    }
                }
                _ => {}
            }
        }
        WidgetFixture {
            artifact: artifact.expect("Cargo reports the fixture's static library"),
            #[cfg(target_os = "macos")]
            target_dir,
            #[cfg(target_os = "macos")]
            native_links: native_links
                .or_else(|| {
                    String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .find_map(|line| line.split_once("native-static-libs: "))
                        .map(|(_, flags)| flags.split_whitespace().map(OsString::from).collect())
                })
                .expect("rustc reports the fixture's native link requirements"),
        }
    })
}

#[test]
#[ignore = "builds framework source; runs in the nightly cli-ignored job"]
fn widget_declarations_pair_exports_with_records() {
    let fixture = build_fixture();
    let symbols = ArtifactSymbols::read(&fixture.artifact).expect("read the fixture artifact");

    let widgets = declared_widgets(&symbols).expect("pair declarations with exports");
    assert_eq!(widgets.len(), 2);

    // Sorted by `kind`: `now-kind` before `steps`.
    let now = &widgets[0];
    assert_eq!(now.function, "now");
    assert_eq!(now.kind, "now-kind");
    assert_eq!(now.display_name, "Now");
    assert_eq!(now.description, "The current time");
    assert_eq!(now.families, ["systemSmall"]);
    assert_eq!(now.symbol, "waterui_widget_widget_app_now");

    let steps = &widgets[1];
    assert_eq!(steps.function, "steps");
    assert_eq!(steps.kind, "steps");
    assert_eq!(steps.display_name, "Steps");
    assert_eq!(steps.description, "Today's step count");
    assert_eq!(
        steps.families,
        ["systemSmall", "systemMedium", "systemLarge"]
    );
    assert_eq!(steps.symbol, "waterui_widget_widget_app_steps");

    // The `waterui-widgets` static record the extension build locates the
    // shipped Swift sources through rides the same artifact.
    let sources = symbols
        .meta_record("waterui_meta_widgets_swift_sources")
        .expect("the swift sources record is in the artifact");
    let sources = String::from_utf8(sources).expect("the record is UTF-8");
    assert!(
        Path::new(&sources).join("FFI.swift").is_file(),
        "the record points at the crate's Swift sources: {sources}"
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "builds framework source and uses the Swift SDK; runs in nightly cli-ignored"]
fn widget_snapshot_round_trips_through_swift() {
    let fixture = build_fixture();
    smol::block_on(async {
        let output = tempfile::tempdir_in(&fixture.target_dir).expect("probe build directory");
        let main = output.path().join("main.swift");
        smol::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/widget_ffi_probe/probe.swift"),
            &main,
        )
        .await
        .expect("copy the probe verbatim as the Swift entry point");
        let symbols = ArtifactSymbols::read(&fixture.artifact).expect("read the fixture artifact");
        let sources = PathBuf::from(
            String::from_utf8(
                symbols
                    .meta_record("waterui_meta_widgets_swift_sources")
                    .expect("the widget sources are declared"),
            )
            .expect("the sources path is UTF-8"),
        );
        let executable = output.path().join("widget-ffi-probe");
        let mut args: Vec<OsString> = [
            "--sdk",
            "macosx",
            "swiftc",
            "-enable-experimental-feature",
            "Extern",
            "-o",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        args.extend([
            executable.clone().into_os_string(),
            sources.join("FFI.swift").into_os_string(),
            sources.join("Schema.swift").into_os_string(),
            main.into_os_string(),
            fixture.artifact.into_os_string(),
        ]);
        args.extend(fixture.native_links);
        let host = Host::current();
        let compile = host
            .output_within("xcrun", args, Duration::from_secs(120))
            .await
            .expect("compile the shipped Swift bridge against the Rust exports");
        assert!(compile.status.success(), "{compile:?}");
        let run = host
            .output_within(
                &executable,
                std::iter::empty::<&str>(),
                Duration::from_secs(30),
            )
            .await
            .expect("execute the snapshot probe");
        assert!(run.status.success(), "{run:?}");
        assert!(
            String::from_utf8_lossy(&run.stdout).contains("FFI probe passed"),
            "{run:?}"
        );
    });
}
