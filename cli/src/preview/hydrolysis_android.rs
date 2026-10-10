//! `water preview --platform android`: the preview renders through
//! Hydrolysis inside the preview host APK's instrumentation — device GPU,
//! device fonts, device realizations — with no Kotlin runtime involved.
//!
//! The CLI builds the launcher's preview-mode cdylib for the device's ABI
//! and stages it with the project's assets as a payload of content-hashed
//! parts. On the device everything lives in the host's private files under
//! `files/waterui-preview`: each run writes its config and clears `out/` there
//! while reading the parts' stamps back, ships only the parts whose content
//! hash differs from the device's stamp — one `adb push` into the shell
//! user's staging directory, then one `run-as` copy into private storage
//! that writes the stamps — and renders through the live preview host it
//! started earlier or starts now: one instrumentation process that answers
//! render requests over an `adb forward`ed channel and reports the payload
//! stamp it loaded in its greeting, so an unchanged payload reuses the
//! process and a changed one force-stops it first. The produced PNGs come
//! back through `adb shell -T`. Round trips that do not depend on each
//! other run side by side: the version check, the clock read and the
//! preparation; the push and the forward lookup; and the render and the
//! staging copy's removal.

mod device_host;
mod payload;

use std::path::{Path, PathBuf};
use std::time::Duration;

use eyre::{Context as _, Result, bail};
use futures_util::future::OptionFuture;
use futures_util::{StreamExt as _, TryStreamExt as _};
use smol::fs;
use tracing::info;

use waterui_preview_protocol::run::{PreviewRunConfig, PreviewRunMode};

use crate::android::adb::{Adb, AdbCommandError, recent_crash_log};
use crate::android::device::{AndroidAbiProvider as _, AndroidTarget};
use crate::hydrolysis::android::{self as hydrolysis_android, PREVIEW_HOST_PACKAGE};
use crate::preview::hydrolysis::{
    HydrolysisPreviewRequest, HydrolysisPreviewScenario, scenario_frame_path,
    write_preview_bindings,
};
use crate::preview::run::{RUN_CONFIG_FILE_NAME, run_config_json};
use crate::project_model::water_dir;
use crate::toolchain::Host;

use payload::{DevicePayload, HeldStamps, PAYLOAD_DIR, PayloadPart};

/// The slowest transport a payload push is given time for, in bytes per
/// second: a weak wireless `adb connect` link. The push's deadline is the
/// shipped parts' size at this rate — see [`payload_deadline`].
const PAYLOAD_MIN_THROUGHPUT: u64 = 1024 * 1024;

/// The least time a payload push or install is given, whatever its size:
/// the round trip and the previous part's removal cost this much before the
/// size matters.
const PAYLOAD_DEADLINE_FLOOR: Duration = Duration::from_secs(30);

/// The shell user's directory a payload push lands in before `run-as`
/// copies it into the host's private files — the app cannot read a push
/// target of its own. Every run preparation removes it, so a push always
/// lands on an absent target and `adb push` copies the source as that
/// target instead of nesting it inside; the install removes it once the
/// copy is done.
const PUSH_STAGING_DIR: &str = "/data/local/tmp/waterui-preview";

/// The run directory inside the preview host's `filesDir` — the
/// instrumentation extras are paths relative to `filesDir` itself.
const FILES_PREVIEW_DIR: &str = "waterui-preview";

/// The same run directory as `run-as` sees it from the app's data dir.
const FILES_RUN_DIR: &str = "files/waterui-preview";

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
/// `kotlin` is the toolchain the caller's toolchain check resolved — the
/// launcher build reuses it rather than probing `kotlinc` again.
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
    kotlin: &crate::android::KotlinToolchain,
) -> Result<()> {
    let host = request.host;
    let project = crate::hydrolysis::backend::open_ready(host, request.project_path).await?;

    // Every run of this project writes its preview bindings and stages its
    // payload into one fixed directory, so a second run of the same project
    // waits until the first finishes; the lease is held for the whole run.
    let _project_lease = water_dir::android_preview_project_lock(host, project.root()).await?;

    let ((), target) = futures_util::try_join!(
        write_preview_bindings(&project, request.source, request.theme, None),
        AndroidTarget::first_available(host),
    )?;

    // Everything local — the host APK and the launcher payload — builds
    // before the device is claimed, so a second preview waiting on this
    // device still overlaps its own builds with this run's render. The ABI
    // is known before the target is up, so an AVD boots alongside the
    // builds too.
    let ((device, device_key), (host_apk, version_code), payload) = futures_util::try_join!(
        async {
            let device = target.launch(host).await?;
            let key = device_lock_key(host, device.adb(), device.identifier()).await?;
            eyre::Ok((device, key))
        },
        hydrolysis_android::ensure_preview_host_apk(&project),
        DevicePayload::stage(&project, host, request, target.android_abi(), kotlin),
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
        host,
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
    /// The staged payload, whose parts ship only when the device's stamp
    /// for them differs.
    payload: &'a DevicePayload,
    /// This run's config document.
    run_config: &'a [u8],
    output_path: &'a Path,
    scenario: Option<&'a HydrolysisPreviewScenario>,
}

/// Take the device lease, then prepare the run alongside the host's version
/// check — installing the host and preparing again when it is not current —
/// ship the payload parts the device's copy is stale in while the `adb
/// forward` is looked up beside the push, then probe the live host —
/// starting one when none answers or a stale one is replaced — send it the render request and pull the output while the
/// push's staging copy is removed.
///
/// `am instrument` force-stops the host package and an install replaces
/// it, so a second run's install or host start would kill the render
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

    // The version query, the `logcat -T` stamp bounding the crash log to
    // this run and the run's preparation are independent round trips
    // whenever the host is already installed at this version — every warm
    // run — so they run side by side. The preparation goes through
    // `run-as`, which needs the installed host: when the query calls for an
    // install, the install starts once all three are done, so it never
    // overlaps a `run-as`, and the preparation runs again against the host
    // it installed — the speculative one found no host or prepared files
    // the replacing install may have touched. `install -r` keeps the host's
    // private files, so a payload already extracted there survives a host
    // upgrade.
    let (installed, since, prepared) = futures_util::join!(
        adb.installed_version_code(host, serial, PREVIEW_HOST_PACKAGE, Duration::from_secs(30)),
        device_time_stamp(host, adb, serial),
        prepare_run(host, adb, serial, run_config),
    );
    let since = since?;
    let held = if installed? == Some(version_code) {
        prepared?
    } else {
        install_host(host, adb, serial, host_apk, version_code).await?;
        prepare_run(host, adb, serial, run_config).await?
    };

    let stale = payload.stale_parts(&held);
    let pushed = !stale.is_empty();
    if pushed {
        info!(?stale, "Pushing the changed preview payload to the device");
    }
    // The forward lookup rides beside the push; the host probe waits for
    // the push, so a fresh host starts only once the payload it will report
    // as loaded is in place, and a reused host's connection is not held
    // open across a long resources push.
    let ((), forward) = futures_util::try_join!(
        async {
            if pushed {
                push_payload(host, adb, serial, payload, &stale).await
            } else {
                Ok(())
            }
        },
        device_host::open_forward(host, adb, serial),
    )?;
    let render = async {
        let mut link = forward
            .link(
                host,
                adb,
                serial,
                &payload.library_paths(),
                payload.stamp(PayloadPart::Libraries),
                &since,
            )
            .await?;
        if let Err(error) = link
            .render(
                &in_files_dir(RUN_CONFIG_FILE_NAME),
                &in_files_dir(&DevicePayload::assets_root()),
            )
            .await
        {
            let crash_log = recent_crash_log(host, adb, serial, Some(&since)).await;
            bail!("{error:#}\n\n=== Crash Log ===\n{crash_log}");
        }
        pull_outputs(host, adb, serial, output_path, scenario).await
    };
    // The staging copy is dead once installed, and nothing the render reads
    // lives there, so its removal overlaps the render instead of delaying
    // it. A run that fails before this point leaves it for the next run's
    // preparation, which clears it first.
    let remove_staging = OptionFuture::from(pushed.then(|| remove_push_staging(host, adb, serial)));
    let (rendered, removed) = futures_util::join!(render, remove_staging);
    rendered?;
    removed.transpose()?;
    Ok(())
}

/// `path`, which the run directory holds, as the host resolves it relative
/// to its `filesDir`.
fn in_files_dir(path: &str) -> String {
    format!("{FILES_PREVIEW_DIR}/{path}")
}

/// Install `apk`, the preview host at `version_code`, over whatever host
/// the device holds.
async fn install_host(
    host: &Host,
    adb: &Adb,
    serial: &str,
    apk: &Path,
    version_code: u32,
) -> Result<()> {
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
    Ok(())
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

/// Prepare the run in one call: as the shell user, remove
/// [`PUSH_STAGING_DIR`] so this run's push lands on an absent target; then,
/// through `run-as`, remove every entry of the run directory its layout does
/// not name — the payload, the part stamps, the run config and `out/` — and
/// every entry of the payload that is not a part, clear `out/` so a failed
/// render can never hand back the previous run's frames, write `run_config`
/// (streamed through stdin) as the run config, and answer the stamps the
/// device holds.
async fn prepare_run(
    host: &Host,
    adb: &Adb,
    serial: &str,
    run_config: &[u8],
) -> Result<HeldStamps> {
    let [libraries, resources] = PayloadPart::ALL;
    let words = [
        "sh",
        "-c",
        PREPARE_SHELL_SCRIPT,
        "sh",
        PUSH_STAGING_DIR,
        "run-as",
        PREVIEW_HOST_PACKAGE,
        "sh",
        "-c",
        PREPARE_SCRIPT,
        "sh",
        FILES_RUN_DIR,
        RUN_CONFIG_FILE_NAME,
        PAYLOAD_DIR,
        libraries.stamp_file(),
        resources.stamp_file(),
        libraries.dir(),
        resources.dir(),
    ];
    let stamps = adb
        .shell_with_input(host, serial, &words, run_config, Duration::from_secs(30))
        .await
        .wrap_err("failed to prepare the preview run in the host's private files")?;
    HeldStamps::parse(&stamps)
}

/// The shell user's half of [`prepare_run`]: `$1` is [`PUSH_STAGING_DIR`],
/// the rest the `run-as` command it then becomes, keeping the stdin the run
/// config arrives on.
const PREPARE_SHELL_SCRIPT: &str = "rm -rf \"$1\" && shift && exec \"$@\"";

/// The `run-as` half of [`prepare_run`]: `$1` is the run directory, `$2`
/// the config file, `$3` the payload directory, `$4` and `$5` the part
/// stamps and `$6` and `$7` the part directories, in [`PayloadPart::ALL`]
/// order. It prints one line per part stamp, empty when the device holds
/// none.
const PREPARE_SCRIPT: &str = "mkdir -p \"$1\" && cd \"$1\" && \
     find . -mindepth 1 -maxdepth 1 ! -name \"$2\" ! -name \"$3\" ! -name \"$4\" \
     ! -name \"$5\" ! -name out -exec rm -rf {} + && \
     if [ -d \"$3\" ]; then \
     find \"$3\" -mindepth 1 -maxdepth 1 ! -name \"$6\" ! -name \"$7\" -exec rm -rf {} +; fi && \
     rm -rf out && mkdir out && cat > \"$2\" && \
     for stamp in \"$4\" \"$5\"; do if [ -f \"$stamp\" ]; then cat \"$stamp\"; fi; echo; done";

/// The deadline for moving `size` bytes of payload: their transfer at
/// [`PAYLOAD_MIN_THROUGHPUT`], never less than [`PAYLOAD_DEADLINE_FLOOR`].
fn payload_deadline(size: u64) -> Duration {
    Duration::from_secs(size.div_ceil(PAYLOAD_MIN_THROUGHPUT)).max(PAYLOAD_DEADLINE_FLOOR)
}

/// Ship `stale` — the payload parts the device's stamps do not match — into
/// the host's private files in two calls.
///
/// One `adb push` carries them into [`PUSH_STAGING_DIR`]: the whole local
/// payload directory when every part is stale, the one stale part's
/// directory otherwise — the staging directory is absent, so either lands
/// as the parts' directories inside it. One `run-as` call then runs
/// [`INSTALL_SCRIPT`], which replaces each part and writes its stamp last;
/// its exit status is the call's, so a failure surfaces. The staging copy
/// stays for [`remove_push_staging`].
async fn push_payload(
    host: &Host,
    adb: &Adb,
    serial: &str,
    payload: &DevicePayload,
    stale: &[PayloadPart],
) -> Result<()> {
    let (local, remote) = match stale {
        [part] => (
            payload.dir().join(part.dir()),
            format!("{PUSH_STAGING_DIR}/{}", part.dir()),
        ),
        _ => (payload.dir().to_path_buf(), PUSH_STAGING_DIR.to_string()),
    };
    let deadline = payload_deadline(payload.size(stale));
    adb.push(host, serial, &local, &remote, deadline)
        .await
        .wrap_err("failed to push the preview payload to the device")?;

    let mut words = vec![
        "run-as",
        PREVIEW_HOST_PACKAGE,
        "sh",
        "-c",
        INSTALL_SCRIPT,
        "sh",
        FILES_RUN_DIR,
        PUSH_STAGING_DIR,
        PAYLOAD_DIR,
    ];
    for part in stale {
        words.extend([part.dir(), part.stamp_file(), payload.stamp(*part)]);
    }
    adb.shell_run(host, serial, &words, deadline)
        .await
        .wrap_err("failed to copy the preview payload into the host's private files")?;
    Ok(())
}

/// Remove [`PUSH_STAGING_DIR`], the shell user's copy of the payload parts
/// [`push_payload`] installed.
async fn remove_push_staging(host: &Host, adb: &Adb, serial: &str) -> Result<()> {
    adb.shell_run(
        host,
        serial,
        &["rm", "-rf", PUSH_STAGING_DIR],
        Duration::from_secs(30),
    )
    .await
    .wrap_err("failed to remove the preview payload's push staging")?;
    Ok(())
}

/// The script [`push_payload`]'s install runs through `run-as`: `$1` is the run
/// directory, `$2` the staging directory, `$3` the payload directory, then
/// one `<part dir> <stamp file> <stamp>` triple per part to install. Each
/// part's stamp is removed before its directory is replaced and written
/// only after the copy and the `chmod` succeeded, so an interruption leaves
/// no stamp naming a part that is partly deleted or partly copied. The
/// copied files become read-only — `chmod a-w` satisfies the linker's
/// read-only `System.load` requirement without write-protecting the
/// directories, since unlinking needs write on the directory, not the file.
const INSTALL_SCRIPT: &str = "cd \"$1\" && staging=$2 && payload=$3 && shift 3 && \
     mkdir -p \"$payload\" || exit 1; \
     while [ $# -gt 0 ]; do \
     rm -f \"$2\" && rm -rf \"$payload/$1\" && cp -R \"$staging/$1\" \"$payload/$1\" && \
     find \"$payload/$1\" -type f -exec chmod a-w {} + && printf %s \"$3\" > \"$2\" || exit 1; \
     shift 3; done";

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::testing::TestMachine;

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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => panic!("read the fake adb argv log: {error}"),
        }
    }

    /// A staged payload: one library plus an asset bundle holding a nested
    /// image and the bundle's sync stamp.
    fn staged_payload(machine: &TestMachine) -> DevicePayload {
        let dir = machine.dir("payload");
        let lib_dir = machine.dir("payload/lib");
        std::fs::write(lib_dir.join("libx.so"), b"\x7fELF-library").expect("lib");
        let bundle = machine.dir("payload/resources/waterui_assets");
        std::fs::write(bundle.join("waterui-sync-stamp"), b"assets-v1").expect("stamp");
        std::fs::create_dir_all(bundle.join("images")).expect("images dir");
        std::fs::write(bundle.join("images/logo.png"), b"logo-bytes").expect("image");
        smol::block_on(DevicePayload::from_staged(
            dir,
            vec!["libx.so".to_string()],
            "assets-v1".to_string(),
        ))
        .expect("the payload stages")
    }

    /// The positional arguments that end a run preparation's argv.
    const PREPARE_ARGS: &str = "sh files/waterui-preview preview-run.json payload lib.stamp \
                                resources.stamp lib resources";

    /// The run config a test run ships.
    const RUN_CONFIG: &[u8] = br#"{"width":320,"height":240}"#;

    /// The abstract-domain socket name the fake forward names — the same
    /// `localabstract:` spec `device_host` registers.
    const HOST_SOCKET_SPEC: &str = "localabstract:dev.waterui.hydrolysis.preview";

    /// What the fake preview host does with each connection it accepts, in
    /// order of arrival.
    #[cfg(unix)]
    enum FakeHostScript {
        /// Every connection gets the greeting and a `rendered` reply — a
        /// live host carrying this run's payload.
        Warm,
        /// The first connection hangs up without greeting — a forward whose
        /// device side is dead; the rest serve normally.
        ColdThenWarm,
        /// The first connection greets with a stale stamp and closes; the
        /// rest serve the current stamp.
        StaleThenWarm(String),
        /// Every connection greets, then answers the request with `failed`.
        WarmFailing,
        /// Every connection sends a line that is not a frame.
        Garbled,
    }

    /// A stand-in for the device-side preview host: a real localhost TCP
    /// listener the CLI reaches through the port the fake adb's `forward`
    /// answers, speaking the protocol's JSON lines.
    #[cfg(unix)]
    struct FakeHost {
        port: u16,
        requests: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[cfg(unix)]
    impl FakeHost {
        fn serve(script: FakeHostScript, stamp: String) -> Self {
            use std::io::{BufRead as _, Write as _};

            let listener =
                std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind the fake host");
            let port = listener.local_addr().expect("local address").port();
            let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let seen = std::sync::Arc::clone(&requests);
            std::thread::spawn(move || {
                let mut first = true;
                for connection in listener.incoming() {
                    let mut stream = connection.expect("accept a connection");
                    let greeting = |stream: &mut std::net::TcpStream, stamp: &str| {
                        let hello = serde_json::json!({
                            "type": "hello",
                            "schema": 1,
                            "stamp": stamp,
                        });
                        stream
                            .write_all(format!("{hello}\n").as_bytes())
                            .expect("greet");
                    };
                    if matches!(script, FakeHostScript::Garbled) {
                        stream.write_all(b"not a frame\n").expect("garble");
                        continue;
                    }
                    if first {
                        first = false;
                        match &script {
                            FakeHostScript::Warm
                            | FakeHostScript::WarmFailing
                            | FakeHostScript::Garbled => {}
                            FakeHostScript::ColdThenWarm => continue,
                            FakeHostScript::StaleThenWarm(stale) => {
                                greeting(&mut stream, stale);
                                continue;
                            }
                        }
                    }
                    greeting(&mut stream, &stamp);
                    let mut request = String::new();
                    std::io::BufReader::new(stream.try_clone().expect("clone the stream"))
                        .read_line(&mut request)
                        .expect("read the render request");
                    seen.lock().expect("recorded requests").push(request);
                    let reply = match &script {
                        FakeHostScript::WarmFailing => {
                            serde_json::json!({"type": "failed", "error": "preview exploded"})
                        }
                        _ => serde_json::json!({"type": "rendered"}),
                    };
                    stream
                        .write_all(format!("{reply}\n").as_bytes())
                        .expect("reply");
                }
            });
            Self { port, requests }
        }

        /// The request lines the host was sent, in arrival order.
        fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("recorded requests").clone()
        }
    }

    /// A fake device whose `cat` answers a PNG, whose installed host is
    /// already `versionCode` 7 and whose preview host is a [`FakeHost`] the
    /// fake adb forwards to — plus the staged payload and host APK a run
    /// sends it. The run config the device receives lands in a file the
    /// test reads.
    struct RenderingDevice {
        machine: TestMachine,
        host: Host,
        log: PathBuf,
        adb: Adb,
        payload: DevicePayload,
        apk: PathBuf,
        run_config: PathBuf,
        fake_host: FakeHost,
    }

    impl RenderingDevice {
        /// A device with no live host: the probe's connection hangs up, and
        /// the host the run then starts serves normally.
        fn new() -> Self {
            Self::with_host_script(FakeHostScript::ColdThenWarm)
        }

        fn with_host_script(script: FakeHostScript) -> Self {
            let machine = TestMachine::new();
            let sdk = machine.install_android_sdk();
            machine.install_adb();
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
            let payload = staged_payload(&machine);
            let fake_host =
                FakeHost::serve(script, payload.stamp(PayloadPart::Libraries).to_string());
            machine.respond("ADB_FORWARD_PORT", &fake_host.port.to_string());
            // A started host's readiness line, as `logcat -m 1` prints it.
            machine.respond(
                "ADB_LOGCAT",
                &format!(
                    "01-01 00:00:01.000  4242  4242 I HydrolysisPreview: preview host serving: \
                     stamp {}",
                    payload.stamp(PayloadPart::Libraries)
                ),
            );
            let log = machine.root().join("adb-argv.log");
            let run_config = machine.root().join("received-run-config.json");
            let host = machine.host([
                ("ANDROID_SDK_ROOT", sdk.as_os_str()),
                ("WATERUI_FAKE_ADB_LOG", log.as_os_str()),
                ("WATERUI_FAKE_ADB_STDIN", run_config.as_os_str()),
            ]);
            let apk = machine.file("host.apk", "apk");
            let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
            // Only the device run's own invocations are under test.
            std::fs::remove_file(&log).expect("clear the locate's argv");
            let device = Self {
                machine,
                host,
                log,
                adb,
                payload,
                apk,
                run_config,
                fake_host,
            };
            device.holds_stamps(None, None);
            device
        }

        /// The part stamps the device answers with.
        fn holds_stamps(&self, libraries: Option<&str>, resources: Option<&str>) {
            self.machine.respond(
                "ADB_RUN_AS_STDOUT",
                &format!(
                    "{}\n{}\n",
                    libraries.unwrap_or_default(),
                    resources.unwrap_or_default()
                ),
            );
        }

        /// One run's device-side half through the production
        /// [`render_on_device`] at `versionCode` 7, which the device holds
        /// unless a test answers the version query otherwise.
        async fn run(&self, out: &Path) {
            self.try_run(out).await.expect("the device run succeeds");
        }

        /// [`run`], but the run's error comes back for a failure test to
        /// inspect.
        async fn try_run(&self, out: &Path) -> Result<()> {
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

    /// A device holding no payload gets the whole payload directory in one
    /// push into the shell user's staging directory, which the preparation
    /// cleared, then one install call that copies every part and writes its
    /// stamp — after the run is prepared and before the instrumentation.
    #[test]
    #[cfg(unix)]
    fn a_device_without_the_payload_receives_one_push() {
        let device = RenderingDevice::new();
        let out = device.machine.root().join("preview.png");

        smol::block_on(device.run(&out));

        let argv = device.argv();
        let lines: Vec<&str> = argv.lines().collect();
        let prepare = line_of(&lines, PREPARE_ARGS);
        assert!(
            lines[prepare].contains(
                "sh /data/local/tmp/waterui-preview run-as dev.waterui.hydrolysis.preview sh -c"
            ),
            "the preparation clears the push staging before its run-as: {}",
            lines[prepare]
        );
        let push = line_of(&lines, " push ");
        assert_eq!(
            lines[push],
            format!(
                "-s serial push {} /data/local/tmp/waterui-preview",
                device.payload.dir().display()
            ),
            "{argv}"
        );
        let install = line_of(&lines, "cp -R");
        assert!(
            lines[install].ends_with(&format!(
                "sh files/waterui-preview /data/local/tmp/waterui-preview payload lib lib.stamp {} \
                 resources resources.stamp {}",
                device.payload.stamp(PayloadPart::Libraries),
                device.payload.stamp(PayloadPart::Resources)
            )),
            "the install names every part with its stamp: {}",
            lines[install]
        );
        let instrument = line_of(&lines, "am instrument");
        let pull = line_of(&lines, "cat files/waterui-preview/out/preview.png");
        assert!(
            prepare < push && push < install && install < instrument && instrument < pull,
            "prepare -> push -> install -> instrument -> pull: {argv}"
        );
        let removal = line_of(
            &lines,
            "-s serial shell rm -rf /data/local/tmp/waterui-preview",
        );
        assert!(
            install < removal,
            "the staging copy is removed once installed: {argv}"
        );
        assert!(
            lines[instrument].contains(&format!(
                "am instrument -e libraries waterui-preview/payload/lib/libx.so \
                 -e payloadStamp {}",
                device.payload.stamp(PayloadPart::Libraries)
            )),
            "the host starts with the installed payload and its stamp: {}",
            lines[instrument]
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

    /// A device whose stamps match every part's content hash receives
    /// nothing: no push, no install and no staging removal.
    #[test]
    #[cfg(unix)]
    fn an_unchanged_payload_issues_no_push() {
        let device = RenderingDevice::new();
        device.holds_stamps(
            Some(device.payload.stamp(PayloadPart::Libraries)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        let argv = device.argv();
        assert!(
            !argv.contains(" push ") && !argv.contains("cp -R") && !argv.contains("rm -rf /data"),
            "an unchanged payload ships nothing: {argv}"
        );
    }

    /// A device whose library stamp differs while its resources are current
    /// gets only the libraries' directory pushed and installed.
    #[test]
    #[cfg(unix)]
    fn a_changed_library_pushes_only_its_directory() {
        let device = RenderingDevice::new();
        device.holds_stamps(
            Some(&"0".repeat(64)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        let argv = device.argv();
        let lines: Vec<&str> = argv.lines().collect();
        assert_eq!(
            lines[line_of(&lines, " push ")],
            format!(
                "-s serial push {} /data/local/tmp/waterui-preview/lib",
                device.payload.dir().join("lib").display()
            ),
            "{argv}"
        );
        let install = lines[line_of(&lines, "cp -R")];
        assert!(
            install.ends_with(&format!(
                "payload lib lib.stamp {}",
                device.payload.stamp(PayloadPart::Libraries)
            )) && !install.contains("resources"),
            "only the libraries install: {install}"
        );
    }

    /// A cold run — the probe's connection hangs up — pushes nothing when
    /// the payload is current, then registers the forward and starts the
    /// host with the payload's library stamp before the render pulls its
    /// output.
    #[test]
    #[cfg(unix)]
    fn a_cold_run_starts_a_host() {
        let device = RenderingDevice::new();
        device.holds_stamps(
            Some(device.payload.stamp(PayloadPart::Libraries)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        let argv = device.argv();
        let lines: Vec<&str> = argv.lines().collect();
        assert_eq!(
            lines[line_of(&lines, "forward tcp:0")],
            format!("-s serial forward tcp:0 {HOST_SOCKET_SPEC}"),
            "{argv}"
        );
        let instrument = lines[line_of(&lines, "am instrument")];
        assert!(
            instrument.starts_with("-s serial shell am instrument -e libraries")
                && instrument.contains("-e payloadStamp")
                && instrument
                    .ends_with("dev.waterui.hydrolysis.preview.HydrolysisPreviewInstrumentation"),
            "the host starts with its payload and stamp, no -w: {instrument}"
        );
        assert!(
            !argv.contains("force-stop"),
            "nothing answered, so nothing was stopped: {argv}"
        );
        let pull = line_of(&lines, "cat files/waterui-preview/out/preview.png");
        assert!(
            line_of(&lines, "am instrument") < pull,
            "the host runs before the pull: {argv}"
        );
    }

    /// A warm run — host installed, payload current, live host answering
    /// with the run's stamp — is the detached server start, the version
    /// query, the clock read and the preparation side by side, then the
    /// forward listing and the pull: five adb invocations and not a single
    /// `am` — the render travels on the socket the live host already owns.
    #[test]
    #[cfg(unix)]
    fn a_warm_run_reuses_the_live_host() {
        let device = RenderingDevice::with_host_script(FakeHostScript::Warm);
        device.machine.respond(
            "ADB_FORWARD_LIST",
            &format!("serial tcp:{} {HOST_SOCKET_SPEC}", device.fake_host.port),
        );
        device.holds_stamps(
            Some(device.payload.stamp(PayloadPart::Libraries)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        let argv = device.argv();
        let all: Vec<&str> = argv.lines().collect();
        assert_eq!(all[0], "start-server", "{argv}");
        let lines = &all[1..];
        assert_eq!(lines.len(), 5, "{argv}");
        let concurrent = &lines[..3];
        assert!(
            concurrent.contains(&"-s serial shell date '+%m-%d %H:%M:%S.000'"),
            "{argv}"
        );
        assert!(
            concurrent.contains(
                &"-s serial shell pm list packages --show-versioncode \
                  dev.waterui.hydrolysis.preview"
            ),
            "{argv}"
        );
        assert!(
            concurrent
                .iter()
                .any(|line| line.starts_with("-s serial shell -T sh -c")
                    && line.ends_with(PREPARE_ARGS)),
            "{argv}"
        );
        assert_eq!(lines[3], "-s serial forward --list", "{argv}");
        assert_eq!(
            lines[4],
            "-s serial shell -T run-as dev.waterui.hydrolysis.preview cat \
             files/waterui-preview/out/preview.png",
            "{argv}"
        );
        assert!(
            !argv.contains("am ") && !argv.contains("forward tcp"),
            "no host start and no new forward on a warm run: {argv}"
        );
        assert_eq!(
            device.fake_host.requests(),
            [concat!(
                r#"{"type":"render","run_config":"waterui-preview/preview-run.json","#,
                r#""assets_root":"waterui-preview/payload/resources/waterui_assets"}"#,
                "\n"
            )],
            "the render request on the wire"
        );
    }

    /// A live host greeting with a different payload stamp is a defined
    /// transition, never a silent reuse: the package is force-stopped and a
    /// fresh host started — `force-stop` before `instrument` — and the run
    /// still lands its frame.
    #[test]
    #[cfg(unix)]
    fn a_stale_host_is_stopped_then_replaced() {
        let device =
            RenderingDevice::with_host_script(FakeHostScript::StaleThenWarm("0".repeat(64)));
        device.holds_stamps(
            Some(device.payload.stamp(PayloadPart::Libraries)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );

        smol::block_on(device.run(&device.machine.root().join("preview.png")));

        let argv = device.argv();
        let lines: Vec<&str> = argv.lines().collect();
        let force_stop = line_of(&lines, "am force-stop dev.waterui.hydrolysis.preview");
        let instrument = line_of(&lines, "am instrument");
        assert!(
            force_stop < instrument,
            "the stale host is stopped before the fresh one starts: {argv}"
        );
        assert!(
            lines[line_of(&lines, "am instrument")].contains("-e payloadStamp"),
            "{argv}"
        );
        assert!(
            std::fs::read(device.machine.root().join("preview.png"))
                .expect("read")
                .starts_with(PNG_SIGNATURE),
            "the pulled PNG landed"
        );
    }

    /// A `failed` reply carries the host's own report; the run's error
    /// surfaces it, alongside the logcat tail.
    #[test]
    #[cfg(unix)]
    fn a_host_render_failure_surfaces_its_report() {
        let device = RenderingDevice::with_host_script(FakeHostScript::WarmFailing);
        device.holds_stamps(
            Some(device.payload.stamp(PayloadPart::Libraries)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );

        let error = smol::block_on(device.try_run(&device.machine.root().join("preview.png")))
            .expect_err("a failed reply must fail the run");
        assert!(error.to_string().contains("preview exploded"), "{error:#}");
    }

    /// A fresh host that logs a failed start fails the run with that line
    /// and the crash log — the wait ends on the host's own report, and no
    /// render is sent.
    #[test]
    #[cfg(unix)]
    fn a_failed_host_start_surfaces_its_log_line() {
        let device = RenderingDevice::new();
        device.holds_stamps(
            Some(device.payload.stamp(PayloadPart::Libraries)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );
        device.machine.respond(
            "ADB_LOGCAT",
            &format!(
                "01-01 00:00:01.000  4242  4242 E HydrolysisPreview: preview host failed to \
                 start: stamp {}",
                device.payload.stamp(PayloadPart::Libraries)
            ),
        );

        let error = smol::block_on(device.try_run(&device.machine.root().join("preview.png")))
            .expect_err("a failed start must fail the run");
        assert!(
            error
                .to_string()
                .contains("the preview host failed to start: 01-01 00:00:01.000"),
            "{error:#}"
        );
        assert!(device.fake_host.requests().is_empty(), "no render was sent");
    }

    /// A greeting that is not a frame fails the run naming what arrived —
    /// never a restart that hides it.
    #[test]
    #[cfg(unix)]
    fn a_malformed_greeting_names_what_arrived() {
        let device = RenderingDevice::with_host_script(FakeHostScript::Garbled);
        device.holds_stamps(
            Some(device.payload.stamp(PayloadPart::Libraries)),
            Some(device.payload.stamp(PayloadPart::Resources)),
        );

        let error = smol::block_on(device.try_run(&device.machine.root().join("preview.png")))
            .expect_err("a garbled greeting must fail the run");
        assert!(
            error
                .to_string()
                .contains("the preview host sent a malformed frame: not a frame"),
            "{error:#}"
        );
        assert!(
            !device.argv().contains("am "),
            "nothing was stopped or started: {}",
            device.argv()
        );
    }

    /// A device whose shell commands run the production scripts for real —
    /// the host's `sh`, `cp`, `find` — with a `run-as` that enters a scratch
    /// private-files directory and a scratch `/data/local/tmp`, so the part
    /// stamps' guarantee is checked against the sequence the device runs.
    #[cfg(unix)]
    struct ScriptedDevice {
        machine: TestMachine,
        adb: Adb,
        payload: DevicePayload,
        /// The scratch private-files directory `run-as` enters.
        data: PathBuf,
        /// The scratch `/data/local/tmp`.
        tmp: PathBuf,
        /// The shims every scripted host puts ahead of the system tools.
        shims: PathBuf,
    }

    #[cfg(unix)]
    impl ScriptedDevice {
        fn new() -> Self {
            let machine = TestMachine::new();
            let sdk = machine.install_android_sdk();
            machine.install_adb();
            let data = machine.dir("device-data");
            let tmp = machine.dir("device-tmp");
            let payload = staged_payload(&machine);
            let adb = smol::block_on(Adb::locate(&machine.host([("ANDROID_SDK_ROOT", &sdk)])))
                .expect("fake adb must locate");
            let shims = machine.dir("device-shims");
            write_shim(
                &shims,
                "run-as",
                "shift\ncd \"$WATERUI_FAKE_DEVICE_DATA\" || exit 1\nexec \"$@\"",
            );
            Self {
                machine,
                adb,
                payload,
                data,
                tmp,
                shims,
            }
        }

        /// A host reaching the scripted device, with the shims in `extra`
        /// — a directory — ahead of the device's tools.
        fn host(&self, extra: Option<&Path>) -> Host {
            let sdk = self.machine.root().join("sdk");
            let path = extra.map_or_else(
                || self.shims.display().to_string(),
                |extra| format!("{}:{}", extra.display(), self.shims.display()),
            );
            self.machine.host([
                ("ANDROID_SDK_ROOT", sdk.as_os_str()),
                ("WATERUI_FAKE_DEVICE_DATA", self.data.as_os_str()),
                ("WATERUI_FAKE_DEVICE_TMP", self.tmp.as_os_str()),
                ("WATERUI_FAKE_DEVICE_PATH", std::ffi::OsStr::new(&path)),
            ])
        }

        fn run_dir(&self) -> PathBuf {
            self.data.join(FILES_RUN_DIR)
        }

        /// The push staging directory as the scratch `/data/local/tmp`
        /// holds it.
        fn staging(&self) -> PathBuf {
            self.tmp.join("waterui-preview")
        }

        /// The stamp the device holds for `part`, `None` when it holds none.
        fn stamp(&self, part: PayloadPart) -> Option<String> {
            match std::fs::read_to_string(self.run_dir().join(part.stamp_file())) {
                Ok(stamp) => Some(stamp),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => panic!("read the device stamp: {error}"),
            }
        }

        /// A directory of its own whose `name` is a shell script running
        /// `body`, to put ahead of the device's tools.
        fn shim(&self, name: &str, body: &str) -> PathBuf {
            let dir = self.machine.dir(format!("shim-{name}"));
            write_shim(&dir, name, body);
            dir
        }

        /// Prepare a run under `host`, answering the stamps it read back.
        fn prepare(&self, host: &Host) -> HeldStamps {
            smol::block_on(prepare_run(host, &self.adb, "serial", RUN_CONFIG))
                .expect("the run prepares")
        }

        /// Push and install `parts` under `host`.
        fn push(&self, host: &Host, parts: &[PayloadPart]) -> Result<()> {
            smol::block_on(push_payload(
                host,
                &self.adb,
                "serial",
                &self.payload,
                parts,
            ))
        }

        /// Prepare and push once with nothing in the way, so the device
        /// holds the whole payload and its stamps.
        fn install_whole_payload(&self) {
            let host = self.host(None);
            let held = self.prepare(&host);
            assert_eq!(
                self.payload.stale_parts(&held),
                PayloadPart::ALL,
                "a fresh device holds no stamp"
            );
            self.push(&host, &PayloadPart::ALL)
                .expect("the whole payload installs");
        }
    }

    /// Write an executable shell script `name` running `body` into `dir`.
    #[cfg(unix)]
    fn write_shim(dir: &Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the shim");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make the shim executable");
    }

    /// The sorted entry names of `dir`.
    #[cfg(unix)]
    fn entry_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("list the directory")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort_unstable();
        names
    }

    /// The preparation clears a stale push staging directory and whatever
    /// the run directory's layout does not name; a whole push then installs
    /// every part — libraries read-only — and its stamp, which the next
    /// preparation reads back while it clears the staging copy the push
    /// left. A stamp lands only after every step before it succeeded: a
    /// failing `chmod` leaves the part without one and the other part's
    /// untouched.
    #[test]
    #[cfg(unix)]
    fn a_pushed_part_installs_its_stamp_last() {
        let device = ScriptedDevice::new();
        let run_dir = device.run_dir();
        std::fs::create_dir_all(run_dir.join("old-layout/lib")).expect("an old layout");
        std::fs::write(run_dir.join("payload.stamp"), "stale").expect("an old stamp");
        std::fs::create_dir_all(run_dir.join(PAYLOAD_DIR)).expect("an old payload");
        std::fs::write(run_dir.join("payload/SHA256SUMS"), "stale").expect("an old manifest");
        std::fs::create_dir_all(device.staging().join("lib")).expect("a stale staging dir");

        device.install_whole_payload();
        assert_eq!(
            entry_names(&run_dir),
            [
                "lib.stamp",
                "out",
                "payload",
                "preview-run.json",
                "resources.stamp"
            ],
            "the run directory holds only what its layout names"
        );
        assert_eq!(
            entry_names(&run_dir.join(PAYLOAD_DIR)),
            ["lib", "resources"],
            "the payload holds only its parts"
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
        assert_eq!(
            std::fs::read(run_dir.join("payload/resources/waterui_assets/images/logo.png"))
                .expect("the asset landed"),
            b"logo-bytes"
        );
        assert!(
            device.staging().join("lib/libx.so").exists(),
            "the push staged the libraries"
        );
        let held = device.prepare(&device.host(None));
        assert!(
            device.payload.stale_parts(&held).is_empty(),
            "the preparation reads every installed stamp back: {held:?}"
        );
        assert!(
            !device.staging().exists(),
            "the preparation clears the staging a previous push left"
        );

        let failing_chmod = device.shim("chmod", "exit 1");
        let error = device
            .push(
                &device.host(Some(&failing_chmod)),
                &[PayloadPart::Libraries],
            )
            .expect_err("a failing chmod fails the install");
        assert!(format!("{error:#}").contains("status"), "{error:#}");
        assert_eq!(device.stamp(PayloadPart::Libraries), None);
        assert_eq!(
            device.stamp(PayloadPart::Resources).as_deref(),
            Some(device.payload.stamp(PayloadPart::Resources)),
            "a part not pushed keeps its stamp"
        );
    }

    /// A delete of the previous part that dies part-way — the libraries
    /// gone, the directory not — leaves no stamp that could name the
    /// remains.
    #[test]
    #[cfg(unix)]
    fn an_interrupted_part_delete_leaves_no_matching_stamp() {
        let device = ScriptedDevice::new();
        device.install_whole_payload();

        let dying_rm = device.shim(
            "rm",
            "for arg; do\n\
             \x20   if [ \"$arg\" = payload/lib ]; then /bin/rm -f payload/lib/*; exit 1; fi\n\
             done\n\
             exec /bin/rm \"$@\"",
        );
        let error = device
            .push(&device.host(Some(&dying_rm)), &[PayloadPart::Libraries])
            .expect_err("an interrupted delete fails the install");
        assert!(format!("{error:#}").contains("status"), "{error:#}");
        assert!(
            !device.run_dir().join("payload/lib/libx.so").exists(),
            "the delete got part-way"
        );
        assert_eq!(device.stamp(PayloadPart::Libraries), None);
    }

    /// A host at another version, or none, is installed once the version
    /// query, the clock read and the speculative preparation are done, and
    /// the run is prepared again against the installed host before anything
    /// ships.
    #[test]
    #[cfg(unix)]
    fn a_different_or_absent_version_code_reinstalls_then_prepares_again() {
        for response in ["package:dev.waterui.hydrolysis.preview versionCode:6", ""] {
            let device = RenderingDevice::new();
            device.machine.respond("ADB_PM_PACKAGES", response);

            smol::block_on(device.run(&device.machine.root().join("preview.png")));

            let argv = device.argv();
            let lines: Vec<&str> = argv.lines().collect();
            let install = line_of(&lines, " install -r ");
            let prepares: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.ends_with(PREPARE_ARGS))
                .map(|(index, _)| index)
                .collect();
            assert!(
                matches!(prepares.as_slice(), [first, second] if *first < install && install < *second),
                "one preparation before the install and one after it for `{response}`: {argv}"
            );
            assert!(
                install < line_of(&lines, " push "),
                "the payload ships to the installed host: {argv}"
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
    /// device-side run: they share the run directory — `out/` is cleared
    /// per run — and the one live host, so the second run's preparation and
    /// render cannot interleave the first run's.
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
        let error = smol::block_on(async { install_host(&host, &adb, "serial", &apk, 7).await })
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
    fn an_adb_timeout_names_the_invocation() {
        let (machine, host, _log) = adb_test_machine();
        let adb = smol::block_on(Adb::locate(&host)).expect("fake adb must locate");
        let host = machine.host([("WATERUI_FAKE_ADB_HANG", "1")]);
        let error = smol::block_on(async {
            adb.shell_run(
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
