//! The `AppKit` preview entry's own suite — `preview::run` end to end, one
//! run per child process.
//!
//! `run` initializes the process exactly once — startup installs the panic
//! hook, the tracing subscriber and the executors, and a second call fails
//! loudly — so every trial launches this executable again
//! (`std::env::current_exe()`) as a child that performs exactly one
//! `preview::run`, selected by `--preview-child <slug>`. The parent samples
//! the child's `CGWindowList` for the child's whole lifetime — no window it
//! owns may ever be on screen — compares the workspace's frontmost
//! application before and after, and checks the child's exit status, its
//! stderr and the PNG it wrote.

// The suite only exists on `AppKit` — the preview entry does too. A
// target must still own a `main` on every other OS, so the whole suite
// sits behind the gate and the entry point stays unconditional.
#[cfg(target_os = "macos")]
mod suite {

    use std::ffi::c_void;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use cocoa_ui::MainThreadMarker;
    use cocoa_ui::objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWorkspace};
    use cocoa_ui::objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFNumberType};
    use cocoa_ui::objc2_core_graphics::{
        CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowIsOnscreen,
        kCGWindowNumber, kCGWindowOwnerPID,
    };
    use libtest_mimic::{Arguments, Trial};
    use waterui::prelude::*;
    use waterui::{AnyView, ResourceContext};
    use waterui_apple::preview::PreviewError;
    use waterui_preview_protocol::run::{
        PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode, RunConfigError,
    };

    /// The argument that turns a spawned copy of this binary into a
    /// one-trial child — intercepted in `run` before `libtest_mimic`
    /// parses the command line.
    const CHILD_FLAG: &str = "--preview-child";

    /// The parent's bound on one child: a preview run answers in seconds,
    /// so a child alive past this is hung, not slow.
    const CHILD_DEADLINE: Duration = Duration::from_secs(60);

    /// How often the parent samples the child's window list — the only
    /// sleep the wait takes.
    const SAMPLE_INTERVAL: Duration = Duration::from_millis(10);

    /// The mimic entry — `main` below dispatches here on `AppKit`.
    pub fn run() {
        // The child role intercepts before `libtest_mimic` parses: one
        // spawned copy performs exactly one trial's `preview::run`.
        let mut args = std::env::args();
        let _ = args.next();
        if args.next().is_some_and(|flag| flag == CHILD_FLAG) {
            let slug = args
                .next()
                .expect("a spawned child names its trial after --preview-child");
            std::process::exit(child(&slug));
        }
        let mut args = Arguments::from_args();
        // `AppKit` objects may only be built on the real main thread; `run`
        // executes sequentially in the calling thread at one thread.
        args.test_threads = Some(1);
        libtest_mimic::run(&args, trials()).exit();
    }

    /// The real main thread — this binary runs every case there.
    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("the suite runs on the process's main thread")
    }

    /// The registered trials — one PNG per image run, written to
    /// `target/preview-captures/<trial>.png` for visual review.
    fn trials() -> Vec<Trial> {
        vec![
            Trial::test("preview::captures_the_issue_view_set_windowless", || {
                run_trial("view_set", Expect::Png);
                Ok(())
            }),
            #[cfg(feature = "gpu_surface")]
            Trial::test("preview::captures_gpu_surfaces_windowless", || {
                run_trial("gpu_set", Expect::Png);
                Ok(())
            }),
            Trial::test("preview::rejects_a_scenario_run", || {
                run_trial(
                    "scenario",
                    Expect::Failure("apple preview supports image captures only"),
                );
                Ok(())
            }),
            Trial::test("preview::rejects_a_semantic_run", || {
                run_trial(
                    "semantic",
                    Expect::Failure("apple preview supports image captures only"),
                );
                Ok(())
            }),
            Trial::test("preview::errors_when_the_run_config_is_missing", || {
                run_trial(
                    "missing_config",
                    Expect::Failure("WATERUI_PREVIEW_RUN_CONFIG is not set"),
                );
                Ok(())
            }),
            Trial::test("preview::errors_when_the_run_config_is_malformed", || {
                run_trial(
                    "malformed_config",
                    Expect::Failure("cannot parse preview run configuration"),
                );
                Ok(())
            }),
        ]
    }

    /// What the parent checks after the child exits.
    #[derive(Clone, Copy)]
    enum Expect {
        /// Exit status 0 and `target/preview-captures/<slug>.png` written
        /// non-empty.
        Png,
        /// A non-zero exit status and this error text on the child's
        /// stderr — the contract a generated preview binary keeps.
        Failure(&'static str),
    }

    /// One trial end to end: launch the child that performs the trial's
    /// `preview::run`, sample the child's window list for its whole
    /// lifetime on a bounded deadline, then check the windowless, focus
    /// and outcome contracts.
    fn run_trial(slug: &str, expect: Expect) {
        let frontmost = frontmost_application();
        assert_ne!(
            frontmost,
            Some(i64::from(std::process::id().cast_signed())),
            "the suite must not already be the frontmost application"
        );

        let dir = output_dir();
        fs::create_dir_all(&dir).expect("the preview capture directory is creatable");
        let stderr_path = dir.join(format!("{slug}.stderr"));
        let stderr_file = fs::File::create(&stderr_path).expect("the child's stderr file writes");
        let exe = std::env::current_exe().expect("the test executable resolves");
        // `WATERUI_LOG` at info turns the child's stderr file into a
        // readable run log — the written PNG's path lands there through
        // `tracing`.
        let mut child = std::process::Command::new(exe)
            .args([CHILD_FLAG, slug])
            .env("WATERUI_LOG", "info")
            .stderr(std::process::Stdio::from(stderr_file))
            .spawn()
            .expect("the trial child spawns");
        let child_pid = i64::from(child.id().cast_signed());

        let mut onscreen = Vec::new();
        let deadline = Instant::now() + CHILD_DEADLINE;
        let status = loop {
            onscreen.extend(onscreen_window_numbers(child_pid));
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    panic!("waiting for the {slug} child failed: {error}");
                }
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!(
                    "the {slug} child outlived its {}-second deadline",
                    CHILD_DEADLINE.as_secs()
                );
            }
            std::thread::sleep(SAMPLE_INTERVAL);
        };
        assert!(
            onscreen.is_empty(),
            "the {slug} child ordered windows on screen: {onscreen:?}"
        );
        assert_eq!(
            frontmost_application(),
            frontmost,
            "the {slug} trial must never change the frontmost application"
        );

        let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
        match expect {
            Expect::Png => {
                assert!(
                    status.success(),
                    "the {slug} child exits 0, got {status:?}; stderr:\n{stderr}"
                );
                let output = dir.join(format!("{slug}.png"));
                let bytes = fs::metadata(&output)
                    .unwrap_or_else(|error| {
                        panic!(
                            "the capture PNG exists at {}: {error}; stderr:\n{stderr}",
                            output.display()
                        )
                    })
                    .len();
                assert!(
                    bytes > 0,
                    "the capture PNG at {} is not empty",
                    output.display()
                );
            }
            Expect::Failure(text) => {
                assert!(
                    !status.success(),
                    "the {slug} child exits non-zero, got {status:?}"
                );
                assert!(
                    stderr.contains(text),
                    "the {slug} child's stderr carries the error — stderr:\n{stderr}"
                );
            }
        }
    }

    /// Runs one trial's work inside a child process — returns its exit
    /// code: 0 on success; on an error the message reaches stderr and the
    /// process exits non-zero, the way a generated preview binary's
    /// `main` reports it.
    fn child(slug: &str) -> i32 {
        let result: Result<(), Box<dyn std::error::Error>> = match slug {
            "view_set" => image_run("view_set", 390.0, 760.0, issue_view_set).map_err(Into::into),
            #[cfg(feature = "gpu_surface")]
            "gpu_set" => image_run("gpu_set", 390.0, 760.0, gpu_set).map_err(Into::into),
            "scenario" => error_run("scenario", |dir| PreviewRunMode::Scenario {
                output_dir: dir.clone(),
                captures_ms: vec![0],
                events: vec![],
            })
            .map_err(Into::into),
            "semantic" => {
                error_run("semantic", |_dir| PreviewRunMode::Semantic).map_err(Into::into)
            }
            "missing_config" => missing_config_run().map_err(Into::into),
            "malformed_config" => malformed_config_run().map_err(Into::into),
            other => panic!("unknown preview child trial {other}"),
        };
        match result {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("{error}");
                1
            }
        }
    }

    /// One image run, end to end: writes the run-configuration file `run`
    /// would be handed, lets `run` reload it through `PREVIEW_RUN_CONFIG_ENV`,
    /// then checks the captured process's activation contract and names the
    /// written PNG through `tracing`.
    fn image_run(
        slug: &str,
        width: f32,
        height: f32,
        view: impl FnOnce() -> AnyView + 'static,
    ) -> Result<(), PreviewError> {
        let output = output_dir().join(format!("{slug}.png"));
        let result = run_with_config(
            slug,
            PreviewRunMode::Image {
                output: output.clone(),
            },
            width,
            height,
            view,
        );
        if result.is_ok() {
            tracing::info!("preview capture written to {}", output.display());
            let mtm = mtm();
            let application = NSApplication::sharedApplication(mtm);
            assert_eq!(
                application.activationPolicy(),
                NSApplicationActivationPolicy::Prohibited,
                "the capture process must hold the Prohibited activation policy"
            );
            assert!(
                !application.isActive(),
                "the capture process must never take focus"
            );
        }
        result
    }

    /// A run that must fail: `mode` is written to a real run-configuration
    /// file like an image run's, and `preview::run`'s `Err` propagates to
    /// the child's stderr and exit status. The view builder never runs —
    /// an unsupported mode answers before the mount.
    fn error_run(
        slug: &str,
        mode: impl FnOnce(&PathBuf) -> PreviewRunMode,
    ) -> Result<(), PreviewError> {
        let dir = output_dir();
        run_with_config(slug, mode(&dir), 848.0, 120.0, issue_view_set)
    }

    /// `load_from_env` with the variable removed — the missing-config path
    /// a generated binary fails on.
    fn missing_config_run() -> Result<(), RunConfigError> {
        // SAFETY: the child owns its process on `main` — nothing else
        // reads the environment while the variable changes.
        unsafe { std::env::remove_var(PREVIEW_RUN_CONFIG_ENV) };
        PreviewRunConfig::load_from_env().map(|_| ())
    }

    /// `load_from_env` over a malformed file — the parse-error path a
    /// generated binary fails on.
    fn malformed_config_run() -> Result<(), RunConfigError> {
        let dir = output_dir();
        fs::create_dir_all(&dir).expect("the preview capture directory is creatable");
        let config_path = dir.join("malformed_config.run-config.json");
        fs::write(&config_path, "{ not json").expect("the malformed file writes");
        // SAFETY: the child owns its process on `main`.
        unsafe { std::env::set_var(PREVIEW_RUN_CONFIG_ENV, &config_path) };
        PreviewRunConfig::load_from_env().map(|_| ())
    }

    /// Drives `preview::run` once against a `mode`/`size` configuration,
    /// written to a real run-configuration file and reloaded through
    /// `PREVIEW_RUN_CONFIG_ENV` — the path a generated preview binary
    /// takes. Also materializes the image asset's directory: the suite's
    /// `ImageAsset` resolves `image.png` against the `ResourceContext`
    /// `resources` gives the run.
    fn run_with_config(
        slug: &str,
        mode: PreviewRunMode,
        width: f32,
        height: f32,
        view: impl FnOnce() -> AnyView + 'static,
    ) -> Result<(), PreviewError> {
        let dir = output_dir();
        fs::create_dir_all(&dir).expect("the preview capture directory is creatable");
        let config_path = dir.join(format!("{slug}.run-config.json"));
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&PreviewRunConfig {
                width,
                height,
                mode,
            })
            .expect("a run configuration serializes"),
        )
        .expect("the run configuration writes");
        // SAFETY: the child owns its process on `main` — `run` has not
        // brought the executors up yet, so no other thread observes the
        // environment while the variable changes.
        unsafe { std::env::set_var(PREVIEW_RUN_CONFIG_ENV, &config_path) };
        let config =
            PreviewRunConfig::load_from_env().expect("the written run configuration parses");

        let assets_dir = dir.join("assets");
        fs::create_dir_all(&assets_dir).expect("the asset directory is creatable");
        write_test_image(&assets_dir.join("image.png"));
        let fonts_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("Tests/fonts");
        waterui_apple::preview::run(
            |env| env,
            view,
            ResourceContext::new(assets_dir, fonts_dir),
            config,
        )
    }

    /// The directory suite artifacts land in — `target/preview-captures`,
    /// so the run's own build output is all it leaves behind.
    fn output_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/preview-captures")
    }

    /// Writes a 16x16 opaque square PNG — generated at run time, so no binary
    /// asset sits in the repository.
    fn write_test_image(path: &Path) {
        let mut pixels = vec![0_u8; 16 * 16 * 4];
        for chunk in pixels.as_chunks_mut::<4>().0 {
            chunk.copy_from_slice(&[235, 128, 40, 255]);
        }
        let png = waterui_preview_protocol::run::encode_png(16, 16, pixels)
            .expect("a 16x16 RGBA image encodes");
        fs::write(path, png).expect("the generated image asset writes");
    }

    /// `pid` of the workspace's frontmost application — `NSWorkspace`'s
    /// answer for "who holds focus".
    fn frontmost_application() -> Option<i64> {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| i64::from(app.processIdentifier()))
    }

    /// `kCGWindowNumber` of every onscreen window `pid` owns — empty when
    /// the process holds only windows it never ordered.
    fn onscreen_window_numbers(pid: i64) -> Vec<i64> {
        let windows = CGWindowListCopyWindowInfo(
            CGWindowListOption(
                CGWindowListOption::OptionAll.0 | CGWindowListOption::ExcludeDesktopElements.0,
            ),
            kCGNullWindowID,
        )
        .expect("the window server answers the window list");
        let mut onscreen = Vec::new();
        for index in 0..windows.count() {
            // SAFETY: the window list holds CFDictionary entries.
            let info = unsafe { &*windows.value_at_index(index).cast::<CFDictionary>() };
            // SAFETY: `kCGWindowOwnerPID` is an extern CFString the window
            // server owns.
            if window_entry_i64(info, unsafe { kCGWindowOwnerPID }) != Some(pid) {
                continue;
            }
            // SAFETY: `kCGWindowIsOnscreen` is an extern CFString the window
            // server owns; a present entry is a CFBoolean.
            let value =
                unsafe { info.value(std::ptr::from_ref(kCGWindowIsOnscreen).cast::<c_void>()) };
            // SAFETY: a present `IsOnscreen` entry is a CFBoolean the
            // dictionary owns — the borrow lives as long as `info`.
            if !value.is_null() && unsafe { (*value.cast::<CFBoolean>()).value() } {
                // SAFETY: `kCGWindowNumber` is an extern CFString the window
                // server owns.
                onscreen
                    .push(window_entry_i64(info, unsafe { kCGWindowNumber }).unwrap_or_default());
            }
        }
        onscreen
    }

    /// A `CFNumber` entry of a window-list dictionary, or `None` when absent
    /// — `kCGWindowOwnerPID`/`kCGWindowNumber`-style numeric values.
    fn window_entry_i64(
        info: &cocoa_ui::objc2_core_foundation::CFDictionary,
        key: &'static cocoa_ui::objc2_core_foundation::CFString,
    ) -> Option<i64> {
        // SAFETY: `key` is a live CFString constant.
        let value = unsafe { info.value(std::ptr::from_ref(key).cast::<c_void>()) };
        if value.is_null() {
            return None;
        }
        let mut number = 0i64;
        // SAFETY: the entry is a CFNumber and `number` is a live i64.
        let read = unsafe {
            (*value.cast::<CFNumber>())
                .value(CFNumberType::SInt64Type, (&raw mut number).cast::<c_void>())
        };
        read.then_some(number)
    }

    /// The issue's plain-`AppKit` set: text and a wrapped paragraph, buttons
    /// both styles, toggle, slider, stepper, a text field with text, picker,
    /// progress, an image asset, filled shapes and a gradient, a material
    /// background and a scroll view.
    fn issue_view_set() -> AnyView {
        use waterui::form::picker::picker;
        use waterui::gradient::Gradient;
        use waterui::shape::{Circle, Rectangle, RoundedRectangle, ShapeExt};

        // `image.png` is generated into the run's assets directory by
        // `run_with_config` before `run` mounts this view — the same
        // directory the run's explicit `ResourceContext` mounts as
        // `Bundle::main()`'s root. `ImageAsset` resolves there but loads
        // through `Photo`'s asynchronous decode, which this
        // first-presented-frames capture does not wait on — its blank
        // slot is water-rs/waterui#2149, kept visible here rather than
        // worked around.
        let wifi = Binding::bool(true);
        let level = Binding::f64(0.6);
        let count = Binding::i32(1);
        let email = Binding::container(Str::from_static("user@example.com"));
        let choice = Binding::container("M");
        let sky = waterui::color::Srgb::from_hex("#38BDF8").resolve();
        let rose = waterui::color::Srgb::from_hex("#FB7185").resolve();

        vstack((
        text("Preview Set").title(),
        text("monospaced styled text").monospaced(),
        text("A longer paragraph that is meant to wrap across several lines once the column gets narrow, so the capture shows real line breaking rather than a single clipped run."),
        hstack((
            button("Bordered"),
            button("Plain").style(ButtonStyle::Plain),
        ))
        .spacing(12.0),
        toggle("Wi-Fi", &wifi),
        slider("Volume", &level),
        hstack((stepper(text!("Count: {count}"), &count), spacer())),
        field("Email", &email),
        picker(
            "Size",
            [
                text("Small").tag("S"),
                text("Medium").tag("M"),
                text("Large").tag("L"),
            ],
            &choice,
        ),
        progress(0.35).label("Downloading"),
        hstack((
            waterui::icon::SystemIcon::new("star.fill").size(48.0, 48.0),
            waterui::ImageAsset::new(waterui::Bundle::main(), "image.png").size(48.0, 48.0),
            Circle.fill(Color::srgb_hex("#3B82F6")).size(48.0, 48.0),
            Rectangle.fill(Color::srgb_hex("#10B981")).size(64.0, 48.0),
            RoundedRectangle::new(0.25)
                .fill(Color::srgb_hex("#F59E0B"))
                .size(64.0, 48.0),
            Gradient::linear(vec![(0.0, rose), (1.0, sky)], [0.0, 0.0], [1.0, 1.0])
                .size(64.0, 48.0),
        ))
        .spacing(8.0),
        text("material row").padding().background(Material::Regular),
        scroll(vstack((
            text("scroll row 1"),
            text("scroll row 2"),
            text("scroll row 3"),
            text("scroll row 4"),
            text("scroll row 5"),
            text("scroll row 6"),
        )))
        .height(80.0),
    ))
    .padding()
    .anyview()
    }

    /// The GPU-surface set: a shader canvas and filtered views — the content
    /// the `cacheDisplay` capture only shows once the surfaces' first frames
    /// have presented.
    #[cfg(feature = "gpu_surface")]
    fn gpu_set() -> AnyView {
        // The checkerboard shader lives beside the suite — shaders in separate
        // files.
        const SHADER: &str = include_str!("gpu_set.wgsl");
        vstack((
            text("GPU canvas + filtered views").title(),
            waterui::graphics::ShaderPaintView::new(SHADER).size(320.0, 96.0),
            text("filtered: grayscale + blur")
                .grayscale(0.8_f32)
                .blur(2.0_f32),
            text("filtered: hue rotation").hue_rotation(120.0_f32),
        ))
        .padding()
        .anyview()
    }
}

#[cfg(target_os = "macos")]
fn main() {
    suite::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {}
