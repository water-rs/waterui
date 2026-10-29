//! WidgetKit extension assembly.
//!
//! `#[widget]` functions emit `waterui_widget_<crate>_<fn>` exports and
//! `waterui_meta_widget_<crate>_<fn>` declaration records into the project
//! crate's artifact. Packaging pairs each export with its record by name
//! suffix, generates the extension's `WidgetBundle` entry point, compiles
//! the crate's shipped `SwiftUI`/`WidgetKit` sources plus that file into
//! the `.appex` executable, and signs it inside the app bundle's `PlugIns`
//! directory.
//!
//! The extension links the same `libwaterui_app` the app does — the
//! static archive for a package build, the staged shared-runtime dylib for
//! a development build — so the `waterui_widget_*` exports resolve
//! everywhere the artifact shape packages them.

use std::path::{Path, PathBuf};

use eyre::{Context as _, bail, eyre};
#[cfg(target_os = "macos")]
use smol::fs;
#[cfg(target_os = "macos")]
use target_lexicon::Architecture;
use target_lexicon::Triple;
#[cfg(target_os = "macos")]
use tracing::info;
use waterui_widgets::{IntentDeclaration, WidgetDeclaration};

/// The `WidgetKit` deployment floor the extension compiles against —
/// `containerBackground` needs `macOS`/`iOS` 26.
pub const WIDGET_MINIMUM_APPLE_VERSION: &str = "26.0";

#[cfg(target_os = "macos")]
use crate::apple::app_bundle;
use crate::apple::app_bundle::AppleAppLayout;
#[cfg(target_os = "macos")]
use crate::apple::toolchain::development_team_id;
use crate::artifact_symbols::ArtifactSymbols;
use crate::build::BuiltTarget;
#[cfg(target_os = "macos")]
use crate::build::{BuildOptions, BuildProfile, NativeLink};
#[cfg(target_os = "macos")]
use crate::platform::{DeviceSigning, PackageAudience};
use crate::platform::{PackageOptions, TargetPlatform};
use crate::project::Project;
use crate::project_model::project_types::AppleBundleIdentifier;
#[cfg(target_os = "macos")]
use smol::stream::StreamExt as _;

/// One `#[widget]` declaration paired with its export symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeclaredWidget {
    /// The attributed function's name.
    pub function: String,
    /// The `WidgetConfiguration` kind string.
    pub kind: String,
    /// The widget gallery title.
    pub display_name: String,
    /// The widget gallery subtitle.
    pub description: String,
    /// `WidgetFamily` case names the widget supports.
    pub families: Vec<String>,
    /// The `waterui_widget_<crate>_<fn>` C symbol the extension calls.
    pub symbol: String,
}

/// One declared `#[widget_intent]` action — the record pairs its
/// `waterui_intent_<crate>_<hash>` export with its
/// `waterui_meta_intent_<crate>_<hash>` record by the shared hash suffix.
#[derive(Debug)]
pub struct DeclaredIntent {
    declaration: IntentDeclaration,
    /// The `waterui_intent_<crate>_<hash>` C symbol the extension calls.
    symbol: String,
}

impl DeclaredIntent {
    /// The declaration's wire record.
    #[must_use]
    pub const fn declaration(&self) -> &IntentDeclaration {
        &self.declaration
    }
    /// The declared kind — unique per application.
    #[must_use]
    pub fn kind(&self) -> &str {
        self.declaration.kind()
    }
    /// The `Swift` `AppIntent` type the record names.
    #[must_use]
    pub fn swift_type(&self) -> &str {
        self.declaration.swift_type()
    }
    /// The `waterui_intent_<crate>_<hash>` C symbol.
    #[must_use]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }
}

/// Every `waterui_widget_*` export paired with its `waterui_meta_widget_*`
/// declaration record.
///
/// The suffix after `waterui_` is identical between the two names, so the
/// pair survives mangled crate names without the CLI reconstructing either.
///
/// # Errors
/// Returns an error when an export has no record, when a record has no
/// export, or when a payload does not decode.
pub fn declared_widgets(symbols: &ArtifactSymbols) -> eyre::Result<Vec<DeclaredWidget>> {
    let mut widgets = Vec::new();
    for export in symbols.leaves_with_prefix("waterui_widget_") {
        let record = format!("waterui_meta_{}", &export["waterui_".len()..]);
        let payload = symbols.meta_record(&record).wrap_err_with(|| {
            format!("widget export `{export}` has no `{record}` declaration record")
        })?;
        let declaration: WidgetDeclaration = serde_json::from_slice(&payload)
            .wrap_err_with(|| format!("`{record}`'s widget declaration does not decode"))?;
        widgets.push(DeclaredWidget {
            function: declaration.function,
            kind: declaration.kind,
            display_name: declaration.display_name,
            description: declaration.description,
            families: declaration.families,
            symbol: export,
        });
    }
    let exports: std::collections::BTreeSet<String> =
        widgets.iter().map(|widget| widget.symbol.clone()).collect();
    for record in symbols.leaves_with_prefix("waterui_meta_widget_") {
        let export = format!("waterui_{}", &record["waterui_meta_".len()..]);
        if !exports.contains(&export) {
            bail!("widget declaration `{record}` has no `{export}` export symbol");
        }
    }
    widgets.sort_by(|a, b| a.kind.cmp(&b.kind));
    for pair in widgets.windows(2) {
        if pair[0].kind == pair[1].kind {
            bail!(
                "widgets `{}` and `{}` declare the same `Widget` kind `{}`",
                pair[0].symbol,
                pair[1].symbol,
                pair[0].kind
            );
        }
    }
    Ok(widgets)
}

/// A declaration's `families` must exist on `platform`: `accessory*`
/// families are iOS/watchOS-only — the extension fails to compile rather
/// than the CLI silently dropping them.
fn validate_families(widgets: &[DeclaredWidget], platform: TargetPlatform) -> eyre::Result<()> {
    let unsupported: Vec<String> = widgets
        .iter()
        .flat_map(|widget| {
            widget
                .families
                .iter()
                .filter(|family| family.starts_with("accessory"))
                .map(|family| format!("`{family}` (widget `{}`)", widget.kind))
        })
        .collect();
    if platform == TargetPlatform::MacOS && !unsupported.is_empty() {
        bail!(
            "widgets {} declare accessory families, which macOS widgets do not support",
            unsupported.join(", ")
        );
    }
    Ok(())
}

/// The swift target triple the extension compiles for — the
/// architecture the Rust slice carries, not a hard-coded arm64.
#[cfg(target_os = "macos")]
fn widget_swift_target(
    platform: TargetPlatform,
    triple: &Triple,
    deployment_target: &str,
) -> eyre::Result<String> {
    // Apple support is ARM64-only: Intel macOS and Intel iOS simulators
    // are retired targets.
    let arch = match triple.architecture {
        Architecture::Aarch64(_) => "arm64",
        Architecture::X86_64 => {
            bail!("Intel Apple targets are retired and unsupported (got {triple})")
        }
        other => bail!("WidgetKit extensions do not support the {other:?} architecture"),
    };
    let target = match platform {
        TargetPlatform::MacOS => format!("{arch}-apple-macosx{deployment_target}"),
        TargetPlatform::IOS => format!("{arch}-apple-ios{deployment_target}"),
        TargetPlatform::IOSSimulator => {
            format!("{arch}-apple-ios{deployment_target}-simulator")
        }
        other => bail!("WidgetKit extensions are not supported on {other:?}"),
    };
    Ok(target)
}

/// The `PlugIns` directory of the app bundle at `app_path`.
#[cfg(target_os = "macos")]
fn plugins_dir(app_path: &Path, sdk_name: &str) -> PathBuf {
    if sdk_name == "macosx" {
        app_path.join("Contents").join("PlugIns")
    } else {
        app_path.join("PlugIns")
    }
}

/// Generates, compiles, bundles and signs the `WidgetKit` extension for the
/// declared widgets, when the artifact declares any.
///
/// Placed after the shared-runtime staging so the development dylib is in
/// `Frameworks` when the extension links it, and before the app's own
/// signature so the `.appex` is signed inside-out the way `xcodebuild`
/// signs.
///
/// # Errors
/// Returns an error when declarations do not pair with exports, when a
/// declared family is unsupported on `platform`, when the crate's Swift
/// sources record is absent, or when compilation, bundling or signing
/// fails.
#[cfg_attr(
    not(target_os = "macos"),
    allow(unused_variables, clippy::unused_async)
)]
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is a distinct input of the packaging step; bundling them would only rename the list"
)]
pub async fn assemble_widget_extensions(
    project: &Project,
    platform: TargetPlatform,
    triple: &Triple,
    layout: &AppleAppLayout,
    built: &BuiltTarget,
    options: &PackageOptions,
    backend_root: &Path,
    deployment_target: &str,
    bundle_id: &AppleBundleIdentifier,
    product_name: &str,
) -> eyre::Result<Option<PathBuf>> {
    let widgets = declared_widgets(&built.app_symbols().await?)?;
    if widgets.is_empty() {
        return Ok(None);
    }
    let intents = declared_intents(&built.app_symbols().await?)?;
    validate_families(&widgets, platform)?;

    let artifact = built.artifact.clone();
    let composed = smol::unblock(move || ArtifactSymbols::read(&artifact)).await?;
    let sources = composed
        .meta_record("waterui_meta_widgets_swift_sources")
        .map(|payload| PathBuf::from(String::from_utf8_lossy(&payload).into_owned()))
        .wrap_err(
            "the app declares widgets but its `waterui-widgets` dependency carried no `waterui_meta_widgets_swift_sources` record",
        )?;
    if !sources.is_dir() {
        bail!(
            "the `waterui-widgets` Swift sources at {} do not exist",
            sources.display()
        );
    }
    let sdk_name = platform
        .sdk_name()
        .ok_or_else(|| eyre!("{platform:?} is not an Apple platform"))?;

    #[cfg(target_os = "macos")]
    {
        let staging = backend_root.join("DerivedData").join("WidgetExtension");
        fs::create_dir_all(&staging).await?;
        render_widgets_swift(&staging.join("Widgets.swift"), &widgets).await?;
        render_declared_intents_swift(&staging.join("DeclaredIntents.swift"), &intents).await?;

        let appex_name = format!("{product_name}Widgets");
        let appex_path =
            plugins_dir(&layout.app_path, sdk_name).join(format!("{appex_name}.appex"));
        let appex_layout = AppleAppLayout::for_app(&appex_path, sdk_name);
        fs::create_dir_all(&appex_layout.executable_dir).await?;

        let compiled = compile_extension(
            project,
            platform,
            triple,
            built,
            options,
            layout,
            &appex_layout,
            &appex_name,
            &sources,
            &[
                staging.join("Widgets.swift"),
                staging.join("DeclaredIntents.swift"),
            ],
            sdk_name,
            deployment_target,
            &intents,
            bundle_id,
            &staging,
        )
        .await?;

        emit_and_validate_intents_metadata(
            project.host(),
            &appex_layout,
            &compiled,
            platform,
            deployment_target,
            &staging,
            &intents,
        )
        .await?;

        write_extension_info_plist(
            layout,
            &appex_layout,
            &appex_name,
            bundle_id,
            deployment_target,
            sdk_name,
        )
        .await?;

        sign_widget_appex(
            project,
            platform,
            options,
            &appex_layout,
            &format!("{}.widgets", bundle_id.as_str()),
            backend_root,
            deployment_target,
        )
        .await?;

        info!(
            "Built WidgetKit extension {} ({} widget{})",
            appex_path.display(),
            widgets.len(),
            if widgets.len() == 1 { "" } else { "s" }
        );
        Ok(Some(appex_path))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (sources, sdk_name, widgets);
        bail!("WidgetKit extension assembly requires macOS (swiftc and codesign)")
    }
}

/// Renders the extension's entry point — the `WidgetBundle` declaring one
/// `WaterUIWidget` per record, each bound to its `waterui_widget_*` export.
#[cfg(target_os = "macos")]
async fn render_widgets_swift(path: &Path, widgets: &[DeclaredWidget]) -> eyre::Result<()> {
    #[derive(askama::Template)]
    #[template(path = "apple_widgets/Widgets.swift.tpl", escape = "none")]
    struct WidgetsTemplate<'a> {
        widgets: &'a [DeclaredWidgetTemplate],
    }

    #[derive(Debug)]
    struct DeclaredWidgetTemplate {
        index: usize,
        symbol: String,
        kind: String,
        display_name: String,
        description: String,
        families: String,
    }

    // String fields are pre-rendered as `Swift` string literals — `JSON`
    // escaping is not `Swift` escaping: a control character encodes
    // `\u0001`, which is not valid `Swift` source (`\u{1}` is).
    let widgets: Vec<DeclaredWidgetTemplate> = widgets
        .iter()
        .enumerate()
        .map(|(index, widget)| DeclaredWidgetTemplate {
            index,
            symbol: widget.symbol.clone(),
            kind: swift_string_literal(&widget.kind),
            display_name: swift_string_literal(&widget.display_name),
            description: swift_string_literal(&widget.description),
            families: widget
                .families
                .iter()
                .map(|family| format!(".{family}"))
                .collect::<Vec<_>>()
                .join(", "),
        })
        .collect();

    fs::write(
        path,
        askama::Template::render(&WidgetsTemplate { widgets: &widgets })
            .expect("the widgets template always renders"),
    )
    .await?;
    Ok(())
}

/// Renders `DeclaredIntents.swift` — one concrete `AppIntent` per declared
/// `#[widget_intent]` plus the `WUIDeclaredIntentButton` dispatch view.
///
/// # Errors
///
/// `path` must be writable.
///
/// # Panics
///
/// The `DeclaredIntents.swift` template always renders.
#[cfg(target_os = "macos")]
pub async fn render_declared_intents_swift(
    path: &Path,
    intents: &[DeclaredIntent],
) -> eyre::Result<()> {
    #[derive(askama::Template)]
    #[template(path = "apple_widgets/DeclaredIntents.swift.tpl", escape = "none")]
    struct DeclaredIntentsTemplate<'a> {
        intents: &'a [DeclaredIntentTemplate],
    }

    #[derive(Debug)]
    struct DeclaredIntentTemplate {
        index: usize,
        swift_type: String,
        kind: String,
        display_name: String,
        description: String,
        symbol: String,
    }

    // User strings need Swift escaping; type and symbol names are
    // compiler-emitted identifiers.
    let intents: Vec<DeclaredIntentTemplate> = intents
        .iter()
        .enumerate()
        .map(|(index, intent)| DeclaredIntentTemplate {
            index,
            swift_type: intent.swift_type().to_string(),
            kind: swift_string_literal(intent.kind()),
            display_name: swift_string_literal(intent.declaration().display_name()),
            description: swift_string_literal(intent.declaration().description()),
            symbol: intent.symbol().to_string(),
        })
        .collect();

    fs::write(
        path,
        askama::Template::render(&DeclaredIntentsTemplate { intents: &intents })
            .expect("the declared-intents template always renders"),
    )
    .await?;
    Ok(())
}

/// Compiles the crate's shipped Swift sources plus the generated entry
/// point into the `.appex` executable, linking `libwaterui_app` in the
/// shape this package built.
#[cfg(target_os = "macos")]
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is a distinct input of the compile step; bundling them would only rename the list"
)]
async fn compile_extension(
    project: &Project,
    platform: TargetPlatform,
    triple: &Triple,
    built: &BuiltTarget,
    options: &PackageOptions,
    app_layout: &AppleAppLayout,
    appex_layout: &AppleAppLayout,
    appex_name: &str,
    sources: &Path,
    generated: &[PathBuf],
    sdk_name: &str,
    deployment_target: &str,
    intents: &[DeclaredIntent],
    bundle_id: &AppleBundleIdentifier,
    staging_dir: &Path,
) -> eyre::Result<ExtensionCompile> {
    let host = project.host();
    let sdk_path = host
        .run("xcrun", ["--sdk", sdk_name, "--show-sdk-path"])
        .await
        .wrap_err_with(|| format!("xcrun reports no {sdk_name} SDK path"))?;
    let swift_target = widget_swift_target(platform, triple, deployment_target)?;

    // The generated `AppIntent` types need a stable module identity the
    // const-gather compiler output names — derived from the bundle
    // identifier, not the display product name.
    let module_name = swift_module_name(&format!("{}.widgets", bundle_id.as_str()));
    let toolchain_dir = swift_toolchain_dir(host).await?;

    let mut source_files = widget_swift_sources(sources).await?;
    source_files.extend(generated.iter().cloned());

    let executable = appex_layout.executable_file(appex_name);
    let mut args: Vec<std::ffi::OsString> = vec![
        "-emit-executable".into(),
        "-module-name".into(),
        module_name.clone().into(),
        "-sdk".into(),
        sdk_path.trim().into(),
        "-target".into(),
        swift_target.clone().into(),
        "-application-extension".into(),
        "-enable-experimental-feature".into(),
        "Extern".into(),
        "-o".into(),
        executable.as_os_str().into(),
    ];
    // Release extensions optimize like the packaged artifact they link.
    if !options.is_debug() {
        args.push("-O".into());
    }
    // Declared `AppIntent`s need const-gather metadata — the compiler
    // emits the declarations' constant values `appintentsmetadataprocessor`
    // packages into `Metadata.appintents`. Artifacts are build inputs —
    // they land in the staging dir, never the `.appex` bundle.
    let const_values_path = if intents.is_empty() {
        None
    } else {
        let const_values = staging_dir.join("const_values.json");
        let protocols = app_intents_protocols_list(&toolchain_dir, staging_dir).await?;
        args.extend([
            "-const-gather-protocols-list".into(),
            protocols.into_os_string(),
            "-emit-const-values-path".into(),
            const_values.clone().into_os_string(),
            "-wmo".into(),
        ]);
        Some(const_values)
    };
    args.extend(source_files.iter().map(|path| path.as_os_str().into()));

    args.extend(
        rust_widget_link_arguments(
            project, built, options, platform, triple, app_layout, sdk_name,
        )
        .await?,
    );

    host.run(&toolchain_dir.join("usr/bin/swiftc"), &args)
        .await
        .wrap_err("failed to compile the WidgetKit extension")?;
    Ok(ExtensionCompile {
        module_name,
        sdk_root: sdk_path.trim().into(),
        target: swift_target,
        source_files,
        const_values_path,
        toolchain_dir,
    })
}

/// The `.appex`'s `Info.plist` — an `XPC!` extension whose sole declared
/// extension point is `com.apple.widgetkit-extension`.
#[cfg(target_os = "macos")]
async fn write_extension_info_plist(
    app_layout: &AppleAppLayout,
    appex_layout: &AppleAppLayout,
    appex_name: &str,
    bundle_id: &AppleBundleIdentifier,
    deployment_target: &str,
    sdk_name: &str,
) -> eyre::Result<()> {
    let host_plist: plist::Value = plist::from_bytes(
        &fs::read(&app_layout.info_plist_path)
            .await
            .wrap_err_with(|| {
                format!(
                    "the host app's Info.plist at {} does not read",
                    app_layout.info_plist_path.display()
                )
            })?,
    )
    .wrap_err("the host app's Info.plist does not decode")?;
    let host_dict = host_plist
        .as_dictionary()
        .ok_or_else(|| eyre::eyre!("the host app's Info.plist is not a dictionary"))?;
    let host_version = |key: &str| -> eyre::Result<plist::Value> {
        host_dict
            .get(key)
            .cloned()
            .ok_or_else(|| eyre::eyre!("the host app's Info.plist has no {key}"))
    };
    let mut dict = plist::Dictionary::new();
    let insert = |dict: &mut plist::Dictionary, key: &str, value: &str| {
        dict.insert(key.to_string(), plist::Value::String(value.to_string()));
    };
    insert(&mut dict, "CFBundleDevelopmentRegion", "en");
    insert(&mut dict, "CFBundleExecutable", appex_name);
    insert(
        &mut dict,
        "CFBundleIdentifier",
        &format!("{}.widgets", bundle_id.as_str()),
    );
    insert(&mut dict, "CFBundleInfoDictionaryVersion", "6.0");
    insert(&mut dict, "CFBundleName", appex_name);
    insert(&mut dict, "CFBundlePackageType", "XPC!");
    // The extension reports the host app's versions verbatim — a
    // `1.0/1` stub would misrepresent the bundle to the App Store.
    dict.insert(
        "CFBundleShortVersionString".to_string(),
        host_version("CFBundleShortVersionString")?,
    );
    dict.insert(
        "CFBundleVersion".to_string(),
        host_version("CFBundleVersion")?,
    );
    // `MinimumOSVersion` is the iOS-family key; the macOS appex plist
    // carries `LSMinimumSystemVersion`.
    if sdk_name == "macosx" {
        insert(&mut dict, "LSMinimumSystemVersion", deployment_target);
    } else {
        insert(&mut dict, "MinimumOSVersion", deployment_target);
    }
    let mut extension = plist::Dictionary::new();
    insert(
        &mut extension,
        "NSExtensionPointIdentifier",
        "com.apple.widgetkit-extension",
    );
    dict.insert(
        "NSExtension".to_string(),
        plist::Value::Dictionary(extension),
    );

    if let Some(parent) = appex_layout.info_plist_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let mut serialized = Vec::new();
    plist::Value::Dictionary(dict)
        .to_writer_xml(&mut serialized)
        .wrap_err("failed to serialize the widget extension Info.plist")?;
    fs::write(&appex_layout.info_plist_path, serialized).await?;
    Ok(())
}

/// Signs the `.appex` — ad-hoc for simulator and unsigned packages, the
/// development provisioning profile matching its own bundle identifier for
/// device packages, `sign_macos_app`'s development signature on macOS.
#[cfg(target_os = "macos")]
async fn sign_widget_appex(
    project: &Project,
    platform: TargetPlatform,
    options: &PackageOptions,
    appex_layout: &AppleAppLayout,
    widget_bundle_id: &str,
    backend_root: &Path,
    deployment_target: &str,
) -> eyre::Result<()> {
    use crate::apple::app_bundle::codesign_path;
    let host = project.host();

    if platform == TargetPlatform::MacOS {
        let signing = match options.audience() {
            PackageAudience::Development => crate::macos_bundle::MacOsSigning::Development {
                requires_stable_identity: false,
            },
            PackageAudience::Distribution => crate::macos_bundle::MacOsSigning::Distribution(
                crate::macos_bundle::DistributionSigning::from_manifest(
                    project.manifest().signing.macos.as_ref(),
                    None,
                )?,
            ),
        };
        let widget_id = AppleBundleIdentifier::try_from(widget_bundle_id.to_string())
            .map_err(eyre::Report::msg)?;
        crate::macos_bundle::sign_macos_app(host, &appex_layout.app_path, &widget_id, &signing)
            .await?;
        return Ok(());
    }

    if platform.is_simulator() {
        codesign_path(host, &appex_layout.app_path, "-", None, None).await?;
        return Ok(());
    }

    if options.device_signing() == DeviceSigning::Unsigned {
        return Ok(());
    }

    // A device build's `.appex` needs its own provisioning profile, exactly
    // the way the app gets its own: the same development team, the
    // extension's `<app>.widgets` bundle identifier, and an empty
    // entitlement set — the extension declares no capabilities.
    let team = development_team_id(host).await?;
    let request = crate::apple::provisioning::SigningRequest {
        team: &team,
        bundle_id: widget_bundle_id,
        device_udid: options.device_udid(),
        entitlements: &plist::Dictionary::new(),
        platform,
    };
    app_bundle::sign_development_bundle(
        host,
        appex_layout,
        &request,
        &backend_root.join("DerivedData/Provisioning"),
        deployment_target,
    )
    .await
}

/// Renders `s` as a `Swift` string literal — control characters escape as
/// `\u{…}` (not the `JSON` `\uXXXX`), quotes and backslashes escape, and
/// everything else — Unicode, `\(` included — stays verbatim so a widget
/// declaration's strings round-trip through generated source.
#[cfg(target_os = "macos")]
fn swift_string_literal(s: &str) -> String {
    use std::fmt::Write as _;
    let mut literal = String::with_capacity(s.len() + 2);
    literal.push('"');
    for c in s.chars() {
        match c {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            c if c.is_control() => {
                write!(literal, "\\u{{{:x}}}", c as u32).expect("writing to a String cannot fail");
            }
            c => literal.push(c),
        }
    }
    literal.push('"');
    literal
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use target_lexicon::Triple;

    #[test]
    fn widget_swift_target_uses_the_rust_slices_architecture() {
        let triple = |value: &str| value.parse::<Triple>().expect("a triple parses");
        assert_eq!(
            widget_swift_target(
                TargetPlatform::MacOS,
                &triple("aarch64-apple-darwin"),
                WIDGET_MINIMUM_APPLE_VERSION
            )
            .unwrap(),
            "arm64-apple-macosx26.0"
        );
        for unsupported in ["x86_64-apple-darwin", "x86_64-apple-ios"] {
            let error = widget_swift_target(
                TargetPlatform::MacOS,
                &triple(unsupported),
                WIDGET_MINIMUM_APPLE_VERSION,
            )
            .expect_err("an Intel Apple target is rejected");
            assert!(
                error.to_string().contains("retired"),
                "the rejection names Intel retirement: {error}"
            );
        }
        let error = widget_swift_target(
            TargetPlatform::MacOS,
            &triple("wasm32-unknown-unknown"),
            WIDGET_MINIMUM_APPLE_VERSION,
        )
        .expect_err("a non-Apple architecture is rejected");
        assert!(error.to_string().contains("architecture"));
    }

    #[test]
    fn swift_string_literal_escapes_for_swift_not_json() {
        assert_eq!(swift_string_literal("plain"), "\"plain\"");
        assert_eq!(swift_string_literal("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(swift_string_literal("back\\slash"), "\"back\\\\slash\"");
        // A raw control character escapes as `\u{…}`, not `JSON`'s `\uXXXX`.
        assert_eq!(swift_string_literal("a\u{1}"), "\"a\\u{1}\"");
        // Unicode and interpolation-looking text stay verbatim.
        assert_eq!(swift_string_literal("日本語 \\(x)"), "\"日本語 \\\\(x)\"");
    }

    #[test]
    fn extension_plist_copies_the_host_versions() {
        let staging = tempfile::tempdir().expect("staging dir");
        let host_path = staging.path().join("Host.app");
        let host_layout = AppleAppLayout::for_app(&host_path, "macosx");
        std::fs::create_dir_all(host_layout.info_plist_path.parent().unwrap()).unwrap();
        let mut host = plist::Dictionary::new();
        host.insert(
            "CFBundleShortVersionString".to_string(),
            plist::Value::String("3.2.1".to_string()),
        );
        host.insert(
            "CFBundleVersion".to_string(),
            plist::Value::String("42".to_string()),
        );
        plist::Value::Dictionary(host)
            .to_file_xml(&host_layout.info_plist_path)
            .unwrap();
        let appex_layout = AppleAppLayout::for_app(
            &host_path.join("Contents/PlugIns/HostWidgets.appex"),
            "macosx",
        );
        std::fs::create_dir_all(appex_layout.info_plist_path.parent().unwrap()).unwrap();
        let bundle_id = AppleBundleIdentifier::try_from("dev.test.app".to_string())
            .expect("a valid bundle identifier");
        smol::block_on(write_extension_info_plist(
            &host_layout,
            &appex_layout,
            "HostWidgets",
            &bundle_id,
            WIDGET_MINIMUM_APPLE_VERSION,
            "macosx",
        ))
        .expect("the extension plist writes");
        let appex: plist::Value = plist::from_file(&appex_layout.info_plist_path).unwrap();
        let dict = appex.as_dictionary().unwrap();
        assert_eq!(
            dict["CFBundleShortVersionString"].as_string().unwrap(),
            "3.2.1",
            "the extension carries the host's marketing version"
        );
        assert_eq!(
            dict["CFBundleVersion"].as_string().unwrap(),
            "42",
            "the extension carries the host's build number"
        );
    }

    fn test_intent(kind: &str, symbol: &str) -> DeclaredIntent {
        DeclaredIntent {
            declaration: serde_json::from_value(serde_json::json!({
                "kind": kind,
                "displayName": kind,
                "description": kind,
                "swiftType": "WUIIntent_test",
            }))
            .unwrap(),
            symbol: symbol.into(),
        }
    }

    #[test]
    fn sort_declared_intents_rejects_duplicate_kinds() {
        let mut intents = vec![
            test_intent("WriteRecord", "waterui_intent_crate_a_1111"),
            test_intent("WriteRecord", "waterui_intent_crate_b_2222"),
        ];
        let error = sort_declared_intents(&mut intents)
            .expect_err("two declarations of the same kind are rejected");
        let message = error.to_string();
        assert!(message.contains("waterui_intent_crate_a_1111"), "{message}");
        assert!(message.contains("waterui_intent_crate_b_2222"), "{message}");
        assert!(message.contains("WriteRecord"), "{message}");
    }

    #[test]
    fn sort_declared_intents_orders_by_kind() {
        let mut intents = vec![
            test_intent("z-kind", "waterui_intent_crate_a_1111"),
            test_intent("a-kind", "waterui_intent_crate_b_2222"),
        ];
        sort_declared_intents(&mut intents).expect("distinct kinds sort");
        assert_eq!(intents[0].kind(), "a-kind");
        assert_eq!(intents[1].kind(), "z-kind");
    }
}

/// Sorts declared intents by kind and rejects two declarations of the
/// same kind — the extension's `AppIntent` dispatch keys on it.
///
/// # Errors
///
/// Two `DeclaredIntent`s must not share a `kind`.
fn sort_declared_intents(intents: &mut [DeclaredIntent]) -> eyre::Result<()> {
    intents.sort_by(|a, b| a.kind().cmp(b.kind()));
    for pair in intents.windows(2) {
        if pair[0].kind() == pair[1].kind() {
            bail!(
                "intents `{}` and `{}` declare the same kind `{}`",
                pair[0].symbol(),
                pair[1].symbol(),
                pair[0].kind()
            );
        }
    }
    Ok(())
}

/// Pairs each `waterui_intent_*` export with its `waterui_meta_intent_*`
/// record — the same enumeration `declared_widgets` uses for snapshot
/// exports, keyed on the shared hash suffix after the crate name.
///
/// # Errors
///
/// An intent export without its record (or vice versa), a record that
/// does not decode, or two intents declaring the same kind.
pub fn declared_intents(symbols: &ArtifactSymbols) -> eyre::Result<Vec<DeclaredIntent>> {
    let mut intents = Vec::new();
    for export in symbols.leaves_with_prefix("waterui_intent_") {
        let record = format!("waterui_meta_{}", &export["waterui_".len()..]);
        let payload = symbols.meta_record(&record).wrap_err_with(|| {
            format!("intent export `{export}` has no `{record}` declaration record")
        })?;
        let declaration: IntentDeclaration = serde_json::from_slice(&payload)
            .wrap_err_with(|| format!("`{record}`'s intent declaration does not decode"))?;
        intents.push(DeclaredIntent {
            declaration,
            symbol: export,
        });
    }
    let intent_exports: std::collections::BTreeSet<String> =
        intents.iter().map(|intent| intent.symbol.clone()).collect();
    for record in symbols.leaves_with_prefix("waterui_meta_intent_") {
        let export = format!("waterui_{}", &record["waterui_meta_".len()..]);
        if !intent_exports.contains(&export) {
            bail!("intent declaration `{record}` has no `{export}` export symbol");
        }
    }
    sort_declared_intents(&mut intents)?;
    Ok(intents)
}

/// What `compile_extension` produced — the inputs
/// `appintentsmetadataprocessor` reuses rather than recomputing.
#[cfg(target_os = "macos")]
struct ExtensionCompile {
    module_name: String,
    sdk_root: PathBuf,
    target: String,
    source_files: Vec<PathBuf>,
    const_values_path: Option<PathBuf>,
    toolchain_dir: PathBuf,
}

/// The generated `Swift` module name — ASCII letters/digits/underscores,
/// a letter prefix, from the app's bundle identifier.
#[cfg(target_os = "macos")]
fn swift_module_name(bundle_id: &str) -> String {
    let mut name = String::with_capacity(bundle_id.len());
    for c in bundle_id.chars() {
        name.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    if name.starts_with(|c: char| c.is_ascii_digit()) {
        name.insert(0, 'w');
    }
    name
}

/// The active `Xcode` toolchain root — `<toolchain>/usr/bin/swiftc` →
/// `<toolchain>`.
#[cfg(target_os = "macos")]
async fn swift_toolchain_dir(host: &crate::toolchain::host::Host) -> eyre::Result<PathBuf> {
    let swiftc = host
        .run("xcrun", ["--find", "swiftc"])
        .await
        .wrap_err("xcrun reports no swiftc")?;
    PathBuf::from(swiftc.trim())
        .ancestors()
        .nth(3)
        .map(PathBuf::from)
        .ok_or_else(|| eyre!("swiftc at {swiftc:?} is not inside a toolchain"))
}

#[cfg(target_os = "macos")]
async fn app_intents_protocols_list(toolchain: &Path, staging_dir: &Path) -> eyre::Result<PathBuf> {
    let official = toolchain.join("usr/share/swift/SwiftConstantValues/AppIntents.json");
    if !official.is_file() {
        bail!(
            "the toolchain {} has no AppIntents const-gather protocol list",
            toolchain.display()
        );
    }
    let doc: serde_json::Value =
        serde_json::from_slice(&fs::read(&official).await.wrap_err_with(|| {
            format!(
                "the AppIntents protocol list {} does not read",
                official.display()
            )
        })?)
        .wrap_err("the toolchain's AppIntents protocol list does not decode")?;
    let protocols = doc
        .get("constValueProtocols")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .ok_or_else(|| {
            eyre!("the toolchain's AppIntents protocol list has no `constValueProtocols` array")
        })?;
    // `-const-gather-protocols-list` expects the bare protocol-name
    // array, not the toolchain file's `{version, constValueProtocols}`
    // wrapper.
    let out = staging_dir.join("app_intents_protocols.json");
    fs::write(
        &out,
        serde_json::to_vec(&protocols).expect("protocol names serialize"),
    )
    .await?;
    Ok(out)
}

/// Runs `appintentsmetadataprocessor` — packages the declared intents'
/// `Metadata.appintents` into the extension's resources before codesign.
///
/// # Errors
///
/// The toolchain, `Xcode` version, or processor invocation must succeed;
/// `Metadata.appintents` must be written.
///
/// # Panics
///
/// The file-list templates always render.
#[cfg(target_os = "macos")]
async fn emit_app_intents_metadata(
    host: &crate::toolchain::host::Host,
    appex_layout: &AppleAppLayout,
    compile: &ExtensionCompile,
    platform: TargetPlatform,
    deployment_target: &str,
    staging_dir: &Path,
) -> eyre::Result<()> {
    let Some(const_values_path) = &compile.const_values_path else {
        bail!("the extension compile produced no const-gather output");
    };
    let toolchain = compile.toolchain_dir.clone();
    let xcode_version = host
        .run("xcodebuild", ["-version"])
        .await
        .wrap_err("xcodebuild -version failed")?;
    let build_version = xcode_version
        .lines()
        .find_map(|line| line.strip_prefix("Build version "))
        .map(str::trim)
        .ok_or_else(|| eyre!("xcodebuild -version has no Build version line"))?;
    let platform_family = match platform {
        TargetPlatform::MacOS => "macOS",
        TargetPlatform::IOS | TargetPlatform::IOSSimulator => "iOS",
        other => bail!("{other:?} has no AppIntents platform family"),
    };
    // One path per line — `UTF-8` and newline-free at the format boundary.
    let source_files: Vec<&str> = compile
        .source_files
        .iter()
        .map(|path| file_list_path(path))
        .collect::<eyre::Result<Vec<_>>>()?;
    let source_list = staging_dir.join("source_file_list.txt");
    fs::write(
        &source_list,
        askama::Template::render(&FileListTemplate {
            files: &source_files,
        })
        .expect("the file-list template always renders"),
    )
    .await?;
    let const_vals_list = staging_dir.join("swift_const_vals_list.txt");
    let const_vals = [file_list_path(const_values_path)?];
    fs::write(
        &const_vals_list,
        askama::Template::render(&FileListTemplate { files: &const_vals })
            .expect("the file-list template always renders"),
    )
    .await?;
    let resources_dir = appex_layout.resources_dir.clone();
    fs::create_dir_all(&resources_dir).await?;
    let args: Vec<std::ffi::OsString> = vec![
        "--output".into(),
        resources_dir.into_os_string(),
        "--toolchain-dir".into(),
        toolchain.clone().into_os_string(),
        "--module-name".into(),
        compile.module_name.clone().into(),
        "--sdk-root".into(),
        compile.sdk_root.clone().into_os_string(),
        "--xcode-version".into(),
        build_version.into(),
        "--platform-family".into(),
        platform_family.into(),
        "--deployment-target".into(),
        deployment_target.into(),
        "--target-triple".into(),
        compile.target.clone().into(),
        "--source-file-list".into(),
        source_list.into_os_string(),
        "--swift-const-vals-list".into(),
        const_vals_list.into_os_string(),
    ];
    host.run(
        &toolchain.join("usr/bin/appintentsmetadataprocessor"),
        &args,
    )
    .await
    .wrap_err("appintentsmetadataprocessor failed")?;
    Ok(())
}

/// Verifies `Metadata.appintents` actually catalogs each declared intent —
/// the processor can exit `RC 0` while producing an empty catalog.
///
/// # Errors
///
/// The catalog must read, decode, and carry each intent's
/// `module.swift_type` identifier.
#[cfg(target_os = "macos")]
async fn validate_app_intents_metadata(
    appex_layout: &AppleAppLayout,
    compile: &ExtensionCompile,
    intents: &[DeclaredIntent],
) -> eyre::Result<()> {
    #[derive(serde::Deserialize)]
    struct NativeAction {
        #[serde(rename = "fullyQualifiedTypeName")]
        fully_qualified_type_name: String,
    }
    #[derive(serde::Deserialize)]
    struct NativeCatalog {
        actions: std::collections::BTreeMap<String, NativeAction>,
    }
    let metadata_path = appex_layout
        .resources_dir
        .join("Metadata.appintents")
        .join("extract.actionsdata");
    let catalog: NativeCatalog =
        serde_json::from_slice(&fs::read(&metadata_path).await.wrap_err_with(|| {
            format!(
                "the extension's AppIntents actions at {} do not read",
                metadata_path.display()
            )
        })?)
        .wrap_err("the extension's AppIntents actions do not decode")?;
    for intent in intents {
        let expected = format!("{}.{}", compile.module_name, intent.swift_type());
        let found = catalog
            .actions
            .values()
            .any(|action| action.fully_qualified_type_name == expected);
        if !found {
            bail!(
                "Metadata.appintents has no action for the declared intent `{expected}` (kind `{}`)",
                intent.kind()
            );
        }
    }
    Ok(())
}

/// Emits `Metadata.appintents` for the declared intents and validates the
/// catalog the processor produced.
#[cfg(target_os = "macos")]
async fn emit_and_validate_intents_metadata(
    host: &crate::toolchain::host::Host,
    appex_layout: &AppleAppLayout,
    compile: &ExtensionCompile,
    platform: TargetPlatform,
    deployment_target: &str,
    staging_dir: &Path,
    intents: &[DeclaredIntent],
) -> eyre::Result<()> {
    if intents.is_empty() {
        return Ok(());
    }
    emit_app_intents_metadata(
        host,
        appex_layout,
        compile,
        platform,
        deployment_target,
        staging_dir,
    )
    .await?;
    validate_app_intents_metadata(appex_layout, compile, intents).await
}

/// One file path per line — the `appintentsmetadataprocessor` file-list
/// inputs.
#[cfg(target_os = "macos")]
#[derive(askama::Template)]
#[template(path = "apple_widgets/file_list.tpl", escape = "none")]
struct FileListTemplate<'a> {
    files: &'a [&'a str],
}

/// Collects the crate's shipped `Swift` widget sources — `*.swift` under
/// `sources`, sorted — without the generated `Widgets.swift` /
/// `DeclaredIntents.swift` additions.
///
/// # Errors
///
/// `sources` must be a readable directory.
#[cfg(target_os = "macos")]
pub async fn widget_swift_sources(sources: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut source_files = Vec::new();
    let mut entries = fs::read_dir(sources).await.wrap_err_with(|| {
        format!(
            "the widget Swift sources at {} do not read",
            sources.display()
        )
    })?;
    while let Some(entry) = entries.next().await {
        let path = entry?.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("swift") {
            source_files.push(path);
        }
    }
    source_files.sort();
    Ok(source_files)
}

/// A file-list path — the `appintentsmetadataprocessor` file lists are
/// one path per line, so a path must be `UTF-8` and carry no newline.
#[cfg(target_os = "macos")]
fn file_list_path(path: &Path) -> eyre::Result<&str> {
    let text = path
        .to_str()
        .ok_or_else(|| eyre!("{} is not UTF-8", path.display()))?;
    if text.contains(['\n', '\r']) {
        bail!(
            "{} contains a newline — file lists are one path per line",
            path.display()
        );
    }
    Ok(text)
}

/// The `swiftc` link arguments that bring the Rust library into the
/// extension — the staged shared-runtime dylib in a development build, or
/// the static archive plus `rustc`'s own native-link report otherwise.
#[cfg(target_os = "macos")]
async fn rust_widget_link_arguments(
    project: &Project,
    built: &BuiltTarget,
    options: &PackageOptions,
    platform: TargetPlatform,
    triple: &Triple,
    app_layout: &AppleAppLayout,
    sdk_name: &str,
) -> eyre::Result<Vec<std::ffi::OsString>> {
    let mut args: Vec<std::ffi::OsString> = Vec::new();
    if options.uses_shared_rust_runtime(triple) {
        // Development: the app companion is the staged shared-runtime
        // dylib; the extension resolves it through the bundle's Frameworks
        // directory at load time.
        let app_dylib = app_layout.frameworks_dir.join("libwaterui_app.dylib");
        if !app_dylib.is_file() {
            bail!(
                "the shared-runtime app library {} was not staged before the widget extension linked",
                app_dylib.display()
            );
        }
        args.push(app_dylib.as_os_str().into());
        let frameworks_rpath = if sdk_name == "macosx" {
            "@executable_path/../../../../Frameworks"
        } else {
            "@executable_path/../../Frameworks"
        };
        args.extend(
            ["-Xlinker", "-rpath", "-Xlinker", frameworks_rpath]
                .iter()
                .map(std::ffi::OsString::from),
        );
    } else {
        // Static packaging: the archive is linked directly, so the
        // extension carries the snapshot producers and everything they
        // pull — native link requirements come from rustc's own report.
        args.push(built.artifact.as_os_str().into());
        let profile = if options.is_debug() {
            BuildProfile::Debug
        } else {
            BuildProfile::Release
        };
        let build_options = BuildOptions::packaging(profile);
        let rust_build = crate::apple::platform::apple_rust_build(
            project,
            project.apple_crate_path(),
            platform,
            triple.clone(),
            &build_options,
        )
        .await?;
        let links = rust_build
            .with_crate_type_override("staticlib")
            .with_final_rustc_arg("--print=native-static-libs")
            .native_static_libraries(!options.is_debug())
            .await?;
        for NativeLink { name, framework } in links {
            if framework {
                args.push("-framework".into());
                args.push(name.into());
            } else {
                args.push(format!("-l{name}").into());
            }
        }
    }
    Ok(args)
}
