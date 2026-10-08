//! `water preview --platform android`: the preview renders through
//! Hydrolysis inside the preview host APK's instrumentation — device GPU,
//! device fonts, device realizations — with no Kotlin runtime involved.
//!
//! The sequence is push-staged, not streamed: the CLI builds the launcher's
//! preview-mode cdylib for the device's ABI, stages its assets and the run
//! config under `<backend>/android-preview/`, pushes the payload to
//! `/data/local/tmp`, copies it into the host's private files with `run-as`
//! (the shell user cannot write app-private storage), and runs
//! `am instrument -w` — whose return is the completion signal — before
//! reading the produced PNGs back through `exec-out`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eyre::{Context as _, Result, bail};
use smol::fs;
use tracing::info;

use waterui_preview_protocol::run::{PreviewRunConfig, PreviewRunMode};

use crate::android::adb::{Adb, recent_crash_log};
use crate::android::device::{AndroidAbiProvider, AndroidTarget};
use crate::android::platform::{AndroidAbi, AndroidPlatform, resolve_android_build_context};
use crate::android::toolchain::ndk_llvm_tool;
use crate::build::{BuildOptions, BuildProfile, BuildProgress};
use crate::device::Device as _;
use crate::hydrolysis::android::{
    self as hydrolysis_android, PREVIEW_HOST_INSTRUMENTATION, PREVIEW_HOST_PACKAGE,
};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::preview::PreviewSource;
use crate::preview::hydrolysis::{
    HydrolysisPreviewScenario, HydrolysisPreviewTheme, scenario_frame_path, write_preview_bindings,
};
use crate::preview::run::{absolute_output_path, expect_nonempty_output, write_run_config};
use crate::project::Project;
use crate::project_model::assets;
use crate::toolchain::Host;

/// `am instrument -w` is the completion signal; this bound is only the
/// backstop for a wedged instrumentation run, not the render's expected
/// duration.
const PREVIEW_RENDER_DEADLINE: Duration = Duration::from_mins(3);

/// The device-side staging root a push replaces each run.
const DEVICE_TMP_DIR: &str = "/data/local/tmp/waterui-preview";

/// `water preview --platform android` parameters — the
/// [`super::HydrolysisPreviewRequest`] fields minus `platform`, which the
/// device decides.
#[derive(Debug)]
pub struct HydrolysisAndroidPreviewRequest<'a> {
    /// `WaterUI` project directory.
    pub project_path: &'a Path,
    /// Preview view source.
    pub source: PreviewSource<'a>,
    /// Theme package the preview runtime mounts with.
    pub theme: HydrolysisPreviewTheme,
    /// Viewport width in logical units.
    pub width: f32,
    /// Viewport height in logical units.
    pub height: f32,
    /// `sccache` binary used for compilation caching, when available.
    pub sccache_path: Option<PathBuf>,
    /// Sink compile progress is reported to while the preview build runs cargo.
    pub progress: Option<BuildProgress>,
}

/// Render a preview on an Android device through Hydrolysis.
///
/// The device's GPU renders the frame and its system fonts shape the text;
/// the PNG lands in the preview host's private files and is read back into
/// `output_path` — or `scenario.output_dir`/`frame-*ms.png` for a scenario.
///
/// # Errors
/// Returns an error when no device or AVD is usable, the host APK cannot be
/// built or installed, the launcher build or staging fails, the
/// instrumentation reports a failure, or the produced PNG cannot be read
/// back.
pub async fn render_preview_with_hydrolysis_android(
    request: HydrolysisAndroidPreviewRequest<'_>,
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    let host = Host::current();
    let project = crate::hydrolysis::backend::open_ready(request.project_path).await?;
    write_preview_bindings(&project, request.source, request.theme, None).await?;

    let target = AndroidTarget::first_available(&host).await?;
    target.launch(&host).await?;
    let serial = target
        .serial()
        .ok_or_else(|| eyre::eyre!("the Android target has no adb serial after launch"))?
        .to_string();
    let abi = target.android_abi();
    let adb = Adb::locate(&host).await?;

    ensure_host_installed(&project, &host, &adb, &serial).await?;

    // The staged payload the device sees: `lib/` carries the cdylib — and
    // `libc++_shared.so` when the build needs the STL — `resources/` the
    // project's staged assets, and `preview-run.json` the run config.
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let device_dir = backend_path.join("android-preview").join("device");
    let libraries = stage_device_payload(&project, &host, &request, abi, &device_dir).await?;

    // The preview host's private root: `run-as` resolves it inside the app's
    // data directory, which `am instrument` then runs under.
    let files_root = adb
        .run_as(
            &host,
            &serial,
            PREVIEW_HOST_PACKAGE,
            &["pwd"],
            Duration::from_secs(30),
        )
        .await?;
    let device_root = format!("{}/files/waterui-preview", files_root.trim_end());

    let mode = scenario.map_or_else(
        || PreviewRunMode::Image {
            output: PathBuf::from(format!("{device_root}/out/preview.png")),
        },
        |scenario| PreviewRunMode::Scenario {
            output_dir: PathBuf::from(format!("{device_root}/out/scenario")),
            captures_ms: scenario.captures_ms.clone(),
            events: scenario.events.clone(),
        },
    );
    write_run_config(
        &device_dir,
        &PreviewRunConfig {
            width: request.width,
            height: request.height,
            mode,
        },
    )
    .await?;

    push_payload(&host, &adb, &serial, &device_dir).await?;
    run_instrumentation(&host, &adb, &serial, &device_root, &libraries).await?;
    pull_outputs(&host, &adb, &serial, &device_root, output_path, scenario).await
}

/// Build — or reuse — the preview host APK and install it when the device's
/// copy carries a different `versionCode`.
async fn ensure_host_installed(
    project: &Project,
    host: &Host,
    adb: &Adb,
    serial: &str,
) -> Result<()> {
    let (apk, version_code) = hydrolysis_android::ensure_preview_host_apk(project, host).await?;
    if adb
        .installed_version_code(host, serial, PREVIEW_HOST_PACKAGE, Duration::from_secs(30))
        .await?
        != Some(version_code)
    {
        info!("Installing the hydrolysis preview host ({version_code})");
        adb.install_any_version(host, serial, &apk, Duration::from_secs(120))
            .await?;
    }
    Ok(())
}

/// Build the launcher's preview-mode cdylib, strip it in place, stage the
/// project's assets, and answer the library file names the payload carries —
/// `libc++_shared.so` first, the app cdylib after it.
async fn stage_device_payload(
    project: &Project,
    host: &Host,
    request: &HydrolysisAndroidPreviewRequest<'_>,
    abi: AndroidAbi,
    device_dir: &Path,
) -> Result<Vec<String>> {
    if device_dir.exists() {
        fs::remove_dir_all(device_dir).await?;
    }
    let lib_dir = device_dir.join("lib");
    fs::create_dir_all(&lib_dir).await?;

    let mut options =
        BuildOptions::development(BuildProfile::Debug).with_output_dir(lib_dir.clone());
    if let Some(sccache_path) = request.sccache_path.clone() {
        options = options.with_sccache(sccache_path);
    }
    if let Some(progress) = request.progress.clone() {
        options = options.with_progress(progress);
    }
    let built = hydrolysis_android::build_with_features(
        project,
        host,
        abi,
        options,
        &["waterui-preview-mode"],
    )
    .await?;

    // Strip debug info in place: the cdylib carries a full desktop-sized
    // symbol set that only bloats the push.
    let triple = AndroidPlatform::new(abi).triple();
    let min_api_level = project
        .resolved_framework()
        .await?
        .android_min_api_level()?;
    let context = resolve_android_build_context(host, abi, &triple, min_api_level).await?;
    let app_library = lib_dir.join(
        built
            .artifact
            .file_name()
            .ok_or_else(|| eyre::eyre!("the built launcher artifact has no file name"))?,
    );
    let strip = ndk_llvm_tool(&context.ndk_path, "llvm-strip").ok_or_else(|| {
        eyre::eyre!(
            "the NDK at {} ships no llvm-strip under toolchains/llvm/prebuilt",
            context.ndk_path.display()
        )
    })?;
    host.run(
        &strip,
        [OsStr::new("--strip-debug"), app_library.as_os_str()],
    )
    .await
    .map_err(|error| eyre::eyre!("llvm-strip on {} failed: {error}", app_library.display()))?;

    // Asset mounts ship as the library layout's `waterui_assets` directory —
    // stage through the shared planner, then place it where the run config's
    // assets root names it.
    let stage_dir = project
        .backend_path::<HydrolysisBackend>()
        .join("android-preview")
        .join("stage");
    assets::stage_project_assets_for_android_library(
        project,
        &stage_dir,
        &built.app_symbols()?,
        false,
    )
    .await?;
    let staged_assets = stage_dir.join("src/main/assets/waterui_assets");
    let assets_dest = device_dir.join("resources").join("waterui_assets");
    if staged_assets.is_dir() {
        fs::create_dir_all(device_dir.join("resources")).await?;
        fs::rename(&staged_assets, &assets_dest)
            .await
            .wrap_err_with(|| {
                format!(
                    "failed to move {} to {}",
                    staged_assets.display(),
                    assets_dest.display()
                )
            })?;
    }

    // `libc++_shared.so` loads first — the cdylib depends on it — and the
    // instrumentation `System.load`s each in order.
    let mut libraries = Vec::new();
    let mut names = Vec::new();
    let mut entries = fs::read_dir(&lib_dir).await?;
    while let Some(entry) = smol::stream::StreamExt::next(&mut entries).await {
        let entry = entry?;
        if entry.path().extension() == Some(OsStr::new("so")) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names.retain(|name| {
        if name == "libc++_shared.so" {
            libraries.push(name.clone());
            false
        } else {
            true
        }
    });
    libraries.extend(names);
    Ok(libraries)
}

/// Push the staged payload to the device and copy it into the preview host's
/// private files — `chmod a-w` on `lib/` satisfies the linker's read-only
/// `System.load` requirement, and `out/` takes the render's output.
async fn push_payload(host: &Host, adb: &Adb, serial: &str, device_dir: &Path) -> Result<()> {
    adb.shell_run(
        host,
        serial,
        &["rm", "-rf", DEVICE_TMP_DIR],
        Duration::from_secs(30),
    )
    .await?;
    adb.push(
        host,
        serial,
        &device_dir.join("."),
        DEVICE_TMP_DIR,
        Duration::from_secs(120),
    )
    .await?;
    // `chmod a-w` touches the libraries only — a write-protected `lib`
    // directory itself would make this same `rm -rf` fail on the next run
    // (unlinking a file needs write on the directory, not the file).
    adb.run_as(
        host,
        serial,
        PREVIEW_HOST_PACKAGE,
        &[
            "sh",
            "-c",
            "rm -rf files/waterui-preview && mkdir -p files && \
             cp -R /data/local/tmp/waterui-preview files/waterui-preview && \
             chmod a-w files/waterui-preview/lib/* && mkdir -p files/waterui-preview/out",
        ],
        Duration::from_secs(60),
    )
    .await?;
    Ok(())
}

/// Run the preview instrumentation: `am instrument -w` returns when the run
/// finishes, and `-r` streams its result bundle. A failure message is the
/// bundle's `error=`/`shortMsg=`/`longMsg=` plus the logcat tail the run
/// left behind.
async fn run_instrumentation(
    host: &Host,
    adb: &Adb,
    serial: &str,
    device_root: &str,
    libraries: &[String],
) -> Result<()> {
    let device_libraries: Vec<String> = libraries
        .iter()
        .map(|name| format!("{device_root}/lib/{name}"))
        .collect();
    let words = instrument_words(
        &device_libraries,
        &format!("{device_root}/preview-run.json"),
        &format!("{device_root}/resources/waterui_assets"),
    );
    let output = adb
        .shell(
            host,
            serial,
            &words.iter().map(String::as_str).collect::<Vec<_>>(),
            PREVIEW_RENDER_DEADLINE,
        )
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Err(message) = parse_instrumentation_output(&stdout) {
        let crash_log = recent_crash_log(host, adb, serial).await;
        bail!("hydrolysis android preview failed: {message}\n\n=== Crash Log ===\n{crash_log}");
    }
    Ok(())
}

/// Read the produced PNGs back out of the preview host's files and write
/// them to the local outputs.
async fn pull_outputs(
    host: &Host,
    adb: &Adb,
    serial: &str,
    device_root: &str,
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    let cat = |remote: String| async move {
        adb.exec_out_run_as_cat(
            host,
            serial,
            PREVIEW_HOST_PACKAGE,
            &remote,
            Duration::from_secs(60),
        )
        .await
    };
    if let Some(scenario) = scenario {
        fs::create_dir_all(&scenario.output_dir).await?;
        for capture_ms in &scenario.captures_ms {
            let remote = scenario_frame_path(
                &PathBuf::from(format!("{device_root}/out/scenario")),
                *capture_ms,
            );
            let local = scenario_frame_path(&scenario.output_dir, *capture_ms);
            let bytes = cat(remote.to_string_lossy().into_owned()).await?;
            fs::write(&local, &bytes).await?;
            expect_nonempty_output(&local, "scenario frame").await?;
        }
        return Ok(());
    }
    let output_path = absolute_output_path(output_path)?;
    let bytes = cat(format!("{device_root}/out/preview.png")).await?;
    fs::write(&output_path, &bytes).await?;
    expect_nonempty_output(&output_path, "output").await
}

/// The `adb shell` word list for one preview instrumentation run:
/// `am instrument -w -r -e libraries <a:b> -e runConfig <p> -e assetsRoot <p>`.
fn instrument_words(libraries: &[String], run_config: &str, assets_root: &str) -> Vec<String> {
    vec![
        "am".to_string(),
        "instrument".to_string(),
        "-w".to_string(),
        "-r".to_string(),
        "-e".to_string(),
        "libraries".to_string(),
        libraries.join(":"),
        "-e".to_string(),
        "runConfig".to_string(),
        run_config.to_string(),
        "-e".to_string(),
        "assetsRoot".to_string(),
        assets_root.to_string(),
        PREVIEW_HOST_INSTRUMENTATION.to_string(),
    ]
}

/// The instrumentation's verdict: `INSTRUMENTATION_CODE: -1` is
/// `RESULT_OK`. Anything else is a failure — the `error=` result bundle value
/// (multi-line, up to the next `INSTRUMENTATION_` line), else `shortMsg=` or
/// `longMsg=`, else the raw output, which covers `INSTRUMENTATION_FAILED` and
/// `Unable to find instrumentation info`.
fn parse_instrumentation_output(stdout: &str) -> Result<(), String> {
    let lines: Vec<&str> = stdout.lines().collect();
    if lines
        .iter()
        .any(|line| line.trim() == "INSTRUMENTATION_CODE: -1")
    {
        return Ok(());
    }
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        for prefix in ["INSTRUMENTATION_RESULT: ", "INSTRUMENTATION_STATUS: "] {
            if let Some(value) = trimmed
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix("error="))
            {
                let mut message = value.to_string();
                for next in &lines[index + 1..] {
                    if next.starts_with("INSTRUMENTATION_") {
                        break;
                    }
                    message.push('\n');
                    message.push_str(next);
                }
                return Err(message);
            }
        }
    }
    for key in ["shortMsg=", "longMsg="] {
        for line in &lines {
            let trimmed = line.trim();
            for prefix in ["INSTRUMENTATION_RESULT: ", "INSTRUMENTATION_STATUS: "] {
                if let Some(value) = trimmed
                    .strip_prefix(prefix)
                    .and_then(|rest| rest.strip_prefix(key))
                {
                    return Err(value.to_string());
                }
            }
        }
    }
    Err(stdout.to_string())
}

#[cfg(test)]
mod tests {
    use super::{instrument_words, parse_instrumentation_output};
    use crate::hydrolysis::android::{PREVIEW_HOST_INSTRUMENTATION, preview_host_version_code};

    #[test]
    fn instrument_words_are_the_cli_contract() {
        let words = instrument_words(
            &[
                "/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/lib/libc++_shared.so"
                    .to_string(),
                "/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/lib/libapp_lib.so"
                    .to_string(),
            ],
            "/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/preview-run.json",
            "/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/resources/waterui_assets",
        );
        assert_eq!(
            words,
            [
                "am",
                "instrument",
                "-w",
                "-r",
                "-e",
                "libraries",
                "/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/lib/libc++_shared.so:/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/lib/libapp_lib.so",
                "-e",
                "runConfig",
                "/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/preview-run.json",
                "-e",
                "assetsRoot",
                "/data/user/0/dev.waterui.hydrolysis.preview/files/waterui-preview/resources/waterui_assets",
                PREVIEW_HOST_INSTRUMENTATION,
            ]
        );
    }

    #[test]
    fn instrumentation_ok_is_instrumentation_code_minus_one() {
        let output = "INSTRUMENTATION_RESULT: stream=\n\nINSTRUMENTATION_CODE: -1\n";
        assert_eq!(parse_instrumentation_output(output), Ok(()));
    }

    #[test]
    fn instrumentation_failure_keeps_the_whole_error_value() {
        let output = "INSTRUMENTATION_RESULT: error=java.lang.IllegalStateException: no preview\n\tat dev.waterui.hydrolysis.preview.PreviewBridge.run(PreviewBridge.kt:20)\n\tat dev.waterui.hydrolysis.preview.HydrolysisPreviewInstrumentation.onStart(HydrolysisPreviewInstrumentation.kt:55)\nINSTRUMENTATION_CODE: 0\n";
        let error = parse_instrumentation_output(output).expect_err("a failure");
        assert!(
            error.starts_with("java.lang.IllegalStateException"),
            "{error}"
        );
        assert!(error.contains("PreviewBridge.kt:20"), "{error}");
        assert!(
            error.contains("HydrolysisPreviewInstrumentation.kt:55"),
            "{error}"
        );
    }

    #[test]
    fn a_crashed_instrumentation_reports_its_short_message() {
        let output = "INSTRUMENTATION_STATUS: class=dev.waterui.hydrolysis.preview.HydrolysisPreviewInstrumentation\nINSTRUMENTATION_STATUS: shortMsg=Process crashed.\nINSTRUMENTATION_STATUS_CODE: 0\n";
        assert_eq!(
            parse_instrumentation_output(output),
            Err("Process crashed.".to_string())
        );
    }

    #[test]
    fn a_missing_instrumentation_reports_the_raw_output() {
        let output = "INSTRUMENTATION_STATUS: Error=Unable to find instrumentation info for: ComponentInfo{dev.waterui.hydrolysis.preview/dev.waterui.hydrolysis.preview.HydrolysisPreviewInstrumentation}\nINSTRUMENTATION_FAILED: dev.waterui.hydrolysis.preview/dev.waterui.hydrolysis.preview.HydrolysisPreviewInstrumentation\n";
        let error = parse_instrumentation_output(output).expect_err("a failure");
        assert!(
            error.contains("Unable to find instrumentation info"),
            "{error}"
        );
        assert!(error.contains("INSTRUMENTATION_FAILED"), "{error}");
    }

    #[test]
    fn the_version_code_is_deterministic_and_bounded() {
        let fingerprint = "a".repeat(64);
        let code = preview_host_version_code(&fingerprint);
        assert_eq!(code, preview_host_version_code(&fingerprint));
        assert!((1..=2_000_000_000).contains(&code));
        assert_ne!(
            preview_host_version_code(&"a".repeat(64)),
            preview_host_version_code(&"b".repeat(64)),
            "distinct fingerprints project to distinct codes almost surely"
        );
    }
}
