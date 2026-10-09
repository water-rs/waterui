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

use crate::android::adb::{Adb, AdbCommandError, recent_crash_log};
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
use crate::preview::run::write_run_config;
use crate::project::Project;
use crate::project_model::{assets, water_dir};
use crate::toolchain::Host;

/// `am instrument -w` is the completion signal; this bound is only the
/// backstop for a wedged instrumentation run, not the render's expected
/// duration.
const PREVIEW_RENDER_DEADLINE: Duration = Duration::from_mins(3);

/// The shell-side staging directory a run pushes its payload into —
/// replaced on every run, removed as soon as `run-as` has copied it into
/// the host's private files.
const DEVICE_TMP_ROOT: &str = "/data/local/tmp/waterui-preview";

/// The payload root inside the preview host's `filesDir` — the
/// instrumentation extras are paths relative to `filesDir` itself, and
/// every run replaces this one fixed directory.
const FILES_PREVIEW_DIR: &str = "waterui-preview";

/// The same payload root as `run-as` sees it from the app's data dir.
const FILES_PAYLOAD_DIR: &str = "files/waterui-preview";

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
    let host = request.host;
    let project = crate::hydrolysis::backend::open_ready(host, request.project_path).await?;

    // Every run of this project writes its preview bindings and stages its
    // payload into one fixed directory, so a second run of the same project
    // waits until the first finishes; the lease is held for the whole run.
    let _project_lease = water_dir::android_preview_project_lock(host, project.root()).await?;

    let ((), (adb, serial, abi)) = futures_util::try_join!(
        write_preview_bindings(&project, request.source, request.theme, None),
        async {
            let target = AndroidTarget::first_available(host).await?;
            target.launch(host).await?;
            let serial = target
                .serial()
                .ok_or_else(|| eyre::eyre!("the Android target has no adb serial after launch"))?
                .to_string();
            eyre::Ok((Adb::locate(host).await?, serial, target.android_abi()))
        },
    )?;

    // Everything local — the host APK and the launcher payload — builds
    // before the device is claimed, so a second preview waiting on this
    // device still overlaps its own builds with this run's render.
    let device_dir = project
        .backend_path::<HydrolysisBackend>()
        .join("android-preview")
        .join("device");
    let ((host_apk, version_code), libraries, device_key) = futures_util::try_join!(
        hydrolysis_android::ensure_preview_host_apk(&project),
        stage_device_payload(&project, request, abi, &device_dir),
        device_lock_key(host, &adb, &serial),
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

    render_on_device(
        host,
        &DeviceRender {
            adb: &adb,
            serial: &serial,
            device_key: &device_key,
            host_apk: &host_apk,
            version_code,
            device_dir: &device_dir,
            run: DeviceRun {
                run_dir: FILES_PREVIEW_DIR,
                config_name: &config_name,
                libraries: &libraries,
            },
            output_path,
            scenario,
        },
    )
    .await
}

/// Everything one preview run does on the device, once its host APK and
/// payload are built locally.
struct DeviceRender<'a> {
    adb: &'a Adb,
    /// The adb transport serial every command addresses.
    serial: &'a str,
    /// The [`device_lock_key`] the device lease is taken under.
    device_key: &'a str,
    host_apk: &'a Path,
    version_code: u32,
    /// The staged payload `push_payload` sends.
    device_dir: &'a Path,
    run: DeviceRun<'a>,
    output_path: &'a Path,
    scenario: Option<&'a HydrolysisPreviewScenario>,
}

/// Take the device lease, then install the host if needed, push and stage
/// the payload, run the instrumentation and pull its output.
///
/// `am instrument` force-stops the host package and an install replaces
/// it, so a second run's install or instrumentation would kill the render
/// already running: the whole sequence holds the lease.
async fn render_on_device(host: &Host, render: &DeviceRender<'_>) -> Result<()> {
    let DeviceRender {
        adb,
        serial,
        device_key,
        host_apk,
        version_code,
        device_dir,
        ref run,
        output_path,
        scenario,
    } = *render;
    let _device_lease = water_dir::android_preview_device_lock(host, device_key).await?;

    // The install, the shell-side push and the `logcat -T` stamp bounding
    // the crash log to this run are independent; only the `run-as` copy
    // into the host's private files needs the host installed.
    let (_installed, (), since) = futures_util::try_join!(
        install_host_if_needed(host, adb, serial, host_apk, version_code),
        push_payload(host, adb, serial, device_dir),
        device_time_stamp(host, adb, serial),
    )?;
    copy_payload_into_host(host, adb, serial).await?;

    // The shell-side copy is dead weight once `run-as` staged it — remove
    // it while the render runs.
    let instrument_and_pull = async {
        run_instrumentation(host, adb, serial, run, &since).await?;
        pull_outputs(host, adb, serial, run.run_dir, output_path, scenario).await
    };
    futures_util::try_join!(clear_device_tmp(host, adb, serial), instrument_and_pull)?;
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
        .await
        .map_err(|error| {
            // The debug host APK is signed with each machine's own debug
            // key, so `install -r` over a host another machine installed is
            // rejected with INSTALL_FAILED_UPDATE_INCOMPATIBLE. Automatic
            // removal would drop that machine's host without asking — name
            // the cause and the exact command instead.
            let AdbCommandError::Failed { details, .. } = &error else {
                return error.into();
            };
            if !details.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
                return error.into();
            }
            eyre::eyre!(
                "{error}\n\n\
                 the installed preview host was signed with a different debug key — \
                 remove it with `adb -s {serial} uninstall {PREVIEW_HOST_PACKAGE}` and retry"
            )
        })?;
    Ok(true)
}

/// The key one device's preview lease is taken under.
///
/// A physical device answers its own `ro.serialno`, the same for every adb
/// transport that reaches it — one phone over USB and over `adb connect`
/// serializes against itself. An emulator's `ro.serialno` is not unique
/// (every AVD of one system image reports the same value), while its
/// `emulator-<port>` adb serial names exactly one running instance, so an
/// emulator is keyed by that serial.
async fn device_lock_key(host: &Host, adb: &Adb, serial: &str) -> Result<String> {
    if serial.starts_with("emulator-") {
        return Ok(serial.to_string());
    }
    let serial_number = adb
        .shell_run(
            host,
            serial,
            &["getprop", "ro.serialno"],
            Duration::from_secs(10),
        )
        .await?
        .trim()
        .to_string();
    if serial_number.is_empty() {
        bail!("the device at adb serial {serial} reports an empty ro.serialno");
    }
    Ok(serial_number)
}

/// The device clock's `logcat -T` time spec (`MM-DD HH:MM:SS.mmm`), read
/// before the instrumentation starts so a crash dump can be bounded to
/// this run.
async fn device_time_stamp(host: &Host, adb: &Adb, serial: &str) -> Result<String> {
    Ok(adb
        .shell_run(
            host,
            serial,
            &["date", "+%m-%d %H:%M:%S.000"],
            Duration::from_secs(10),
        )
        .await?
        .trim()
        .to_string())
}

/// Build the launcher's preview-mode cdylib, strip it in place, stage the
/// project's assets under `resources/`, and answer the staged library file
/// names in `System.load` order.
async fn stage_device_payload(
    project: &Project,
    request: &HydrolysisPreviewRequest<'_>,
    abi: AndroidAbi,
    device_dir: &Path,
) -> Result<Vec<String>> {
    let host = project.host();
    match fs::remove_dir_all(device_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
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
    let build =
        hydrolysis_android::build_with_features(project, abi, options, &["waterui-preview-mode"])
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
    let (_manifest, staged) = assets::stage_project_assets_for_android_library(
        project,
        &stage_dir,
        &build.built.app_symbols()?,
        false,
    )
    .await?;
    let resources = device_dir.join("resources");
    fs::create_dir_all(&resources).await?;
    let assets_dest = resources.join(assets::ANDROID_ASSET_BUNDLE_DIR);
    fs::rename(&staged.bundle, &assets_dest)
        .await
        .wrap_err_with(|| {
            format!(
                "failed to move {} to {}",
                staged.bundle.display(),
                assets_dest.display()
            )
        })?;

    Ok(build.staged_libraries)
}

/// Remove the shell-side staging directory.
async fn clear_device_tmp(host: &Host, adb: &Adb, serial: &str) -> Result<()> {
    adb.shell_run(
        host,
        serial,
        &["rm", "-rf", DEVICE_TMP_ROOT],
        Duration::from_secs(30),
    )
    .await?;
    Ok(())
}

/// Push the staged payload into the shell-side staging directory, cleared
/// first: `adb push` merges into an existing directory, so a previous run's
/// failed copy, timeout or interrupt would otherwise leave its files inside
/// this run's payload.
async fn push_payload(host: &Host, adb: &Adb, serial: &str, device_dir: &Path) -> Result<()> {
    clear_device_tmp(host, adb, serial).await?;
    adb.push(
        host,
        serial,
        &device_dir.join("."),
        DEVICE_TMP_ROOT,
        Duration::from_secs(120),
    )
    .await?;
    Ok(())
}

/// Copy the pushed payload into the preview host's private files, replacing
/// the previous run's — `chmod a-w` on the libraries satisfies the linker's
/// read-only `System.load` requirement without write-protecting `lib/`
/// itself (unlinking needs write on the directory, not the file), and
/// `out/` takes the render's output. `run-as` needs the host installed.
async fn copy_payload_into_host(host: &Host, adb: &Adb, serial: &str) -> Result<()> {
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
            FILES_PAYLOAD_DIR,
            DEVICE_TMP_ROOT,
        ],
        Duration::from_secs(60),
    )
    .await?;
    Ok(())
}

/// The device-side layout of a pushed run — every path is relative to the
/// preview host's `filesDir` and resolved there by the instrumentation.
struct DeviceRun<'a> {
    /// The run's `waterui-preview` payload root under `filesDir`.
    run_dir: &'a str,
    /// The run config's file name inside `run_dir`.
    config_name: &'a str,
    /// Staged library file names in `System.load` order.
    libraries: &'a [String],
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
    since: &str,
) -> Result<()> {
    let run_dir = run.run_dir;
    let device_libraries: Vec<String> = run
        .libraries
        .iter()
        .map(|name| format!("{run_dir}/lib/{name}"))
        .collect();
    let words = instrument_words(
        &device_libraries,
        &format!("{run_dir}/{}", run.config_name),
        &format!("{run_dir}/resources/{}", assets::ANDROID_ASSET_BUNDLE_DIR),
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
        let crash_log = recent_crash_log(host, adb, serial, Some(since)).await;
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
/// absolute private path it could be wrong about.
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
        Ok(())
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

    /// A fake device whose instrumentation succeeds, whose `cat` answers a
    /// PNG, and whose installed host is already `versionCode` 7 — plus the
    /// staged payload and host APK a run sends it.
    struct RenderingDevice {
        machine: TestMachine,
        host: Host,
        log: PathBuf,
        adb: Adb,
        device_dir: PathBuf,
        apk: PathBuf,
    }

    impl RenderingDevice {
        fn new() -> Self {
            let (machine, host, log) = adb_test_machine();
            machine.respond(
                "ADB_AM_INSTRUMENT",
                "INSTRUMENTATION_STATUS: stream=\nINSTRUMENTATION_CODE: -1\n",
            );
            machine.respond(
                "ADB_PM_PACKAGES",
                "package:dev.waterui.hydrolysis.preview versionCode:7",
            );
            // A PNG signature + filler — raw bytes, not a UTF-8 string.
            std::fs::write(
                machine.responses().join("ADB_CAT"),
                b"\x89PNG\r\n\x1a\nfake-frame-bytes",
            )
            .expect("stage the canned cat");
            let device_dir = staged_payload(&machine);
            let apk = machine.file("host.apk", "apk");
            let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
            Self {
                machine,
                host,
                log,
                adb,
                device_dir,
                apk,
            }
        }

        /// One run's device-side half through the production
        /// [`render_on_device`]; the installed `versionCode` matches, so it
        /// goes straight to push, stage, instrument and pull.
        async fn run(&self, out: &Path) {
            let libraries = ["libx.so".to_string()];
            render_on_device(
                &self.host,
                &DeviceRender {
                    adb: &self.adb,
                    serial: "serial",
                    device_key: "serial",
                    host_apk: &self.apk,
                    version_code: 7,
                    device_dir: &self.device_dir,
                    run: DeviceRun {
                        run_dir: FILES_PREVIEW_DIR,
                        config_name: "preview-run.json",
                        libraries: &libraries,
                    },
                    output_path: out,
                    scenario: None,
                },
            )
            .await
            .expect("the device run succeeds");
        }
    }

    // Every adb-driven test below is `#[cfg(unix)]`: on Windows the staged
    // `platform-tools/adb.exe` cannot carry the shell dispatcher (see
    // `TestMachine::install_adb`), so the fake adb cannot run there.
    #[test]
    #[cfg(unix)]
    fn the_pipeline_pushes_stages_instruments_and_pulls_in_order() {
        let device = RenderingDevice::new();
        let out = device.machine.root().join("preview.png");

        smol::block_on(device.run(&out));

        let argv = adb_argv(&device.log);
        let clear = argv
            .find("shell rm -rf /data/local/tmp/waterui-preview")
            .expect("the shell-side staging dir was cleared");
        let push = argv.find(" push ").expect("push ran");
        let run_as = argv.find("run-as").expect("run-as ran");
        let instrument = argv.find("am instrument").expect("instrument ran");
        let cat = argv.rfind("cat files/").expect("cat ran");
        assert!(
            clear < push && push < run_as && run_as < instrument && instrument < cat,
            "clear -> push -> run-as -> instrument -> pull order: {argv}"
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
            copy_payload_into_host(&host, &adb, "serial")
                .await
                .expect_err("a non-zero run-as must fail")
        });
        assert!(error.to_string().contains("run-as"), "{error}");
    }

    /// Two previews on one serial hold the device lease for the whole
    /// device-side run: the second push, install and instrument cannot
    /// interleave the first run's, because `am instrument` force-stops the
    /// host package the first render lives in.
    #[test]
    #[cfg(unix)]
    fn concurrent_previews_on_one_serial_serialize() {
        let device = RenderingDevice::new();

        let out_a = device.machine.root().join("a.png");
        let out_b = device.machine.root().join("b.png");
        smol::block_on(async {
            futures_util::join!(device.run(&out_a), device.run(&out_b));
        });

        // The loser's whole device run lands after the winner's pull —
        // its push never interleaves the winner's instrument/pull window.
        let argv = adb_argv(&device.log);
        let second_push = argv.rfind(" push ").expect("two pushes ran");
        let first_pull = argv.find("cat files/").expect("the first run pulled");
        let first_instrument = argv.find("am instrument").expect("instrumented");
        assert!(
            first_pull < second_push && first_instrument < first_pull,
            "the second run waited for the first to finish:\n{argv}"
        );
    }

    /// A physical device is keyed by its own `ro.serialno`, whatever
    /// transport reaches it; an emulator by its `emulator-<port>` serial,
    /// because emulators share their `ro.serialno`.
    #[test]
    #[cfg(unix)]
    fn device_lock_keys_name_one_device() {
        let (machine, host, log) = adb_test_machine();
        machine.respond("ADB_GETPROP", "R5CT1234ABC\n");
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
        smol::block_on(async {
            assert_eq!(
                device_lock_key(&host, &adb, "emulator-5556")
                    .await
                    .expect("emulator key"),
                "emulator-5556"
            );
            assert!(
                !std::fs::read_to_string(&log)
                    .unwrap_or_default()
                    .contains("getprop"),
                "an emulator is keyed without asking it"
            );
            for transport in ["R5CT1234ABC", "192.168.1.20:5555"] {
                assert_eq!(
                    device_lock_key(&host, &adb, transport)
                        .await
                        .expect("physical key"),
                    "R5CT1234ABC"
                );
            }
        });
    }

    /// A host APK installed from another machine's debug key rejects
    /// `install -r` with `INSTALL_FAILED_UPDATE_INCOMPATIBLE` — the error
    /// names the cause and the exact `adb uninstall` the user must run;
    /// the CLI never removes it on its own.
    #[test]
    #[cfg(unix)]
    fn a_debug_key_mismatch_names_the_uninstall_command() {
        let (machine, host, _log) = adb_test_machine();
        machine.respond(
            "ADB_PM_PACKAGES",
            "package:dev.waterui.hydrolysis.preview versionCode:6",
        );
        machine.respond(
            "ADB_INSTALL",
            "Performing Streamed Install\nFailure [INSTALL_FAILED_UPDATE_INCOMPATIBLE]",
        );
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
        let host = machine.host([
            (
                "ANDROID_SDK_ROOT",
                machine.install_android_sdk().as_os_str(),
            ),
            ("WATERUI_FAKE_ADB_INSTALL_STATUS", "1".as_ref()),
        ]);
        let apk = machine.file("host.apk", "apk");
        let error =
            smol::block_on(async { install_host_if_needed(&host, &adb, "serial", &apk, 7).await })
                .expect_err("a debug-key mismatch must fail");
        let message = format!("{error:#}");
        assert!(
            message.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE"),
            "the failure keeps adb's verdict: {message}"
        );
        assert!(
            message.contains("debug key")
                && message.contains("adb -s serial uninstall dev.waterui.hydrolysis.preview"),
            "the remedy names the exact command: {message}"
        );
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
                    run_dir: FILES_PREVIEW_DIR,
                    config_name: "preview-run.json",
                    libraries: &[],
                },
                "01-01 00:00:00.000",
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
