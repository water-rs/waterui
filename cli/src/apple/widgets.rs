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
use waterui_widgets::WidgetDeclaration;

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
use crate::utils::copy_file;
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
    let arch = match triple.architecture {
        Architecture::Aarch64(_) => "arm64",
        Architecture::X86_64 => "x86_64",
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

        let appex_name = format!("{product_name}Widgets");
        let appex_path =
            plugins_dir(&layout.app_path, sdk_name).join(format!("{appex_name}.appex"));
        let appex_layout = AppleAppLayout::for_app(&appex_path, sdk_name);
        fs::create_dir_all(&appex_layout.executable_dir).await?;

        compile_extension(
            project,
            platform,
            triple,
            built,
            options,
            layout,
            &appex_layout,
            &appex_name,
            &sources,
            &staging.join("Widgets.swift"),
            sdk_name,
            deployment_target,
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
    generated: &Path,
    sdk_name: &str,
    deployment_target: &str,
) -> eyre::Result<()> {
    let host = project.host();
    let sdk_path = host
        .run("xcrun", ["--sdk", sdk_name, "--show-sdk-path"])
        .await
        .wrap_err_with(|| format!("xcrun reports no {sdk_name} SDK path"))?;
    let swift_target = widget_swift_target(platform, triple, deployment_target)?;

    let mut source_files = Vec::new();
    let mut entries = fs::read_dir(sources).await?;
    while let Some(entry) = entries.next().await {
        let path = entry?.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("swift") {
            source_files.push(path);
        }
    }
    source_files.sort();
    source_files.push(generated.to_path_buf());

    let executable = appex_layout.executable_file(appex_name);
    let mut args: Vec<std::ffi::OsString> = vec![
        "-emit-executable".into(),
        "-module-name".into(),
        appex_name
            .replace(|c: char| !c.is_alphanumeric(), "_")
            .into(),
        "-sdk".into(),
        sdk_path.trim().into(),
        "-target".into(),
        swift_target.into(),
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
    args.extend(source_files.iter().map(|path| path.as_os_str().into()));

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

    host.run("swiftc", &args)
        .await
        .wrap_err("failed to compile the WidgetKit extension")?;
    Ok(())
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
    let selection = match crate::apple::provisioning::select_development_profile(host, &request)
        .await
    {
        Ok(selection) => selection,
        Err(crate::apple::provisioning::SelectError::NoMatch(first)) => {
            info!("{first}; asking xcodebuild to provision one");
            crate::apple::provisioning::provision_via_xcodebuild(
                host,
                &request,
                &backend_root.join("DerivedData/Provisioning"),
                deployment_target,
            )
            .await?;
            match crate::apple::provisioning::select_development_profile(host, &request).await {
                Ok(selection) => selection,
                Err(crate::apple::provisioning::SelectError::NoMatch(still)) => {
                    bail!(
                        "xcodebuild installed a profile but no installed profile qualifies: {still}"
                    )
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(error) => return Err(error.into()),
    };

    copy_file(
        &selection.path,
        appex_layout.app_path.join("embedded.mobileprovision"),
    )
    .await?;

    let entitlements = crate::apple::provisioning::signing_entitlements(
        &selection.data,
        &request,
        &selection.app_id_prefix,
    )?;
    let appex_name = appex_layout.app_path.file_stem().map_or_else(
        || "Widgets".to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let xcent = appex_layout
        .app_path
        .with_file_name(format!("{appex_name}.xcent"));
    app_bundle::write_entitlements(&xcent, entitlements).await?;
    codesign_path(
        host,
        &appex_layout.app_path,
        &selection.identity,
        Some(&xcent),
        Some(widget_bundle_id),
    )
    .await?;
    Ok(())
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
                "26.0"
            )
            .unwrap(),
            "arm64-apple-macosx26.0"
        );
        assert_eq!(
            widget_swift_target(
                TargetPlatform::MacOS,
                &triple("x86_64-apple-darwin"),
                "26.0"
            )
            .unwrap(),
            "x86_64-apple-macosx26.0"
        );
        assert_eq!(
            widget_swift_target(
                TargetPlatform::IOSSimulator,
                &triple("x86_64-apple-ios"),
                "26.0"
            )
            .unwrap(),
            "x86_64-apple-ios26.0-simulator"
        );
        let error = widget_swift_target(
            TargetPlatform::MacOS,
            &triple("wasm32-unknown-unknown"),
            "26.0",
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
            "26.0",
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
}
