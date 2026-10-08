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
//! reading the produced PNGs back through `adb shell -T`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eyre::{Context as _, Result, bail};
use futures_util::{StreamExt as _, TryStreamExt as _};
use smol::fs;
use tracing::info;

use waterui_preview_protocol::run::{PreviewRunConfig, PreviewRunMode};

use crate::android::adb::{Adb, recent_crash_log};
use crate::android::device::{AndroidAbiProvider, AndroidTarget};
use crate::android::platform::{AndroidAbi, ndk_llvm_tool};
use crate::build::{BuildOptions, BuildProfile};
use crate::device::Device as _;
use crate::hydrolysis::android::{
    self as hydrolysis_android, PREVIEW_HOST_INSTRUMENTATION, PREVIEW_HOST_PACKAGE,
};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::preview::hydrolysis::{
    HydrolysisPreviewRequest, HydrolysisPreviewScenario, scenario_frame_path,
    write_preview_bindings,
};
use crate::preview::run::{expect_nonempty_output, write_run_config};
use crate::project::Project;
use crate::project_model::assets;
use crate::toolchain::Host;

/// `am instrument -w` is the completion signal; this bound is only the
/// backstop for a wedged instrumentation run, not the render's expected
/// duration.
const PREVIEW_RENDER_DEADLINE: Duration = Duration::from_mins(3);

/// The shell-side staging root a run pushes into — each run stages under
/// its own id beneath it.
const DEVICE_TMP_ROOT: &str = "/data/local/tmp/waterui-preview";

/// The same per-run root inside the preview host's `filesDir` — the
/// instrumentation extras are paths relative to `filesDir` itself.
const FILES_PREVIEW_DIR: &str = "waterui-preview";

/// Scenario frame pulls overlap the shell round trips a `run-as cat` spends
/// per frame — bounded so a wide capture set cannot flood the transport.
const SCENARIO_PULL_CONCURRENCY: usize = 4;

/// The PNG magic a frame must carry — a failed remote `cat` would otherwise
/// write its own error text out as the image.
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

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
    request: &HydrolysisPreviewRequest<'_>,
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

    // Each run stages under its own id in both roots, so two previews on one
    // device cannot clobber each other's payload.
    let run_id = format!(
        "{:x}-{:x}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |stamp| stamp.as_nanos())
    );
    // `run_dir` is the payload root relative to the host's `filesDir`;
    // `files_dir` is the same directory as `run-as` sees it from the app
    // data dir.
    let run_dir = format!("{FILES_PREVIEW_DIR}/{run_id}");
    let files_dir = format!("files/{run_dir}");
    let tmp_dir = format!("{DEVICE_TMP_ROOT}/{run_id}");

    // Installing the host APK is independent of the local payload — query
    // and install while the launcher builds and stages.
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let device_dir = backend_path.join("android-preview").join("device");
    let ((), payload) = futures_util::try_join!(
        ensure_host_installed(&project, &host, &adb, &serial),
        stage_device_payload(&project, &host, request, abi, &device_dir),
    )?;

    // Output paths in the run config are relative: the runtime resolves
    // them against the config's own directory, so nothing host-side needs
    // the device's absolute layout.
    let mode = scenario.map_or_else(
        || PreviewRunMode::Image {
            output: PathBuf::from("out/preview.png"),
        },
        |scenario| PreviewRunMode::Scenario {
            output_dir: PathBuf::from("out/scenario"),
            captures_ms: scenario.captures_ms.clone(),
            events: scenario.events.clone(),
        },
    );
    let config_path = write_run_config(
        &device_dir,
        &PreviewRunConfig {
            width: request.width,
            height: request.height,
            mode,
        },
    )
    .await?;
    let config_name = config_path
        .file_name()
        .ok_or_else(|| {
            eyre::eyre!(
                "the run config path {} has no file name",
                config_path.display()
            )
        })?
        .to_string_lossy()
        .into_owned();

    push_payload(&host, &adb, &serial, &device_dir, &tmp_dir, &files_dir).await?;

    // Bound the crash log to this run: the device clock's `logcat -T`
    // timestamp captured before the instrumentation starts.
    let since = adb
        .shell_run(
            &host,
            &serial,
            &["date", "+%m-%d %H:%M:%S.000"],
            Duration::from_secs(10),
        )
        .await
        .ok()
        .map(|stamp| stamp.trim().to_string());

    let run = DeviceRun {
        run_dir: &run_dir,
        config_name: &config_name,
        libraries: &payload.libraries,
        assets_root_name: payload.assets_root_name.as_deref(),
    };
    let result = async {
        run_instrumentation(&host, &adb, &serial, &run, since.as_deref()).await?;
        pull_outputs(&host, &adb, &serial, &run_dir, output_path, scenario).await
    }
    .await;
    cleanup_payload(&host, &adb, &serial, &files_dir, &tmp_dir).await;
    result
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
    install_host_if_needed(host, adb, serial, &apk, version_code).await?;
    Ok(())
}

/// Install `apk` when the device's installed `versionCode` differs from
/// `version_code` — including a missing install — and skip the round trip
/// otherwise. Returns whether it installed.
async fn install_host_if_needed(
    host: &Host,
    adb: &Adb,
    serial: &str,
    apk: &Path,
    version_code: u32,
) -> Result<bool> {
    let installed = adb
        .installed_version_code(host, serial, PREVIEW_HOST_PACKAGE, Duration::from_secs(30))
        .await?;
    if installed == Some(version_code) {
        return Ok(false);
    }
    info!("Installing the hydrolysis preview host ({version_code})");
    adb.install_any_version(host, serial, apk, Duration::from_secs(120))
        .await?;
    Ok(true)
}

/// The staged payload: the library file names in load order plus the staged
/// asset bundle's directory name under `resources/`.
struct StagedPayload {
    /// Staged `.so` file names in `System.load` order.
    libraries: Vec<String>,
    /// The staged asset bundle's directory name inside `resources/`, when
    /// the project staged one.
    assets_root_name: Option<String>,
}

/// Build the launcher's preview-mode cdylib, strip it in place, stage the
/// project's assets, and answer the payload's library and asset names.
async fn stage_device_payload(
    project: &Project,
    host: &Host,
    request: &HydrolysisPreviewRequest<'_>,
    abi: AndroidAbi,
    device_dir: &Path,
) -> Result<StagedPayload> {
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
    let build = hydrolysis_android::build_with_features(
        project,
        host,
        abi,
        options,
        &["waterui-preview-mode"],
    )
    .await?;

    // Strip debug info in place: the cdylib carries a full desktop-sized
    // symbol set that only bloats the push. The build hands back the
    // context it resolved, so the strip runs under the same NDK without a
    // second resolve.
    let app_library = lib_dir.join(
        build
            .staged_libraries
            .last()
            .ok_or_else(|| eyre::eyre!("the launcher build staged no libraries"))?,
    );
    let strip = smol::unblock({
        let ndk_path = build.context.ndk_path.clone();
        move || ndk_llvm_tool(&ndk_path, "llvm-strip")
    })
    .await?;
    host.run(
        &strip,
        [OsStr::new("--strip-debug"), app_library.as_os_str()],
    )
    .await
    .map_err(|error| eyre::eyre!("llvm-strip on {} failed: {error}", app_library.display()))?;

    // Asset mounts ship as the library layout's `waterui_assets` directory —
    // stage through the shared planner, then place it under `resources/`
    // where the run's `assetsRoot` extra names it.
    let stage_dir = project
        .backend_path::<HydrolysisBackend>()
        .join("android-preview")
        .join("stage");
    let (_manifest, staged_assets) = assets::stage_project_assets_for_android_library(
        project,
        &stage_dir,
        &build.built.app_symbols()?,
        false,
    )
    .await?;
    let assets_root_name = if staged_assets.is_dir() {
        let name = staged_assets
            .file_name()
            .ok_or_else(|| {
                eyre::eyre!(
                    "the staged assets dir {} has no name",
                    staged_assets.display()
                )
            })?
            .to_string_lossy()
            .into_owned();
        fs::create_dir_all(device_dir.join("resources")).await?;
        let assets_dest = device_dir.join("resources").join(&name);
        fs::rename(&staged_assets, &assets_dest)
            .await
            .wrap_err_with(|| {
                format!(
                    "failed to move {} to {}",
                    staged_assets.display(),
                    assets_dest.display()
                )
            })?;
        Some(name)
    } else {
        None
    };

    Ok(StagedPayload {
        libraries: build.staged_libraries,
        assets_root_name,
    })
}

/// Push the staged payload to the device and copy it into the preview
/// host's private files — `chmod a-w` on the libraries satisfies the
/// linker's read-only `System.load` requirement without write-protecting
/// `lib/` itself (unlinking needs write on the directory, not the file),
/// and `out/` takes the render's output.
async fn push_payload(
    host: &Host,
    adb: &Adb,
    serial: &str,
    device_dir: &Path,
    tmp_dir: &str,
    files_dir: &str,
) -> Result<()> {
    adb.shell_run(
        host,
        serial,
        &["rm", "-rf", tmp_dir],
        Duration::from_secs(30),
    )
    .await?;
    adb.shell_run(
        host,
        serial,
        &["mkdir", "-p", tmp_dir],
        Duration::from_secs(30),
    )
    .await?;
    adb.push(
        host,
        serial,
        &device_dir.join("."),
        tmp_dir,
        Duration::from_secs(120),
    )
    .await?;
    // The script's positions keep the two paths off the command template:
    // `$1` is the `files/`-relative payload root inside the host's private
    // data, `$2` the shell-side staging directory it is copied from.
    adb.run_as(
        host,
        serial,
        PREVIEW_HOST_PACKAGE,
        &[
            "sh",
            "-c",
            "rm -rf \"$1\" && mkdir -p \"$(dirname \"$1\")\" && cp -R \"$2\" \"$1\" && \
             chmod a-w \"$1\"/lib/* && mkdir -p \"$1\"/out",
            "sh",
            files_dir,
            tmp_dir,
        ],
        Duration::from_secs(60),
    )
    .await?;
    Ok(())
}

/// Remove the run's payload from both staging roots — the shell-side copy
/// under `/data/local/tmp` and the private copy under `files/`. Best-effort:
/// a cleanup failure warns rather than masking the run's own verdict.
async fn cleanup_payload(host: &Host, adb: &Adb, serial: &str, files_dir: &str, tmp_dir: &str) {
    if let Err(error) = adb
        .run_as(
            host,
            serial,
            PREVIEW_HOST_PACKAGE,
            &["rm", "-rf", files_dir],
            Duration::from_secs(30),
        )
        .await
    {
        tracing::warn!("failed to remove the preview payload {files_dir}: {error}");
    }
    if let Err(error) = adb
        .shell_run(
            host,
            serial,
            &["rm", "-rf", tmp_dir],
            Duration::from_secs(30),
        )
        .await
    {
        tracing::warn!("failed to remove the preview staging dir {tmp_dir}: {error}");
    }
}

/// The device-side layout of a pushed run — every path is relative to the
/// preview host's `filesDir` and resolved there by the instrumentation.
struct DeviceRun<'a> {
    /// The run's own `waterui-preview/<id>` directory under `filesDir`.
    run_dir: &'a str,
    /// The run config's file name inside `run_dir`.
    config_name: &'a str,
    /// Staged library file names in `System.load` order.
    libraries: &'a [String],
    /// The staged asset bundle's directory name under `run_dir/resources/`,
    /// when the project staged one.
    assets_root_name: Option<&'a str>,
}

/// Run the preview instrumentation: `am instrument -w` returns when the run
/// finishes, and `-r` streams its result bundle. A failure reports the
/// bundle's `error=`/`shortMsg=`/`longMsg=`, the adb status and stderr, and
/// the logcat tail this run left behind (`since` bounds it to the run).
async fn run_instrumentation(
    host: &Host,
    adb: &Adb,
    serial: &str,
    run: &DeviceRun<'_>,
    since: Option<&str>,
) -> Result<()> {
    let run_dir = run.run_dir;
    let device_libraries: Vec<String> = run
        .libraries
        .iter()
        .map(|name| format!("{run_dir}/lib/{name}"))
        .collect();
    let assets_root = run
        .assets_root_name
        .map(|name| format!("{run_dir}/resources/{name}"));
    let words = instrument_words(
        &device_libraries,
        &format!("{run_dir}/{}", run.config_name),
        assets_root.as_deref(),
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
        let crash_log = recent_crash_log(host, adb, serial, since).await;
        bail!(
            "hydrolysis android preview failed: {message}\n\n\
             `am instrument` exited with status {} — stderr:\n{}\n\n\
             === Crash Log ===\n{crash_log}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// The `am instrument` words the run launches with. Libraries, run config
/// and the assets root are paths relative to the host's `filesDir` — the
/// instrumentation resolves them against it, so the device never sees an
/// absolute private path it could be wrong about. `assets_root` is absent
/// entirely when the project staged no assets.
fn instrument_words(
    libraries: &[String],
    run_config: &str,
    assets_root: Option<&str>,
) -> Vec<String> {
    let mut words = vec![
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
    ];
    if let Some(assets_root) = assets_root {
        words.extend([
            "-e".to_string(),
            "assetsRoot".to_string(),
            assets_root.to_string(),
        ]);
    }
    words.push(PREVIEW_HOST_INSTRUMENTATION.to_string());
    words
}

/// Read the run's rendered output back and write it locally — the image at
/// `out/preview.png`, or every captured scenario frame under
/// `out/scenario/` at a bounded overlap of [`SCENARIO_PULL_CONCURRENCY`].
/// Each pull verifies the PNG signature before writing, so a failed remote
/// `cat` can never be saved as an image.
async fn pull_outputs(
    host: &Host,
    adb: &Adb,
    serial: &str,
    run_dir: &str,
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    /// Read one rendered PNG out of the host's private files and write it
    /// locally once its signature checks out.
    async fn pull_png(
        host: &Host,
        adb: &Adb,
        serial: &str,
        remote: &str,
        local: &Path,
    ) -> Result<()> {
        let bytes = adb
            .run_as_cat(
                host,
                serial,
                PREVIEW_HOST_PACKAGE,
                remote,
                Duration::from_secs(60),
            )
            .await?;
        if !bytes.starts_with(PNG_SIGNATURE) {
            bail!(
                "`{remote}` did not read back as a PNG ({} bytes starting {:02x?})",
                bytes.len(),
                &bytes[..bytes.len().min(8)]
            );
        }
        fs::write(local, &bytes).await?;
        expect_nonempty_output(local, "output").await
    }

    /// Pull `remote_name` — a path relative to the run dir — out of the
    /// host's `files/` tree into `local`.
    async fn pull_relative(
        host: &Host,
        adb: &Adb,
        serial: &str,
        run_dir: &str,
        remote_name: &str,
        local: &Path,
    ) -> Result<()> {
        // `run-as` sees `files/` from the app's data dir.
        pull_png(
            host,
            adb,
            serial,
            &format!("files/{run_dir}/{remote_name}"),
            local,
        )
        .await
    }

    if let Some(scenario) = scenario {
        if let Some(parent) = scenario.output_dir.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::create_dir_all(&scenario.output_dir).await?;
        futures_util::stream::iter(scenario.captures_ms.iter().copied().map(|capture_ms| {
            let remote = format!("out/scenario/frame-{capture_ms:04}ms.png");
            let local = scenario_frame_path(&scenario.output_dir, capture_ms);
            async move { pull_relative(host, adb, serial, run_dir, &remote, &local).await }
        }))
        .buffer_unordered(SCENARIO_PULL_CONCURRENCY)
        .try_collect::<()>()
        .await?;
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    pull_relative(host, adb, serial, run_dir, "out/preview.png", output_path).await
}

/// Parse `am instrument -r` output: `-1` means OK, any other code or an
/// `error=`/`shortMsg=`/`longMsg=` in the bundle is the failure text.
fn parse_instrumentation_output(stdout: &str) -> Result<(), String> {
    let mut code = None;
    let mut details = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("INSTRUMENTATION_CODE:") {
            code = rest.trim().parse::<i32>().ok();
        } else if let Some(rest) = line.strip_prefix("INSTRUMENTATION_RESULT:") {
            details.push(rest.trim().to_string());
        }
    }
    match code {
        Some(-1) if details.is_empty() || details.iter().all(|line| line.starts_with("time=")) => {
            Ok(())
        }
        Some(other) => Err(format!(
            "instrumentation reported code {other}: {}",
            details.join("; ")
        )),
        None => Err(format!("instrumentation produced no result code: {stdout}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::testing::TestMachine;

    #[test]
    fn instrumentation_output_ok_on_code_minus_one() {
        let stdout = "INSTRUMENTATION_STATUS: stream=\nINSTRUMENTATION_RESULT: time=0.01\nINSTRUMENTATION_CODE: -1\n";
        parse_instrumentation_output(stdout).expect("code -1 is success");
    }

    #[test]
    fn instrumentation_output_fails_on_error_bundle() {
        let stdout = "INSTRUMENTATION_RESULT: error=preview exploded\nINSTRUMENTATION_CODE: 0\n";
        let error = parse_instrumentation_output(stdout).expect_err("code 0 must fail");
        assert!(error.contains("preview exploded"), "{error}");
    }

    /// The fake `adb` logs every invocation's argv to `WATERUI_FAKE_ADB_LOG`;
    /// canned replies come from response files.
    fn adb_test_machine() -> (TestMachine, Host, PathBuf) {
        let machine = TestMachine::new();
        let sdk = machine.install_android_sdk();
        machine.install_adb();
        let log = machine.root().join("adb-argv.log");
        let host = machine.host([
            ("ANDROID_SDK_ROOT", sdk.as_os_str()),
            ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
        ]);
        (machine, host, log)
    }

    /// The logged adb argv — one invocation per line.
    fn adb_argv(log: &Path) -> String {
        std::fs::read_to_string(log).expect("read the fake adb argv log")
    }

    /// A staged payload dir a `push_payload` can send: `lib/` plus a run
    /// config.
    fn staged_payload(machine: &TestMachine) -> PathBuf {
        let device_dir = machine.dir("device");
        std::fs::create_dir_all(device_dir.join("lib")).expect("lib dir");
        std::fs::write(device_dir.join("lib/libx.so"), b"\x7fELF").expect("lib");
        std::fs::write(device_dir.join("preview-run.json"), b"{}").expect("config");
        device_dir
    }

    #[test]
    #[cfg(unix)]
    fn the_pipeline_pushes_stages_instruments_and_pulls_in_order() {
        let (machine, host, log) = adb_test_machine();
        machine.respond(
            "ADB_AM_INSTRUMENT",
            "INSTRUMENTATION_STATUS: stream=\nINSTRUMENTATION_CODE: -1\n",
        );
        // A PNG signature + filler — raw bytes, not a UTF-8 string.
        std::fs::write(
            machine.responses().join("ADB_CAT"),
            b"\x89PNG\r\n\x1a\nfake-frame-bytes",
        )
        .expect("stage the canned cat");
        let device_dir = staged_payload(&machine);
        let out = machine.root().join("preview.png");
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");

        smol::block_on(async {
            push_payload(
                &host,
                &adb,
                "serial",
                &device_dir,
                "/data/local/tmp/waterui-preview/run-1",
                "files/waterui-preview/run-1",
            )
            .await
            .expect("push_payload");
            run_instrumentation(
                &host,
                &adb,
                "serial",
                &DeviceRun {
                    run_dir: "waterui-preview/run-1",
                    config_name: "preview-run.json",
                    libraries: &["libx.so".to_string()],
                    assets_root_name: Some("waterui_assets"),
                },
                None,
            )
            .await
            .expect("instrumentation succeeds");
            pull_outputs(&host, &adb, "serial", "waterui-preview/run-1", &out, None)
                .await
                .expect("pull_outputs");
        });

        let argv = adb_argv(&log);
        let push = argv.find("push").expect("push ran");
        let run_as = argv.find("run-as").expect("run-as ran");
        let instrument = argv.find("am instrument").expect("instrument ran");
        let cat = argv.rfind("cat files/").expect("cat ran");
        assert!(
            push < run_as && run_as < instrument && instrument < cat,
            "push -> run-as -> instrument -> pull order: {argv}"
        );
        assert!(
            out.is_file()
                && std::fs::read(&out)
                    .expect("read")
                    .starts_with(PNG_SIGNATURE),
            "the pulled PNG landed"
        );
    }

    #[test]
    #[cfg(unix)]
    fn an_equal_installed_version_code_skips_the_reinstall() {
        let (machine, host, log) = adb_test_machine();
        machine.respond(
            "ADB_PM_PACKAGES",
            "package:dev.waterui.hydrolysis.preview versionCode:7",
        );
        let apk = machine.file("host.apk", "apk");
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
        let installed =
            smol::block_on(async { install_host_if_needed(&host, &adb, "serial", &apk, 7).await })
                .expect("version query");
        assert!(!installed);
        assert!(!adb_argv(&log).contains("install -r"), "no install ran");
    }

    #[test]
    #[cfg(unix)]
    fn a_different_or_absent_version_code_reinstalls() {
        for (response, version_code) in [
            ("package:dev.waterui.hydrolysis.preview versionCode:6", 7u32),
            ("", 7u32),
        ] {
            let (machine, host, log) = adb_test_machine();
            machine.respond("ADB_PM_PACKAGES", response);
            let apk = machine.file("host.apk", "apk");
            let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
            let installed = smol::block_on(async {
                install_host_if_needed(&host, &adb, "serial", &apk, version_code).await
            })
            .expect("install decision");
            assert!(installed, "expected an install for `{response}`");
            assert!(
                adb_argv(&log).contains("install -r"),
                "an install ran for `{response}`"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn a_nonzero_run_as_surfaces() {
        let (machine, host, _log) = adb_test_machine();
        let device_dir = staged_payload(&machine);
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
        // The run-as copy fails the way a missing package would.
        let host = machine.host([
            (
                "ANDROID_SDK_ROOT",
                machine.install_android_sdk().as_os_str(),
            ),
            ("WATERUI_FAKE_ADB_RUN_AS_STATUS", "7".as_ref()),
        ]);
        let error = smol::block_on(async {
            push_payload(
                &host,
                &adb,
                "serial",
                &device_dir,
                "/data/local/tmp/waterui-preview/run-1",
                "files/waterui-preview/run-1",
            )
            .await
            .expect_err("a non-zero run-as must fail")
        });
        assert!(error.to_string().contains("run-as"), "{error}");
    }

    #[test]
    #[cfg(unix)]
    fn an_instrumentation_failure_surfaces_with_its_bundle() {
        let (machine, host, _log) = adb_test_machine();
        machine.respond(
            "ADB_AM_INSTRUMENT",
            "INSTRUMENTATION_RESULT: error=preview exploded\nINSTRUMENTATION_CODE: 0\n",
        );
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
        let error = smol::block_on(async {
            run_instrumentation(
                &host,
                &adb,
                "serial",
                &DeviceRun {
                    run_dir: "waterui-preview/run-1",
                    config_name: "preview-run.json",
                    libraries: &[],
                    assets_root_name: None,
                },
                None,
            )
            .await
            .expect_err("a failed instrumentation must surface")
        });
        assert!(error.to_string().contains("preview exploded"), "{error}");
    }

    #[test]
    #[cfg(unix)]
    fn an_adb_timeout_names_the_invocation() {
        let (machine, host, _log) = adb_test_machine();
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
        let host = machine.host([("WATERUI_FAKE_ADB_HANG", "1")]);
        let error = smol::block_on(async {
            adb.shell(
                &host,
                "serial",
                &["am", "instrument", "-w", "x/y"],
                Duration::from_millis(100),
            )
            .await
            .expect_err("a wedged adb must time out")
        });
        assert!(
            error.to_string().contains("timed out") && error.to_string().contains("am instrument"),
            "{error}"
        );
    }
}
