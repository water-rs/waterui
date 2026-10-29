//! `#[widget]` end-to-end: the fixture crate's declarations land as
//! `waterui_widget_*` exports paired with `waterui_meta_widget_*` records,
//! and `apple::widgets::declared_widgets` pairs them back into the
//! declarations the extension build consumes.

#[cfg(target_os = "macos")]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cargo_metadata::Message;
#[cfg(target_os = "macos")]
use waterui_cli::apple::widgets::{
    WIDGET_MINIMUM_APPLE_VERSION, render_declared_intents_swift, widget_swift_sources,
};
use waterui_cli::apple::widgets::{declared_intents, declared_widgets};
use waterui_cli::artifact_symbols::ArtifactSymbols;
use waterui_cli::toolchain::Host;

/// The staged fixture's `Cargo.toml`.
///
/// The fixture builds the enclosing checkout's framework source, so it must
/// resolve the checkout's dependency graph: the checkout's `[patch]` tables
/// (Cargo reads them only at the root of the workspace it builds) beside a
/// lock seeded from the checkout's own. A standalone manifest takes every
/// split component the checkout pins to git from crates.io instead.
#[derive(serde::Serialize)]
struct FixtureManifest {
    package: FixturePackage,
    workspace: toml::Table,
    lib: FixtureLib,
    dependencies: std::collections::BTreeMap<&'static str, FixtureDependency>,
    patch: toml::Value,
}

#[derive(serde::Serialize)]
struct FixturePackage {
    name: &'static str,
    version: &'static str,
    edition: &'static str,
}

/// The artifact shape Apple packaging reads: every object — the app's own
/// `waterui_widget_*`/`waterui_meta_widget_*` emission and `waterui-widgets`'
/// Swift-sources record — merged into one archive.
#[derive(serde::Serialize)]
struct FixtureLib {
    path: PathBuf,
    #[serde(rename = "crate-type")]
    crate_type: [&'static str; 1],
}

/// `widgets` alone: the declaration path carries no renderer dependencies.
#[derive(serde::Serialize)]
struct FixtureDependency {
    path: PathBuf,
    #[serde(rename = "default-features")]
    default_features: bool,
    features: [&'static str; 1],
}

/// Stage the fixture crate in a fresh directory under `target_dir`: its
/// manifest roots the committed sources at the checkout's patch set, and its
/// lock starts from the checkout's, which Cargo prunes to the fixture's graph.
fn stage_fixture(workspace: &Path, target_dir: &Path) -> tempfile::TempDir {
    let staging = target_dir.join("test-fixtures");
    std::fs::create_dir_all(&staging).expect("the fixture staging directory is creatable");
    let staged = tempfile::tempdir_in(&staging).expect("a staged fixture directory");
    let patches = waterui_cli::framework::checkout_patch_tables(workspace)
        .expect("the checkout's patch set resolves");
    let manifest = FixtureManifest {
        package: FixturePackage {
            name: "widget-app",
            version: "0.0.0",
            edition: "2024",
        },
        workspace: toml::Table::new(),
        lib: FixtureLib {
            path: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/widget_app/src/lib.rs"),
            crate_type: ["staticlib"],
        },
        dependencies: std::iter::once((
            "waterui",
            FixtureDependency {
                path: workspace.to_path_buf(),
                default_features: false,
                features: ["widgets"],
            },
        ))
        .collect(),
        patch: toml::Value::try_from(&patches).expect("the patch set serializes"),
    };
    std::fs::write(
        staged.path().join("Cargo.toml"),
        toml::to_string(&manifest).expect("the fixture manifest serializes"),
    )
    .expect("write the staged fixture manifest");
    std::fs::copy(
        workspace.join("Cargo.lock"),
        staged.path().join("Cargo.lock"),
    )
    .expect("seed the fixture lock from the checkout's");
    staged
}

struct WidgetFixture {
    artifact: PathBuf,
    #[cfg(target_os = "macos")]
    target_dir: PathBuf,
    #[cfg(target_os = "macos")]
    native_links: Vec<OsString>,
}

fn build_fixture(profile: &str) -> WidgetFixture {
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
        let staged = stage_fixture(workspace, &target_dir);
        // Not `--locked`: Cargo prunes the seeded checkout lock to the
        // fixture's graph, which keeps every version the checkout pins.
        let output = host
            .with_cwd(staged.path())
            .with_env("CARGO_TARGET_DIR", &target_dir)
            .output_within(
                env!("CARGO"),
                [
                    "rustc",
                    "--lib",
                    "--profile",
                    profile,
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
    for profile in ["dev", "release"] {
        assert_fixture_declarations(&build_fixture(profile));
    }
}

fn assert_fixture_declarations(fixture: &WidgetFixture) {
    let symbols = ArtifactSymbols::read(&fixture.artifact).expect("read the fixture artifact");

    let widgets = declared_widgets(&symbols).expect("pair declarations with exports");
    assert_eq!(widgets.len(), 3);

    // Sorted by `kind`: `now-kind`, then `record-kind`, then `steps`.
    let now = &widgets[0];
    assert_eq!(now.function(), "now");
    assert_eq!(now.kind(), "now-kind");
    assert_eq!(now.display_name(), "Now");
    assert_eq!(now.description(), "The current time");
    assert_eq!(now.families(), ["systemSmall"]);
    assert_eq!(now.symbol(), "waterui_widget_widget_app_now");

    let record = &widgets[1];
    assert_eq!(record.function(), "record");
    assert_eq!(record.kind(), "record-kind");
    assert_eq!(record.display_name(), "Record");
    assert_eq!(record.description(), "Writes a record");
    assert_eq!(record.families(), ["systemSmall"]);
    assert_eq!(record.symbol(), "waterui_widget_widget_app_record");

    let steps = &widgets[2];
    assert_eq!(steps.function(), "steps");
    assert_eq!(steps.kind(), "steps");
    assert_eq!(steps.display_name(), "Steps");
    assert_eq!(steps.description(), "Today's step count");
    assert_eq!(
        steps.families(),
        ["systemSmall", "systemMedium", "systemLarge"]
    );
    assert_eq!(steps.symbol(), "waterui_widget_widget_app_steps");

    let intents = declared_intents(&symbols).expect("pair intent declarations with exports");
    assert_eq!(intents.len(), 1);
    let intent = &intents[0];
    assert_eq!(intent.kind(), "WriteRecord");
    assert_eq!(intent.declaration().display_name(), "Write record");
    assert_eq!(
        intent.declaration().description(),
        "Writes a value to the record file"
    );
    let hash = intent
        .swift_type()
        .strip_prefix("WUIIntent_")
        .expect("the macro emits a stable intent type");
    assert_eq!(hash.len(), 64);
    assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_eq!(intent.symbol(), format!("waterui_intent_widget_app_{hash}"));

    // The `waterui-widgets` static record the extension build locates the
    // shipped Swift sources through rides the same artifact.
    let sources = fixture_swift_sources(&symbols);
    assert!(
        sources.join("FFI.swift").is_file(),
        "the record points at the crate's Swift sources: {}",
        sources.display()
    );
}

fn fixture_swift_sources(symbols: &ArtifactSymbols) -> PathBuf {
    PathBuf::from(
        String::from_utf8(
            symbols
                .meta_record("waterui_meta_widgets_swift_sources")
                .expect("the widget sources are declared"),
        )
        .expect("the sources path is UTF-8"),
    )
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "builds framework source and uses the Swift SDK; runs in nightly cli-ignored"]
fn widget_snapshot_round_trips_through_swift() {
    let fixture = build_fixture("dev");
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
        let sources = fixture_swift_sources(&symbols);
        run_swift_probe(
            &fixture,
            output.path(),
            &[
                sources.join("FFI.swift"),
                sources.join("Schema.swift"),
                main,
            ],
            &[],
            &[],
        )
        .await;
    });
}

#[cfg(target_os = "macos")]
async fn run_swift_probe(
    fixture: &WidgetFixture,
    directory: &Path,
    sources: &[PathBuf],
    swift_flags: &[OsString],
    run_args: &[&str],
) {
    let executable = directory.join("widget-probe");
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
    args.push(executable.clone().into_os_string());
    args.extend(swift_flags.iter().cloned());
    args.extend(sources.iter().map(|path| path.as_os_str().to_owned()));
    args.push(fixture.artifact.as_os_str().to_owned());
    args.extend(fixture.native_links.iter().cloned());
    let host = Host::current().with_cwd(directory);
    let compile = host
        .output_within("xcrun", args, Duration::from_secs(120))
        .await
        .expect("compile the shipped Swift bridge against the Rust exports");
    assert!(compile.status.success(), "{compile:?}");
    let run = host
        .output_within(&executable, run_args, Duration::from_secs(30))
        .await
        .expect("execute the native Swift probe");
    assert!(run.status.success(), "{run:?}");
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "builds framework source and executes AppIntents; runs in nightly cli-ignored"]
fn widget_intent_performs_through_generated_app_intent() {
    #[derive(askama::Template)]
    #[template(path = "apple_widgets/IntentProbe.swift.tpl", escape = "none")]
    struct IntentProbe<'a> {
        provider_symbol: &'a str,
        intent_type: &'a str,
    }

    let fixture = build_fixture("dev");
    smol::block_on(async {
        let output = tempfile::tempdir_in(&fixture.target_dir).expect("intent probe directory");
        let symbols = ArtifactSymbols::read(&fixture.artifact).expect("read the fixture artifact");
        let intents = declared_intents(&symbols).expect("the fixture declares its action");
        let intent = &intents[0];
        let widgets = declared_widgets(&symbols).expect("the fixture declares its widgets");
        let widget = widgets
            .iter()
            .find(|widget| widget.kind() == "record-kind")
            .expect("the record widget invokes the action");
        let shipped_sources = fixture_swift_sources(&symbols);
        let generated = output.path().join("DeclaredIntents.swift");
        render_declared_intents_swift(&generated, &intents)
            .await
            .expect("render the production AppIntent declarations");
        let probe = output.path().join("IntentProbe.swift");
        smol::fs::write(
            &probe,
            askama::Template::render(&IntentProbe {
                provider_symbol: widget.symbol(),
                intent_type: intent.swift_type(),
            })
            .expect("the intent probe template renders"),
        )
        .await
        .expect("write the generated probe");
        let mut sources = widget_swift_sources(&shipped_sources)
            .await
            .expect("collect the extension's shipped Swift sources");
        sources.extend([generated, probe]);
        let flags = [
            "-parse-as-library".into(),
            "-target".into(),
            format!("arm64-apple-macosx{WIDGET_MINIMUM_APPLE_VERSION}").into(),
        ];
        run_swift_probe(&fixture, output.path(), &sources, &flags, &[intent.kind()]).await;
        assert!(output.path().join("record.txt").is_file());
    });
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "builds framework source and uses the Swift SDK; runs in nightly cli-ignored"]
fn widget_renderer_typechecks_without_intents() {
    let fixture = build_fixture("dev");
    smol::block_on(async {
        let output = tempfile::tempdir_in(&fixture.target_dir).expect("renderer probe directory");
        let symbols = ArtifactSymbols::read(&fixture.artifact).expect("read the fixture artifact");
        let generated = output.path().join("DeclaredIntents.swift");
        render_declared_intents_swift(&generated, &[])
            .await
            .expect("render the empty intent dispatch");
        let mut args: Vec<OsString> = [
            "--sdk",
            "macosx",
            "swiftc",
            "-typecheck",
            "-enable-experimental-feature",
            "Extern",
            "-target",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        args.push(format!("arm64-apple-macosx{WIDGET_MINIMUM_APPLE_VERSION}").into());
        args.extend(
            widget_swift_sources(&fixture_swift_sources(&symbols))
                .await
                .expect("collect the extension's shipped Swift sources")
                .into_iter()
                .chain([generated])
                .map(PathBuf::into_os_string),
        );
        let checked = Host::current()
            .output_within("xcrun", args, Duration::from_secs(120))
            .await
            .expect("typecheck the renderer with no declared intents");
        assert!(checked.status.success(), "{checked:?}");
    });
}
