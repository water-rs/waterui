//! `water preview --platform android`: the preview renders through
//! Hydrolysis inside the preview host APK's instrumentation — device GPU,
//! device fonts, device realizations — with no Kotlin runtime involved.
//!
//! The CLI builds the launcher's preview-mode cdylib for the device's ABI
//! and stages it with the project's assets as one content-hashed payload. On
//! the device everything lives in the host's private files under
//! `files/waterui-preview`: each run writes its config and clears `out/` there
//! while reading the payload stamp back, streams the payload straight into
//! private storage through `run-as` only when its content hash differs from
//! that stamp, and runs `am instrument -w` — whose return is the completion
//! signal — before reading the produced PNGs back through `adb shell -T`.

mod payload;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use eyre::{Context as _, Result, bail};
use futures_util::{StreamExt as _, TryStreamExt as _};
use smol::fs;
use tracing::info;

use waterui_preview_protocol::run::{PreviewRunConfig, PreviewRunMode};

use crate::android::adb::{Adb, AdbCommandError, recent_crash_log};
use crate::android::device::{AndroidAbiProvider as _, AndroidTarget};
use crate::hydrolysis::android::{
    self as hydrolysis_android, PREVIEW_HOST_INSTRUMENTATION, PREVIEW_HOST_PACKAGE,
};
use crate::preview::hydrolysis::{
    HydrolysisPreviewRequest, HydrolysisPreviewScenario, scenario_frame_path,
    write_preview_bindings,
};
use crate::preview::run::{RUN_CONFIG_FILE_NAME, run_config_json};
use crate::project_model::water_dir;
use crate::toolchain::Host;

use payload::{DevicePayload, INCOMING_STAMP_FILE, MANIFEST_FILE, PAYLOAD_DIR, STAMP_FILE};

/// `am instrument -w` is the completion signal; this bound is only the
/// backstop for a wedged instrumentation run, not the render's expected
/// duration.
const PREVIEW_RENDER_DEADLINE: Duration = Duration::from_mins(3);

/// The slowest transport a payload stream is given time for, in bytes per
/// second: a weak wireless `adb connect` link. The stream's deadline is the
/// payload's size at this rate — see [`payload_stream_deadline`].
const PAYLOAD_STREAM_MIN_THROUGHPUT: u64 = 1024 * 1024;

/// The least time a payload stream is given, whatever its size: the
/// `run-as` round trip, the previous payload's removal and the extraction
/// cost this much before the size matters.
const PAYLOAD_STREAM_DEADLINE_FLOOR: Duration = Duration::from_secs(30);

/// Where the CLI used to stage a payload in the shell user's temporary
/// directory before copying it into the host's private files. No layout of
/// this module names it — payloads now stream straight into private
/// storage — so a device that holds no payload stamp yet has it removed,
/// once, alongside its first stream.
const LEGACY_STAGING_DIR: &str = "/data/local/tmp/waterui-preview";

/// The run directory inside the preview host's `filesDir` — the
/// instrumentation extras are paths relative to `filesDir` itself.
const FILES_PREVIEW_DIR: &str = "waterui-preview";

/// The same run directory as `run-as` sees it from the app's data dir.
const FILES_RUN_DIR: &str = "files/waterui-preview";

/// Archive chunks in flight between the archive thread and `adb`'s stdin —
/// bounded so a slow transport paces the archive instead of buffering the
/// whole payload in memory.
const ARCHIVE_CHUNKS_IN_FLIGHT: usize = 8;

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

    // Every run of this project writes its preview bindings and stages its
    // payload into one fixed directory, so a second run of the same project
    // waits until the first finishes; the lease is held for the whole run.
    let _project_lease = water_dir::android_preview_project_lock(&host, project.root()).await?;

    let ((), target) = futures_util::try_join!(
        write_preview_bindings(&project, request.source, request.theme, None),
        AndroidTarget::first_available(&host),
    )?;

    // Everything local — the host APK and the launcher payload — builds
    // before the device is claimed, so a second preview waiting on this
    // device still overlaps its own builds with this run's render. The ABI
    // is known before the target is up, so an AVD boots alongside the
    // builds too.
    let ((device, device_key), (host_apk, version_code), payload) = futures_util::try_join!(
        async {
            let device = target.launch(&host).await?;
            let key = device_lock_key(&host, device.adb(), device.identifier()).await?;
            eyre::Ok((device, key))
        },
        hydrolysis_android::ensure_preview_host_apk(&project, &host),
        DevicePayload::stage(&project, &host, request, target.android_abi()),
    )?;
    // The device carries the client its scan located, so every command
    // below reuses that one running server.
    let (adb, serial) = (device.adb(), device.identifier());

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
    let run_config = run_config_json(&PreviewRunConfig {
        width: request.width,
        height: request.height,
        mode,
    })?;

    render_on_device(
        &host,
        &DeviceRender {
            adb,
            serial,
            device_key: &device_key,
            host_apk: &host_apk,
            version_code,
            payload: &payload,
            run_config: &run_config,
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
    /// The staged payload, shipped only when the device's stamp differs.
    payload: &'a DevicePayload,
    /// This run's config document.
    run_config: &'a [u8],
    output_path: &'a Path,
    scenario: Option<&'a HydrolysisPreviewScenario>,
}

/// Take the device lease, then install the host if needed, prepare the run
/// and ship the payload if the device's copy is stale, run the
/// instrumentation and pull its output.
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
        payload,
        run_config,
        output_path,
        scenario,
    } = *render;
    let _device_lease = water_dir::android_preview_device_lock(host, device_key).await?;

    // The server the device was found through may have died while the
    // builds ran; restart it detached before any command would spawn one
    // that inherits our pipes.
    adb.start_server(host).await?;

    // The install and the `logcat -T` stamp bounding the crash log to this
    // run are independent; everything `run-as` does needs the host
    // installed. `install -r` keeps the host's private files, so a payload
    // already extracted there survives a host upgrade.
    let (_installed, since) = futures_util::try_join!(
        install_host_if_needed(host, adb, serial, host_apk, version_code),
        device_time_stamp(host, adb, serial),
    )?;
    match prepare_run(host, adb, serial, run_config).await? {
        Some(stamp) if stamp == payload.stamp() => {}
        Some(_) => {
            info!("Streaming the preview payload to the device");
            stream_payload(host, adb, serial, payload).await?;
        }
        None => {
            info!("Streaming the preview payload to the device");
            futures_util::try_join!(
                stream_payload(host, adb, serial, payload),
                remove_legacy_staging(host, adb, serial),
            )?;
        }
    }
    run_instrumentation(host, adb, serial, &payload.library_paths(), &since).await?;
    pull_outputs(host, adb, serial, output_path, scenario).await
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

/// Remove [`LEGACY_STAGING_DIR`]. It belongs to the shell user, so this is
/// a plain `adb shell`, not a `run-as`.
async fn remove_legacy_staging(host: &Host, adb: &Adb, serial: &str) -> Result<()> {
    adb.shell_run(
        host,
        serial,
        &["rm", "-rf", LEGACY_STAGING_DIR],
        Duration::from_secs(30),
    )
    .await?;
    Ok(())
}

/// Prepare the run directory in one `run-as` round trip: remove every entry
/// the run directory's layout does not name — the payload, its stamp, the
/// run config and `out/` — clear `out/` so a failed render can never hand
/// back the previous run's frames, write `run_config` (streamed through
/// stdin) as the run config, and answer the payload stamp the device holds
/// — `None` when it holds none.
async fn prepare_run(
    host: &Host,
    adb: &Adb,
    serial: &str,
    run_config: &[u8],
) -> Result<Option<String>> {
    // The script's positions keep the names off the command template: `$1`
    // is the run directory, `$2` the config file, `$3` the payload stamp,
    // `$4` the payload directory.
    let stamp = adb
        .run_as_with_input(
            host,
            serial,
            PREVIEW_HOST_PACKAGE,
            &[
                "sh",
                "-c",
                "mkdir -p \"$1\" && cd \"$1\" && \
                 find . -mindepth 1 -maxdepth 1 ! -name \"$4\" ! -name \"$3\" ! -name \"$2\" \
                 ! -name out -exec rm -rf {} + && \
                 rm -rf out && mkdir out && cat > \"$2\" && if [ -f \"$3\" ]; then cat \"$3\"; fi",
                "sh",
                FILES_RUN_DIR,
                RUN_CONFIG_FILE_NAME,
                STAMP_FILE,
                PAYLOAD_DIR,
            ],
            run_config,
            Duration::from_secs(30),
        )
        .await
        .wrap_err("failed to prepare the preview run in the host's private files")?;
    let stamp = stamp.trim();
    Ok((!stamp.is_empty()).then(|| stamp.to_string()))
}

/// The deadline for streaming `size` bytes of encoded payload: their
/// transfer at [`PAYLOAD_STREAM_MIN_THROUGHPUT`], never less than
/// [`PAYLOAD_STREAM_DEADLINE_FLOOR`].
fn payload_stream_deadline(size: u64) -> Duration {
    Duration::from_secs(size.div_ceil(PAYLOAD_STREAM_MIN_THROUGHPUT))
        .max(PAYLOAD_STREAM_DEADLINE_FLOOR)
}

/// Stream `payload` as a base64-encoded tar archive straight into the
/// host's private files. The stamps go first — so no interruption after
/// that point can leave a stamp naming a payload that is partly deleted or
/// partly extracted — then the previous payload; the archive is decoded and
/// extracted in place, every extracted file is checked against the
/// archive's manifest — so the stamp vouches for the bytes, whatever a
/// lenient decoder let through — the libraries become read-only — `chmod a-w`
/// satisfies the linker's read-only `System.load` requirement without
/// write-protecting `lib/` itself, since unlinking needs write on the
/// directory, not the file — and the archive's trailing stamp is renamed
/// into place last, so the device names a payload only once all of it
/// landed.
///
/// The script's exit status is the first verdict. The second rides in the
/// same call: the script ends by printing the stamp it installed, which
/// must be the one sent.
async fn stream_payload(
    host: &Host,
    adb: &Adb,
    serial: &str,
    payload: &DevicePayload,
) -> Result<()> {
    let (sender, receiver) = async_channel::bounded(ARCHIVE_CHUNKS_IN_FLIGHT);
    let archive = receiver.map(Ok::<_, io::Error>).into_async_read();
    let upload = adb.run_as_with_input(
        host,
        serial,
        PREVIEW_HOST_PACKAGE,
        &STREAM_SCRIPT,
        archive,
        payload_stream_deadline(payload.stream_size()),
    );
    let (archived, uploaded) = futures_util::join!(payload.write_encoded_archive(sender), upload);
    let installed = match (archived, uploaded) {
        // A payload file that could not be read is the cause of whatever
        // the device then reported about the truncated archive.
        (Err(error), _) if error.kind() != io::ErrorKind::BrokenPipe => {
            return Err(eyre::eyre!(error).wrap_err("failed to archive the preview payload"));
        }
        (_, Err(error)) => {
            return Err(error
                .wrap_err("failed to stream the preview payload into the host's private files"));
        }
        // A `BrokenPipe` archive under a successful upload: the device
        // stopped reading after the stamp member, and the stamp check below
        // is the verdict on what it installed.
        (_, Ok(stdout)) => stdout,
    };
    let installed = installed.trim();
    if installed != payload.stamp() {
        bail!(
            "the device installed payload stamp `{installed}` after streaming payload stamp `{}`",
            payload.stamp()
        );
    }
    Ok(())
}

/// The `run-as` words [`stream_payload`] runs on the device: `$1` is the
/// run directory, `$2` the payload directory, `$3` the stamp, `$4` the
/// archive's incoming stamp and `$5` its manifest.
///
/// The device's `/system/bin/sh` is mksh (AOSP `external/mksh`, the
/// `cc_binary` named `sh`), whose `pipefail` option makes a failing
/// `base64 -d` fail the pipeline even when `tar` succeeds. Toybox's
/// `base64 -d` skips characters it cannot decode instead of failing, so the
/// manifest check is what catches a stream corrupted in transit: toybox
/// `sha256sum -c` (`toys/lsb/md5sum.c`) exits 1 on any mismatched, missing
/// or malformed line, and `--status` silences its per-file report.
const STREAM_SCRIPT: [&str; 9] = [
    "sh",
    "-c",
    "set -o pipefail && cd \"$1\" && rm -f \"$3\" \"$4\" && rm -rf \"$2\" && \
     base64 -d | tar -xf - && sha256sum --status -c \"$5\" && chmod a-w \"$2\"/lib/* && \
     mv \"$4\" \"$3\" && cat \"$3\"",
    "sh",
    FILES_RUN_DIR,
    PAYLOAD_DIR,
    STAMP_FILE,
    INCOMING_STAMP_FILE,
    MANIFEST_FILE,
];

/// Run the preview instrumentation: `am instrument -w` returns when the run
/// finishes, and `-r` streams its result bundle. A failure reports the
/// bundle's `error=`/`shortMsg=`/`longMsg=`, the adb status and stderr, and
/// the logcat tail this run left behind (`since` bounds it to the run).
async fn run_instrumentation(
    host: &Host,
    adb: &Adb,
    serial: &str,
    libraries: &[String],
    since: &str,
) -> Result<()> {
    let in_run_dir = |path: &str| format!("{FILES_PREVIEW_DIR}/{path}");
    let device_libraries: Vec<String> = libraries.iter().map(|path| in_run_dir(path)).collect();
    let words = instrument_words(
        &device_libraries,
        &in_run_dir(RUN_CONFIG_FILE_NAME),
        &in_run_dir(&DevicePayload::assets_root()),
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
    output_path: &Path,
    scenario: Option<&HydrolysisPreviewScenario>,
) -> Result<()> {
    /// Read the rendered PNG at `remote_name` — a path relative to the run
    /// directory — out of the host's private files, and write it to `local`
    /// once its signature checks out.
    async fn pull_png(
        host: &Host,
        adb: &Adb,
        serial: &str,
        remote_name: &str,
        local: &Path,
    ) -> Result<()> {
        let remote = format!("{FILES_RUN_DIR}/{remote_name}");
        let bytes = adb
            .run_as_cat(
                host,
                serial,
                PREVIEW_HOST_PACKAGE,
                &remote,
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

    if let Some(scenario) = scenario {
        if let Some(parent) = scenario.output_dir.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::create_dir_all(&scenario.output_dir).await?;
        futures_util::stream::iter(scenario.captures_ms.iter().copied().map(|capture_ms| {
            let remote = format!("out/scenario/frame-{capture_ms:04}ms.png");
            let local = scenario_frame_path(&scenario.output_dir, capture_ms);
            async move { pull_png(host, adb, serial, &remote, &local).await }
        }))
        .buffer_unordered(SCENARIO_PULL_CONCURRENCY)
        .try_collect::<()>()
        .await?;
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    pull_png(host, adb, serial, "out/preview.png", output_path).await
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

    /// The logged adb argv — one invocation per line, none when adb never
    /// ran.
    fn adb_argv(log: &Path) -> String {
        match std::fs::read_to_string(log) {
            Ok(argv) => argv,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("read the fake adb argv log: {error}"),
        }
    }

    /// A staged payload: one library plus an asset bundle holding a nested
    /// image and the bundle's sync stamp.
    fn staged_payload(machine: &TestMachine) -> DevicePayload {
        let lib_dir = machine.dir("lib");
        std::fs::write(lib_dir.join("libx.so"), b"\x7fELF-library").expect("lib");
        let bundle = machine.dir("stage/waterui_assets");
        std::fs::write(bundle.join("waterui-sync-stamp"), b"assets-v1").expect("stamp");
        std::fs::create_dir_all(bundle.join("images")).expect("images dir");
        std::fs::write(bundle.join("images/logo.png"), b"logo-bytes").expect("image");
        smol::block_on(DevicePayload::from_staged(
            &lib_dir,
            vec!["libx.so".to_string()],
            &bundle,
            "assets-v1",
        ))
        .expect("the payload stages")
    }

    /// The bytes a base64 stream in LF-ended lines carries.
    fn base64_decoded(encoded: &[u8]) -> Vec<u8> {
        use base64::Engine as _;

        let text: Vec<u8> = encoded
            .iter()
            .copied()
            .filter(|byte| *byte != b'\n')
            .collect();
        base64::engine::general_purpose::STANDARD
            .decode(text)
            .expect("the stream is base64")
    }

    /// Every member of the tar archive a base64 stream carries, in archive
    /// order, with its bytes.
    fn archive_members(encoded: &[u8]) -> Vec<(String, Vec<u8>)> {
        use std::io::Read as _;

        let archive = base64_decoded(encoded);
        tar::Archive::new(archive.as_slice())
            .entries()
            .expect("the stream is a tar archive")
            .map(|entry| {
                let mut entry = entry.expect("a well-formed member");
                let path = entry
                    .path()
                    .expect("a member path")
                    .to_string_lossy()
                    .into_owned();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).expect("member bytes");
                (path, bytes)
            })
            .collect()
    }

    /// The positional arguments that end a run preparation's argv.
    const PREPARE_ARGS: &str = "sh files/waterui-preview preview-run.json payload.stamp payload";

    /// The run config a test run ships.
    const RUN_CONFIG: &[u8] = br#"{"width":320,"height":240}"#;

    /// A fake device whose instrumentation succeeds, whose `cat` answers a
    /// PNG, and whose installed host is already `versionCode` 7 — plus the
    /// staged payload and host APK a run sends it. The streamed archive and
    /// the run config the device receives land in files the test reads.
    struct RenderingDevice {
        machine: TestMachine,
        host: Host,
        log: PathBuf,
        adb: Adb,
        payload: DevicePayload,
        apk: PathBuf,
        stream: PathBuf,
        run_config: PathBuf,
    }

    impl RenderingDevice {
        fn new() -> Self {
            let machine = TestMachine::new();
            let sdk = machine.install_android_sdk();
            machine.install_adb();
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
            let log = machine.root().join("adb-argv.log");
            let stream = machine.root().join("streamed-payload.tar");
            let run_config = machine.root().join("received-run-config.json");
            let host = machine.host([
                ("ANDROID_SDK_ROOT", sdk.as_os_str()),
                ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
                ("WATERUI_FAKE_ADB_STREAM", stream.as_os_str()),
                ("WATERUI_FAKE_ADB_RUN_CONFIG", run_config.as_os_str()),
            ]);
            let payload = staged_payload(&machine);
            // The stream script ends by printing the stamp it installed.
            machine.respond("ADB_STREAM_STAMP", payload.stamp());
            let apk = machine.file("host.apk", "apk");
            let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
            // Only the device run's own invocations are under test.
            std::fs::remove_file(&log).expect("clear the locate's argv");
            Self {
                machine,
                host,
                log,
                adb,
                payload,
                apk,
                stream,
                run_config,
            }
        }

        /// The payload stamp the device answers with.
        fn holds_stamp(&self, stamp: &str) {
            self.machine.respond("ADB_PAYLOAD_STAMP", stamp);
        }

        /// One run's device-side half through the production
        /// [`render_on_device`]; the installed `versionCode` matches, so it
        /// never installs.
        async fn run(&self, out: &Path) {
            render_on_device(
                &self.host,
                &DeviceRender {
                    adb: &self.adb,
                    serial: "serial",
                    device_key: "serial",
                    host_apk: &self.apk,
                    version_code: 7,
                    payload: &self.payload,
                    run_config: RUN_CONFIG,
                    output_path: out,
                    scenario: None,
                },
            )
            .await
            .expect("the device run succeeds");
        }

        fn argv(&self) -> String {
            adb_argv(&self.log)
        }
    }

    /// The index of the one argv line containing `needle`.
    fn line_of(lines: &[&str], needle: &str) -> usize {
        let found: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains(needle))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(found.len(), 1, "exactly one `{needle}` line: {lines:#?}");
        found[0]
    }

    // Every adb-driven test below is `#[cfg(unix)]`: on Windows the staged
    // `platform-tools/adb.exe` cannot carry the shell dispatcher (see
    // `TestMachine::install_adb`), so the fake adb cannot run there.

    /// A device holding no payload gets it streamed straight into the host's
    /// private files — no `adb push`, no shell-side staging — after the run
    /// is prepared and before the instrumentation; the archive carries the
    /// whole payload and ends with the stamp. The old shell-side staging
    /// directory is removed alongside, and that is its only mention.
    #[test]
    #[cfg(unix)]
    fn a_device_without_the_payload_receives_it_streamed() {
        let device = RenderingDevice::new();
        let out = device.machine.root().join("preview.png");

        smol::block_on(device.run(&out));

        let argv = device.argv();
        let lines: Vec<&str> = argv.lines().collect();
        assert!(
            !argv.contains(" push "),
            "nothing goes through adb push: {argv}"
        );
        let legacy = line_of(&lines, "/data/local/tmp");
        assert_eq!(
            lines[legacy], "-s serial shell rm -rf /data/local/tmp/waterui-preview",
            "{argv}"
        );
        let prepare = line_of(&lines, PREPARE_ARGS);
        let stream = line_of(&lines, "tar -xf");
        let instrument = line_of(&lines, "am instrument");
        let pull = line_of(&lines, "cat files/waterui-preview/out/preview.png");
        assert!(
            prepare < stream && stream < instrument && instrument < pull,
            "prepare -> stream -> instrument -> pull: {argv}"
        );
        assert!(
            lines[instrument].contains(
                "libraries waterui-preview/payload/lib/libx.so -e runConfig \
                 waterui-preview/preview-run.json -e assetsRoot \
                 waterui-preview/payload/resources/waterui_assets"
            ),
            "the instrumentation loads the streamed payload: {}",
            lines[instrument]
        );

        let members = archive_members(&std::fs::read(&device.stream).expect("the stream landed"));
        let names: Vec<&str> = members.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "payload",
                "payload/lib",
                "payload/resources",
                "payload/resources/waterui_assets",
                "payload/resources/waterui_assets/images",
                "payload/lib/libx.so",
                "payload/resources/waterui_assets/images/logo.png",
                "payload/resources/waterui_assets/waterui-sync-stamp",
                "payload/SHA256SUMS",
                "payload.stamp.new",
            ],
            "directories first, then the payload, its manifest, the stamp last"
        );
        assert_eq!(members[5].1, b"\x7fELF-library");
        let mut manifest = String::new();
        for (path, bytes) in &members[5..8] {
            use sha2::Digest as _;

            manifest.push_str(&hex::encode(sha2::Sha256::digest(bytes)));
            manifest.push_str("  ");
            manifest.push_str(path);
            manifest.push('\n');
        }
        assert_eq!(
            String::from_utf8_lossy(&members[8].1),
            manifest,
            "the manifest is every payload file's `sha256sum` line"
        );
        assert_eq!(
            members[9].1,
            device.payload.stamp().as_bytes(),
            "the archive's trailing member is the payload's stamp"
        );
        assert_eq!(
            std::fs::read(&device.run_config).expect("the run config landed"),
            RUN_CONFIG,
            "the run config travels through the preparation's stdin"
        );
        assert!(
            std::fs::read(&out)
                .expect("read")
                .starts_with(PNG_SIGNATURE),
            "the pulled PNG landed"
        );
    }

    /// A device whose stamp matches the payload's content hash receives
    /// nothing: the payload is neither streamed nor pushed.
    #[test]
    #[cfg(unix)]
    fn an_unchanged_payload_issues_no_push() {
        let device = RenderingDevice::new();
        device.holds_stamp(device.payload.stamp());

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        let argv = device.argv();
        assert!(
            !argv.contains("tar -xf") && !argv.contains(" push "),
            "an unchanged payload ships nothing: {argv}"
        );
        assert!(
            !device.stream.exists(),
            "no archive reached the device's stdin"
        );
    }

    /// A device holding a different stamp gets the changed payload streamed,
    /// carrying the new stamp.
    #[test]
    #[cfg(unix)]
    fn a_changed_payload_streams() {
        let device = RenderingDevice::new();
        device.holds_stamp(&"0".repeat(64));

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        assert!(device.argv().contains("tar -xf"), "{}", device.argv());
        let members = archive_members(&std::fs::read(&device.stream).expect("the stream landed"));
        let (last, stamp) = members.last().expect("the archive has members");
        assert_eq!(last, "payload.stamp.new");
        assert_eq!(stamp, device.payload.stamp().as_bytes());
    }

    /// A warm run — host installed, payload current — is the detached
    /// server start, the version query and the clock read side by side, then
    /// one preparation, the instrumentation and the pull: six adb
    /// invocations, no others.
    #[test]
    #[cfg(unix)]
    fn a_warm_run_issues_the_minimal_adb_sequence() {
        let device = RenderingDevice::new();
        device.holds_stamp(device.payload.stamp());

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        let argv = device.argv();
        let all: Vec<&str> = argv.lines().collect();
        assert_eq!(all[0], "start-server", "{argv}");
        let lines = &all[1..];
        let mut concurrent = lines[..2].to_vec();
        concurrent.sort_unstable();
        assert_eq!(
            concurrent,
            [
                "-s serial shell date '+%m-%d %H:%M:%S.000'",
                "-s serial shell pm list packages --show-versioncode \
                 dev.waterui.hydrolysis.preview",
            ],
            "{argv}"
        );
        assert_eq!(lines.len(), 5, "{argv}");
        assert!(
            lines[2].starts_with("-s serial shell -T run-as dev.waterui.hydrolysis.preview sh -c")
                && lines[2].ends_with(PREPARE_ARGS),
            "{argv}"
        );
        assert!(
            lines[3].starts_with("-s serial shell am instrument"),
            "{argv}"
        );
        assert_eq!(
            lines[4],
            "-s serial shell -T run-as dev.waterui.hydrolysis.preview cat \
             files/waterui-preview/out/preview.png",
            "{argv}"
        );
    }

    /// A device whose `run-as` commands run the production scripts for real
    /// — the host's `sh`, `tar`, `find` — inside a scratch private-files
    /// directory, so the payload stamp's guarantee is checked against the
    /// shell sequence the device actually runs.
    #[cfg(unix)]
    struct ScriptedDevice {
        machine: TestMachine,
        adb: Adb,
        payload: DevicePayload,
        /// The scratch private-files directory `run-as` starts in.
        data: PathBuf,
    }

    #[cfg(unix)]
    impl ScriptedDevice {
        fn new() -> Self {
            let machine = TestMachine::new();
            let sdk = machine.install_android_sdk();
            machine.install_adb();
            let data = machine.dir("device-data");
            let payload = staged_payload(&machine);
            let adb = smol::block_on(Adb::locate(&machine.host([("ANDROID_SDK_ROOT", &sdk)])))
                .expect("fake adb must locate");
            Self {
                machine,
                adb,
                payload,
                data,
            }
        }

        /// A host reaching the scripted device, with `extra` set as well.
        fn host(&self, extra: &[(&str, &std::ffi::OsStr)]) -> Host {
            let sdk = self.machine.root().join("sdk");
            self.machine.host(
                [
                    ("ANDROID_SDK_ROOT", sdk.as_os_str()),
                    ("WATERUI_FAKE_DEVICE_DATA", self.data.as_os_str()),
                ]
                .into_iter()
                .chain(extra.iter().copied()),
            )
        }

        fn run_dir(&self) -> PathBuf {
            self.data.join(FILES_RUN_DIR)
        }

        /// The payload stamp the device holds, `None` when it holds none.
        fn stamp(&self) -> Option<String> {
            match std::fs::read_to_string(self.run_dir().join(STAMP_FILE)) {
                Ok(stamp) => Some(stamp),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => panic!("read the device stamp: {error}"),
            }
        }

        /// A directory whose `name` is a shell script running `body`, to put
        /// ahead of the device's tools.
        fn shim(&self, name: &str, body: &str) -> PathBuf {
            use std::os::unix::fs::PermissionsExt as _;

            let dir = self.machine.dir("device-shims");
            let path = dir.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the shim");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("make the shim executable");
            dir
        }

        /// Prepare a run under `host`, answering the stamp it read back.
        fn prepare(&self, host: &Host) -> Option<String> {
            smol::block_on(prepare_run(host, &self.adb, "serial", RUN_CONFIG))
                .expect("the run prepares")
        }

        /// Stream the payload under `host`.
        fn stream(&self, host: &Host) -> Result<()> {
            smol::block_on(stream_payload(host, &self.adb, "serial", &self.payload))
        }

        /// Prepare and stream once with nothing in the way, so the device
        /// holds the whole payload and its stamp.
        fn install_whole_payload(&self) {
            let host = self.host(&[]);
            assert_eq!(self.prepare(&host), None, "a fresh device holds no stamp");
            self.stream(&host).expect("a whole archive streams");
            assert_eq!(self.stamp().as_deref(), Some(self.payload.stamp()));
        }
    }

    /// The encoded archive [`DevicePayload::write_encoded_archive`]
    /// produces, whole.
    #[cfg(unix)]
    fn encoded_archive(payload: &DevicePayload) -> Vec<u8> {
        let (sender, receiver) = async_channel::unbounded();
        smol::block_on(payload.write_encoded_archive(sender)).expect("the archive writes");
        std::iter::from_fn(|| receiver.try_recv().ok())
            .flatten()
            .collect()
    }

    /// A whole archive installs the payload — libraries read-only — and its
    /// stamp, which the next preparation reads back, and the preparation
    /// removes whatever the run directory's layout does not name. The stamp
    /// lands only after every step before it succeeded: a failing `chmod`,
    /// the step right before the rename, leaves none.
    #[test]
    #[cfg(unix)]
    fn a_whole_payload_archive_installs_its_stamp_last() {
        let device = ScriptedDevice::new();
        let run_dir = device.run_dir();
        std::fs::create_dir_all(run_dir.join("old-layout/lib")).expect("an old layout");
        std::fs::write(run_dir.join(INCOMING_STAMP_FILE), "stale").expect("a stale stamp");

        device.install_whole_payload();
        assert_eq!(
            device.prepare(&device.host(&[])).as_deref(),
            Some(device.payload.stamp()),
            "the preparation reads the installed stamp back"
        );
        let mut names: Vec<String> = std::fs::read_dir(&run_dir)
            .expect("list the run dir")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["out", "payload", "payload.stamp", "preview-run.json"],
            "the run directory holds only what its layout names"
        );
        let library = run_dir.join("payload/lib/libx.so");
        assert_eq!(
            std::fs::read(&library).expect("the library landed"),
            b"\x7fELF-library"
        );
        assert!(
            std::fs::metadata(&library)
                .expect("library metadata")
                .permissions()
                .readonly(),
            "the library is read-only for System.load"
        );

        let failing_chmod = device.shim("chmod", "exit 1");
        let host = device.host(&[("WATERUI_FAKE_DEVICE_PATH", failing_chmod.as_os_str())]);
        let error = device
            .stream(&host)
            .expect_err("a failing chmod fails the stream");
        assert!(format!("{error:#}").contains("status"), "{error:#}");
        assert_eq!(device.stamp(), None, "no stamp before the last step");
    }

    /// An archive cut off before its end — here inside the stamp member's
    /// header, after every payload file — fails the stream and leaves the
    /// device holding no stamp.
    #[test]
    #[cfg(unix)]
    fn a_cut_off_payload_archive_installs_no_stamp() {
        let device = ScriptedDevice::new();
        device.install_whole_payload();

        // The archive ends with the stamp's 512-byte header, its data padded
        // to 512 bytes, and the 1024-byte end marker. The cut falls on a
        // whole base64 group, so the decoder itself sees nothing wrong and
        // the archive's own end is what is missing.
        let archive_len = base64_decoded(&encoded_archive(&device.payload)).len();
        let groups = (archive_len - 1024 - 512 - 256) / 3 * 4;
        let cut = (groups + groups / 76).to_string();
        let host = device.host(&[("WATERUI_FAKE_DEVICE_STDIN_LIMIT", cut.as_ref())]);
        let error = device
            .stream(&host)
            .expect_err("a cut-off archive fails the stream");
        assert!(format!("{error:#}").contains("status"), "{error:#}");
        assert_eq!(device.stamp(), None);
    }

    /// A failing decoder fails the script, and leaves the device holding
    /// no stamp, even when `tar` succeeds — what the test pins is that
    /// `set -o pipefail` is there, so `base64 -d`'s exit status reaches
    /// the script. The host's decoder rejects the appended `!!!!`; the
    /// device's toybox `base64 -d` would skip those bytes and exit 0 —
    /// the right outcome there, the archive is intact — and on the device
    /// `sha256sum -c` is what catches corruption.
    #[test]
    #[cfg(unix)]
    fn a_corrupted_base64_stream_installs_no_stamp() {
        let device = ScriptedDevice::new();
        device.install_whole_payload();

        let corrupt_at = encoded_archive(&device.payload).len().to_string();
        let host = device.host(&[("WATERUI_FAKE_DEVICE_STDIN_CORRUPT_AT", corrupt_at.as_ref())]);
        let error = device
            .stream(&host)
            .expect_err("a corrupted stream fails the stream");
        assert!(format!("{error:#}").contains("status"), "{error:#}");
        assert_eq!(device.stamp(), None);
    }

    /// A byte of file data changed in transit — the same length, the
    /// base64 still valid, so neither the decoder nor `tar` can notice —
    /// fails the manifest check, and the device holds no stamp.
    #[test]
    #[cfg(unix)]
    fn a_substituted_data_byte_installs_no_stamp() {
        use base64::Engine as _;

        let device = ScriptedDevice::new();
        device.install_whole_payload();

        // A 3-byte group inside the library's data encodes to 4 characters
        // at the same place in the stream; flipping one of its bytes gives
        // 4 other valid characters.
        let encoded = encoded_archive(&device.payload);
        let archive = base64_decoded(&encoded);
        let data = archive
            .windows(b"ELF-library".len())
            .position(|window| window == b"ELF-library")
            .expect("the library's data is in the archive");
        let group = data.div_ceil(3) * 3;
        let original = &archive[group..group + 3];
        let mut changed = original.to_vec();
        changed[1] ^= 0x01;
        let engine = base64::engine::general_purpose::STANDARD;
        let (from, to) = (engine.encode(original), engine.encode(&changed));
        assert_eq!(
            encoded
                .windows(from.len())
                .filter(|window| *window == from.as_bytes())
                .count(),
            1,
            "`{from}` names exactly one place in the stream"
        );

        let replace = format!("{from}:{to}");
        let host = device.host(&[("WATERUI_FAKE_DEVICE_STDIN_REPLACE", replace.as_ref())]);
        let error = device
            .stream(&host)
            .expect_err("a substituted data byte fails the stream");
        assert!(format!("{error:#}").contains("status"), "{error:#}");
        assert_eq!(device.stamp(), None);
    }

    /// A delete of the previous payload that dies part-way — the libraries
    /// gone, the rest not — leaves no stamp that could name the remains.
    #[test]
    #[cfg(unix)]
    fn an_interrupted_payload_delete_leaves_no_matching_stamp() {
        let device = ScriptedDevice::new();
        device.install_whole_payload();

        let dying_rm = device.shim(
            "rm",
            "for arg; do\n\
             \x20   if [ \"$arg\" = payload ]; then /bin/rm -f payload/lib/*; exit 1; fi\n\
             done\n\
             exec /bin/rm \"$@\"",
        );
        let host = device.host(&[("WATERUI_FAKE_DEVICE_PATH", dying_rm.as_os_str())]);
        let error = device
            .stream(&host)
            .expect_err("an interrupted delete fails the stream");
        assert!(format!("{error:#}").contains("status"), "{error:#}");
        assert!(
            !device.run_dir().join("payload/lib/libx.so").exists(),
            "the delete got part-way"
        );
        assert_eq!(device.stamp(), None);
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
        // The preparation fails the way a missing package would.
        let host = machine.host([
            (
                "ANDROID_SDK_ROOT",
                machine.install_android_sdk().as_os_str(),
            ),
            ("WATERUI_FAKE_ADB_RUN_AS_STATUS", "7".as_ref()),
        ]);
        let error = smol::block_on(async {
            prepare_run(&host, &adb, "serial", RUN_CONFIG)
                .await
                .expect_err("a non-zero run-as must fail")
        });
        let message = format!("{error:#}");
        assert!(message.contains("run-as"), "{message}");
    }

    /// Two previews on one serial hold the device lease for the whole
    /// device-side run: the second run's preparation, stream, install and
    /// instrument cannot interleave the first run's, because `am instrument`
    /// force-stops the host package the first render lives in.
    #[test]
    #[cfg(unix)]
    fn concurrent_previews_on_one_serial_serialize() {
        let device = RenderingDevice::new();

        let out_a = device.machine.root().join("a.png");
        let out_b = device.machine.root().join("b.png");
        smol::block_on(async {
            futures_util::join!(device.run(&out_a), device.run(&out_b));
        });

        // The loser's whole device run lands after the winner's pull — its
        // preparation never interleaves the winner's instrument/pull window.
        let argv = device.argv();
        let second_prepare = argv.rfind(PREPARE_ARGS).expect("two runs prepared");
        let first_pull = argv.find("cat files/").expect("the first run pulled");
        let first_instrument = argv.find("am instrument").expect("instrumented");
        assert!(
            first_pull < second_prepare && first_instrument < first_pull,
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
            run_instrumentation(&host, &adb, "serial", &[], "01-01 00:00:00.000")
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
