//! The preview entry's own suite — `preview::run` end to end, one run per
//! child process, for the `AppKit` realization on macOS and the `UIKit`
//! realization on Mac Catalyst.
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

// On Mac Catalyst the executable is exec'd without a bundle: its embedded
// `Info.plist` puts `UIKit` under the iPad idiom, the contract a generated
// preview binary keeps the same way.
#[cfg(catalyst)]
waterui_apple::preview_info_plist!();

// The suite only exists where the preview entry does — `AppKit` and Mac
// Catalyst. A target must still own a `main` on every other OS, so the
// whole suite sits behind the gate and the entry point stays
// unconditional.
#[cfg(any(target_os = "macos", catalyst))]
mod suite {

    use std::error::Error as StdError;
    use std::ffi::c_void;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    #[cfg(target_os = "macos")]
    use cocoa_ui::MainThreadMarker;
    #[cfg(target_os = "macos")]
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
        DEFAULT_CONTENT_DEADLINE, PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode,
        RunConfigError,
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
            // A GPU-surface run waits on presented frames, which a
            // sleeping display never delivers (#3529) — the child holds
            // the display awake for its `preview::run`.
            #[cfg(target_os = "macos")]
            let _display_awake = cocoa_ui::native_test::DisplayAwake::hold();
            std::process::exit(child(&slug));
        }
        let mut args = Arguments::from_args();
        // Native objects may only be built on the real main thread; `run`
        // executes sequentially in the calling thread at one thread.
        args.test_threads = Some(1);
        libtest_mimic::run(&args, trials()).exit();
    }

    /// The real main thread — this binary runs every case there.
    #[cfg(target_os = "macos")]
    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("the suite runs on the process's main thread")
    }

    fn png_trial(name: &'static str, slug: &'static str, check: Option<fn(&Path)>) -> Trial {
        Trial::test(name, move || {
            run_trial(slug, Expect::Png);
            if let Some(check) = check {
                check(&output_dir().join(format!("{slug}.png")));
            }
            Ok(())
        })
    }

    fn failure_trial(name: &'static str, slug: &'static str, needle: &'static str) -> Trial {
        Trial::test(name, move || {
            run_trial(slug, Expect::Failure(needle));
            Ok(())
        })
    }

    /// The registered trials — one PNG per image run, written to
    /// `target/preview-captures/<trial>.png` for visual review.
    fn trials() -> Vec<Trial> {
        vec![
            png_trial(
                "preview::captures_the_issue_view_set_windowless",
                "view_set",
                None,
            ),
            png_trial("preview::awaits_an_image_asset_decode", "image_asset", None),
            // Its first pass runs at layout, before the view has a window;
            // joining the window changes the backing scale and rebuilds the
            // installed layer tree (#4209).
            #[cfg(feature = "canvas")]
            png_trial(
                "preview::captures_a_canvas_that_joins_a_window",
                "canvas",
                None,
            ),
            failure_trial(
                "preview::fails_naming_content_still_owed_at_the_deadline",
                "content_deadline",
                "still owes content after 200ms",
            ),
            failure_trial(
                "preview::fails_naming_content_that_failed_to_load",
                "content_failed",
                "failed to load its content",
            ),
            #[cfg(feature = "gpu_surface")]
            png_trial("preview::captures_gpu_surfaces_windowless", "gpu_set", None),
            png_trial(
                "preview::captures_liquid_glass_windowless",
                "liquid_glass",
                Some(assert_liquid_glass_png),
            ),
            #[cfg(feature = "gpu_surface")]
            png_trial(
                "preview::captures_liquid_glass_with_gpu_surface_windowless",
                "liquid_glass_gpu",
                Some(assert_liquid_glass_gpu_png),
            ),
            png_trial(
                "preview::captures_tabs_windowless",
                "tabs",
                Some(assert_tabs_png),
            ),
            png_trial(
                "preview::captures_navigation_windowless",
                "navigation",
                Some(assert_navigation_png),
            ),
            png_trial(
                "preview::captures_checkbox_toggle_windowless",
                "checkbox_toggle",
                Some(assert_checkbox_toggle_png),
            ),
            #[cfg(feature = "gpu_surface")]
            png_trial(
                "preview::offscreen_renderer_captures_tabs_nav_checkbox",
                "offscreen_tabs_nav_checkbox",
                None,
            ),
            failure_trial(
                "preview::rejects_a_scenario_run",
                "scenario",
                "apple preview supports image captures only",
            ),
            failure_trial(
                "preview::rejects_a_semantic_run",
                "semantic",
                "apple preview supports image captures only",
            ),
            failure_trial(
                "preview::errors_when_the_run_config_is_missing",
                "missing_config",
                "WATERUI_PREVIEW_RUN_CONFIG is not set",
            ),
            failure_trial(
                "preview::errors_when_the_run_config_is_malformed",
                "malformed_config",
                "cannot parse preview run configuration",
            ),
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
            "image_asset" => {
                image_run("image_asset", 200.0, 200.0, image_asset_view).map_err(Into::into)
            }
            #[cfg(feature = "canvas")]
            "canvas" => image_run("canvas", 200.0, 200.0, canvas_view).map_err(Into::into),
            "content_deadline" => content_run(
                "content_deadline",
                Duration::from_millis(200),
                never_loading_view,
            )
            .map_err(Into::into),
            "content_failed" => content_run(
                "content_failed",
                DEFAULT_CONTENT_DEADLINE,
                missing_image_view,
            )
            .map_err(Into::into),
            #[cfg(feature = "gpu_surface")]
            "gpu_set" => image_run("gpu_set", 390.0, 760.0, gpu_set).map_err(Into::into),
            "liquid_glass" => {
                image_run("liquid_glass", 280.0, 160.0, liquid_glass_view).map_err(Into::into)
            }
            #[cfg(feature = "gpu_surface")]
            "liquid_glass_gpu" => {
                image_run("liquid_glass_gpu", 280.0, 200.0, liquid_glass_gpu_view)
                    .map_err(Into::into)
            }
            "tabs" => image_run("tabs", 400.0, 260.0, tabs_view).map_err(Into::into),
            "navigation" => {
                image_run("navigation", 400.0, 260.0, navigation_view).map_err(Into::into)
            }
            "checkbox_toggle" => {
                image_run("checkbox_toggle", 220.0, 80.0, checkbox_toggle_view).map_err(Into::into)
            }
            #[cfg(feature = "gpu_surface")]
            "offscreen_tabs_nav_checkbox" => offscreen_tabs_nav_checkbox_run(),
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
    fn image_run<V: View>(
        slug: &str,
        width: f32,
        height: f32,
        view: impl FnOnce() -> V + 'static,
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
            assert_background_application();
        }
        result
    }

    /// The capture process holds the `Prohibited` activation policy and
    /// never took focus.
    #[cfg(target_os = "macos")]
    fn assert_background_application() {
        let application = NSApplication::sharedApplication(mtm());
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

    /// The capture process holds the `Prohibited` activation policy and
    /// never took focus — read from the process's `NSApplication` by name,
    /// since the iOS SDK declares no `AppKit` binding.
    #[cfg(catalyst)]
    fn assert_background_application() {
        use cocoa_ui::objc2::msg_send;
        use cocoa_ui::objc2::rc::Retained;
        use cocoa_ui::objc2::runtime::{AnyClass, AnyObject};
        /// `NSApplicationActivationPolicyProhibited`.
        const PROHIBITED: isize = 2;
        let class = AnyClass::get(c"NSApplication").expect("Catalyst loads AppKit");
        // SAFETY: `+sharedApplication`, `-activationPolicy` and `-isActive`
        // are `NSApplication`'s, called on the main thread with the
        // signatures `AppKit` declares.
        let (policy, active) = unsafe {
            let application: Retained<AnyObject> = msg_send![class, sharedApplication];
            let policy: isize = msg_send![&*application, activationPolicy];
            let active: bool = msg_send![&*application, isActive];
            (policy, active)
        };
        assert_eq!(
            policy, PROHIBITED,
            "the capture process must hold the Prohibited activation policy"
        );
        assert!(!active, "the capture process must never take focus");
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
    fn run_with_config<V: View>(
        slug: &str,
        mode: PreviewRunMode,
        width: f32,
        height: f32,
        view: impl FnOnce() -> V + 'static,
    ) -> Result<(), PreviewError> {
        run_with_deadline(slug, mode, width, height, DEFAULT_CONTENT_DEADLINE, view)
    }

    /// [`run_with_config`] with an explicit content deadline.
    fn run_with_deadline<V: View>(
        slug: &str,
        mode: PreviewRunMode,
        width: f32,
        height: f32,
        content_deadline: Duration,
        view: impl FnOnce() -> V + 'static,
    ) -> Result<(), PreviewError> {
        let dir = output_dir();
        fs::create_dir_all(&dir).expect("the preview capture directory is creatable");
        let config_path = dir.join(format!("{slug}.run-config.json"));
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&PreviewRunConfig::new(
                width,
                height,
                mode,
                content_deadline,
            ))
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
            move || AnyView::new(view()),
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
    #[cfg(target_os = "macos")]
    fn frontmost_application() -> Option<i64> {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| i64::from(app.processIdentifier()))
    }

    /// See the macOS function — `NSWorkspace` read by name, since the iOS
    /// SDK declares no `AppKit` binding.
    #[cfg(catalyst)]
    fn frontmost_application() -> Option<i64> {
        use cocoa_ui::objc2::msg_send;
        use cocoa_ui::objc2::rc::Retained;
        use cocoa_ui::objc2::runtime::{AnyClass, AnyObject};
        let class = AnyClass::get(c"NSWorkspace").expect("Catalyst loads AppKit");
        // SAFETY: `+sharedWorkspace`, `-frontmostApplication` and
        // `-processIdentifier` are `NSWorkspace`'s and
        // `NSRunningApplication`'s, called with the signatures `AppKit`
        // declares.
        unsafe {
            let workspace: Retained<AnyObject> = msg_send![class, sharedWorkspace];
            let frontmost: Option<Retained<AnyObject>> =
                msg_send![&*workspace, frontmostApplication];
            frontmost.map(|app| {
                let pid: libc::pid_t = msg_send![&*app, processIdentifier];
                i64::from(pid)
            })
        }
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

    /// The issue's plain native set: text and a wrapped paragraph, buttons
    /// both styles, toggle, slider, stepper, a text field with text, picker,
    /// progress, an image asset, filled shapes and a gradient, a material
    /// background and a scroll view.
    fn issue_view_set() -> impl View {
        use waterui::form::picker::picker;
        use waterui::gradient::Gradient;
        use waterui::shape::{Circle, Rectangle, RoundedRectangle, ShapeExt};

        // `image.png` is generated into the run's assets directory by
        // `run_with_config` before `run` mounts this view — the same
        // directory the run's explicit `ResourceContext` mounts as
        // `Bundle::main()`'s root. `ImageAsset` resolves there and loads
        // through `Photo`'s asynchronous decode, which the capture awaits
        // through the content ledger.
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
            waterui_apple::SfSymbol::STAR_FILL.size(48.0, 48.0),
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
    }

    /// One image asset, large enough to read: the capture must show the
    /// decoded image, not the empty slot its asynchronous load starts as.
    fn image_asset_view() -> impl View {
        waterui::ImageAsset::new(waterui::Bundle::main(), "image.png")
            .size(160.0, 160.0)
            .padding()
    }

    /// A red square drawn by a `Canvas`.
    #[cfg(feature = "canvas")]
    fn canvas_view() -> impl View {
        use waterui::color::WorkingColor;
        use waterui::graphics::canvas::Canvas;
        use waterui::layout::{Rect, Size};

        Canvas::new(|cx| {
            cx.fill(
                Rect::from_size(Size::new(100.0, 100.0)),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        })
        .intrinsic_size(Size::new(100.0, 100.0))
    }

    /// A `Suspense` whose content never arrives: the capture fails at its
    /// content deadline, naming it.
    fn never_loading_view() -> impl View {
        waterui::widget::suspense::Suspense::new(core::future::pending::<waterui::text::Text>())
            .loading(text("loading"))
    }

    /// An image asset whose file does not exist: its load fails and the
    /// capture reports the failure, naming it.
    fn missing_image_view() -> impl View {
        waterui::ImageAsset::new(waterui::Bundle::main(), "missing.png").size(48.0, 48.0)
    }

    /// A content-ledger run that must fail: an image run whose `view` owes
    /// content the capture never receives, under `content_deadline`.
    fn content_run<V: View>(
        slug: &str,
        content_deadline: Duration,
        view: impl FnOnce() -> V + 'static,
    ) -> Result<(), PreviewError> {
        run_with_deadline(
            slug,
            PreviewRunMode::Image {
                output: output_dir().join(format!("{slug}.png")),
            },
            200.0,
            200.0,
            content_deadline,
            view,
        )
    }

    /// The GPU-surface set: a shader canvas and filtered views — the content
    /// the `cacheDisplay` capture only shows once the surfaces' first frames
    /// have presented.
    #[cfg(feature = "gpu_surface")]
    fn gpu_set() -> impl View {
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
    }

    /// Decodes a written PNG file into top-down premultiplied RGBA8 pixels via
    /// `Core Graphics` (zero extra test dependencies on macOS and Mac Catalyst).
    fn decode_png_rgba(path: &Path) -> (Vec<u8>, usize, usize) {
        use core::ptr::null;

        use cocoa_ui::Rect;
        use cocoa_ui::bitmap::{bitmap_context, draw_image};
        use cocoa_ui::objc2_core_foundation::CFData;
        use cocoa_ui::objc2_core_graphics::{CGColorRenderingIntent, CGDataProvider, CGImage};

        let bytes = fs::read(path).expect("captured PNG is readable");
        let data = CFData::from_bytes(&bytes);
        let provider =
            CGDataProvider::with_cf_data(Some(&data)).expect("PNG data provider constructs");
        // SAFETY: `decode` is null and `provider` is a live `CGDataProvider` over the PNG bytes.
        let image = unsafe {
            CGImage::with_png_data_provider(
                Some(&provider),
                null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
        .expect("captured PNG decodes cleanly");
        let width = CGImage::width(Some(&image));
        let height = CGImage::height(Some(&image));
        let mut pixels = vec![0_u8; width * height * 4];
        let context =
            bitmap_context(&mut pixels, width, height).expect("RGBA8 bitmap context constructs");
        #[expect(
            clippy::cast_precision_loss,
            reason = "preview capture dimensions are a few hundred pixels"
        )]
        draw_image(
            &context,
            &image,
            Rect::new(0.0, 0.0, width as f64, height as f64),
        );
        (pixels, width, height)
    }

    /// Pure-native Liquid Glass scene: red/green split backdrop behind a
    /// prominent glass button and a regular material badge, driven with zero
    /// special API.
    fn liquid_glass_view() -> impl View {
        use waterui::color::Color;
        use waterui::shape::{Rectangle, ShapeExt};

        zstack((
            vstack((
                Rectangle.fill(Color::srgb_hex("#FF3B30")).height(80.0),
                Rectangle.fill(Color::srgb_hex("#34C759")).height(80.0),
            ))
            .spacing(0.0),
            vstack((
                button("Continue").style(ButtonStyle::BorderedProminent),
                text("Glass").padding().background(Material::Regular),
            ))
            .spacing(16.0),
        ))
    }

    fn assert_liquid_glass_png(path: &Path) {
        let (pixels, width, height) = decode_png_rgba(path);
        assert!(
            width >= 280 && height >= 160,
            "unexpected size {width}x{height}"
        );
        let pixel_at = |x: usize, y: usize| -> [u8; 4] {
            let offset = (y * width + x) * 4;
            [
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                pixels[offset + 3],
            ]
        };
        let top = pixel_at(4, 4);
        assert!(
            top[0] > 200 && top[1] < 110 && top[2] < 110 && top[3] == 255,
            "top-left must be red (top-down orientation), got {top:?}"
        );
        let bottom = pixel_at(4, height - 5);
        assert!(
            bottom[1] > 150 && bottom[0] < 110 && bottom[2] < 120 && bottom[3] == 255,
            "bottom-left must be green (top-down orientation), got {bottom:?}"
        );
        let (mut glass_blue, mut white_glyph) = (0_usize, 0_usize);
        for y in 0..height / 2 {
            for x in 0..width {
                let [r, g, b, a] = pixel_at(x, y);
                glass_blue += usize::from(a >= 200 && b >= 180 && g >= 70 && r <= 100);
                white_glyph += usize::from(a >= 200 && r >= 220 && g >= 220 && b >= 220);
            }
        }
        assert!(
            glass_blue > 500,
            "prominent glass button in pure-native preview must render real blue glass fill, got {glass_blue}"
        );
        assert!(
            white_glyph > 40,
            "prominent glass button in pure-native preview must render crisp white title glyphs, got {white_glyph}"
        );
    }

    /// Mixed native Liquid Glass + Metal `ShaderPaintView` scene: proves
    /// `ViewCapture`'s render-server path composites real native glass alongside
    /// GPU surfaces in one preview capture.
    #[cfg(feature = "gpu_surface")]
    fn liquid_glass_gpu_view() -> impl View {
        use waterui::graphics::ShaderPaintView;

        const SHADER: &str = include_str!("gpu_set.wgsl");
        vstack((
            button("Continue").style(ButtonStyle::BorderedProminent),
            ShaderPaintView::new(SHADER).size(240.0, 96.0),
        ))
        .spacing(16.0)
        .padding()
    }

    #[cfg(feature = "gpu_surface")]
    fn assert_liquid_glass_gpu_png(path: &Path) {
        let (pixels, width, height) = decode_png_rgba(path);
        assert!(
            width >= 280 && height >= 200,
            "unexpected size {width}x{height}"
        );
        let pixel_at = |x: usize, y: usize| -> [u8; 4] {
            let offset = (y * width + x) * 4;
            [
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                pixels[offset + 3],
            ]
        };
        let mut glass_blue = 0_usize;
        for y in 0..height / 2 {
            for x in 0..width {
                let [r, g, b, a] = pixel_at(x, y);
                glass_blue += usize::from(a >= 200 && b >= 180 && g >= 70 && r <= 100);
            }
        }
        assert!(
            glass_blue > 500,
            "prominent glass button alongside GPU surface must render real blue glass fill, got {glass_blue}"
        );
        // `gpu_set.wgsl` paints pink (top) -> blue (bottom) vertical bars
        // alternating at 0.6x and 1.0x brightness.
        let (mut pink_top, mut blue_bottom) = (0_usize, 0_usize);
        for y in (height / 3)..(height - 16) {
            for x in 20..(width - 20) {
                let [r, _, b, a] = pixel_at(x, y);
                if a == 255 {
                    pink_top += usize::from(r > 180 && b < 180 && r > b + 40);
                    blue_bottom += usize::from(b > 180 && r < 150 && b > r + 40);
                }
            }
        }
        assert!(
            pink_top > 500 && blue_bottom > 500,
            "ShaderPaintView gradient bars must composite alongside glass (pink_top={pink_top}, blue_bottom={blue_bottom})"
        );
    }

    fn tabs_view() -> impl View {
        use waterui::navigation::{NavigationView, Tab, Tabs};
        use waterui::theme::color::Background;

        let selection = Binding::container(0_usize);
        zstack((
            Background,
            Tabs::new(
                &selection,
                (
                    Tab::new("Home", || {
                        NavigationView::new("Home", text("Home Content").title())
                    }),
                    Tab::new("Search", || {
                        NavigationView::new("Search", text("Search Content").title())
                    }),
                ),
            ),
        ))
        .size(400.0, 260.0)
    }

    fn navigation_view() -> impl View {
        use waterui::navigation::{NavigationStack, NavigationView};
        use waterui::theme::color::Background;

        zstack((
            Background,
            NavigationStack::new(NavigationView::new("Root", text("Root Body").title())),
        ))
        .size(400.0, 260.0)
    }

    fn checkbox_toggle_view() -> impl View {
        use waterui::component::toggle::ToggleStyle;
        use waterui::theme::color::Background;

        let on = Binding::bool(true);
        zstack((
            Background,
            toggle("Wi-Fi", &on).style(ToggleStyle::Checkbox).padding(),
        ))
        .size(220.0, 80.0)
    }

    fn count_ink_pixels(pixels: &[u8], width: usize, height: usize, bg: [u8; 4]) -> usize {
        let mut count = 0_usize;
        for y in 0..height {
            for x in 0..width {
                let offset = (y * width + x) * 4;
                let dr = pixels[offset].abs_diff(bg[0]);
                let dg = pixels[offset + 1].abs_diff(bg[1]);
                let db = pixels[offset + 2].abs_diff(bg[2]);
                if usize::from(dr) + usize::from(dg) + usize::from(db) >= 90 {
                    count += 1;
                }
            }
        }
        count
    }

    fn assert_tabs_png(path: &Path) {
        let (pixels, width, height) = decode_png_rgba(path);
        let bg = [pixels[0], pixels[1], pixels[2], pixels[3]];
        let ink = count_ink_pixels(&pixels, width, height, bg);
        assert!(
            ink > 300,
            "tabs capture must render visible tab bar and selected pane content, got {ink} ink pixels in {width}x{height}"
        );
    }

    fn assert_navigation_png(path: &Path) {
        let (pixels, width, height) = decode_png_rgba(path);
        let bg = [pixels[0], pixels[1], pixels[2], pixels[3]];
        let ink = count_ink_pixels(&pixels, width, height, bg);
        assert!(
            ink > 300,
            "navigation capture must render visible navigation bar and root page content, got {ink} ink pixels in {width}x{height}"
        );
    }

    fn assert_checkbox_toggle_png(path: &Path) {
        let (pixels, width, height) = decode_png_rgba(path);
        // The checkbox sits to the left of the centered "Wi-Fi" label; require
        // a compact, clearly saturated accent-tinted checkmark box (rejecting
        // both an invisible box and UIButtonType::System's oversized legacy
        // selection plate) with a bright interior checkmark.
        let mut accent_px = 0_usize;
        let (mut min_x, mut max_x, mut min_y, mut max_y) =
            (usize::MAX, 0_usize, usize::MAX, 0_usize);
        for y in 0..height {
            for x in 0..(width / 2) {
                let offset = (y * width + x) * 4;
                let [r, g, b, a] = [
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ];
                if a >= 200 && b >= 140 && b > r.saturating_add(40) && g >= 60 {
                    accent_px += 1;
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
            }
        }
        assert!(
            accent_px > 200 && accent_px < 2200,
            "checked checkbox toggle must render a compact accent-tinted checkmark box, got {accent_px} accent pixels in {width}x{height}"
        );
        let mut inner_white = 0_usize;
        for y in (min_y + 2)..max_y.saturating_sub(1) {
            for x in (min_x + 2)..max_x.saturating_sub(1) {
                let offset = (y * width + x) * 4;
                let [r, g, b, a] = [
                    pixels[offset],
                    pixels[offset + 1],
                    pixels[offset + 2],
                    pixels[offset + 3],
                ];
                if a >= 200 && r >= 220 && g >= 220 && b >= 220 {
                    inner_white += 1;
                }
            }
        }
        assert!(
            inner_white > 25,
            "checked checkbox toggle must render a bright checkmark glyph inside the accent box, got {inner_white} white pixels"
        );
    }

    #[cfg(feature = "gpu_surface")]
    fn offscreen_tabs_nav_checkbox_run() -> Result<(), Box<dyn StdError>> {
        use cocoa_ui::MainThreadMarker;
        #[cfg(target_os = "macos")]
        use cocoa_ui::appkit::{ActivationPolicy, Application};
        #[cfg(catalyst)]
        use cocoa_ui::uikit::catalyst::BackgroundApplication;
        use waterui::layout::Size;
        #[cfg(target_os = "macos")]
        use waterui_apple::native_test_support::gpu_surface;
        #[cfg(catalyst)]
        use waterui_apple::native_test_support::initialize_process;
        use waterui_apple::native_test_support::view_renderer::{
            AppleOffscreenRenderer, GpuRuntime,
        };
        use waterui_apple::native_test_support::{block_on_main, install_gpu, window_environment};
        use waterui_capture::view_renderer::{RenderRequest, ViewRenderer};
        use waterui_preview_protocol::run::{Alpha, write_png};

        let mtm = MainThreadMarker::new().ok_or("must run on main thread")?;
        #[cfg(catalyst)]
        let _bg = {
            initialize_process();
            BackgroundApplication::bring_up(mtm)
        };
        #[cfg(target_os = "macos")]
        {
            gpu_surface::initialize_process();
            let app = Application::shared(mtm);
            if app.activation_policy() != ActivationPolicy::Prohibited {
                assert!(app.set_activation_policy(ActivationPolicy::Prohibited));
            }
        }
        let (mut env, _theme) = window_environment(mtm);
        let runtime = block_on_main(30.0, GpuRuntime::new())
            .map_err(|error| format!("GPU runtime failed: {error:?}"))?;
        install_gpu(&mut env, runtime);
        let renderer = AppleOffscreenRenderer::new(mtm);

        let dir = output_dir();
        let (tabs_png, nav_png, checkbox_png) = (
            dir.join("offscreen_tabs.png"),
            dir.join("offscreen_nav.png"),
            dir.join("offscreen_checkbox.png"),
        );
        let out_marker = dir.join("offscreen_tabs_nav_checkbox.png");

        block_on_main(60.0, async {
            let tabs = renderer
                .render_to_rgba(
                    tabs_view(),
                    &env,
                    RenderRequest::new(Size::new(400.0, 260.0), DEFAULT_CONTENT_DEADLINE),
                )
                .await?;
            let (rgba, w, h) = tabs.into_parts();
            write_png(&tabs_png, w, h, rgba, Alpha::Premultiplied)?;

            let nav = renderer
                .render_to_rgba(
                    navigation_view(),
                    &env,
                    RenderRequest::new(Size::new(400.0, 260.0), DEFAULT_CONTENT_DEADLINE),
                )
                .await?;
            let (rgba, w, h) = nav.into_parts();
            write_png(&nav_png, w, h, rgba, Alpha::Premultiplied)?;

            let cb = renderer
                .render_to_rgba(
                    checkbox_toggle_view(),
                    &env,
                    RenderRequest::new(Size::new(220.0, 80.0), DEFAULT_CONTENT_DEADLINE),
                )
                .await?;
            let (rgba, w, h) = cb.into_parts();
            write_png(&checkbox_png, w, h, rgba.clone(), Alpha::Premultiplied)?;
            write_png(&out_marker, w, h, rgba, Alpha::Premultiplied)?;
            Ok::<_, Box<dyn StdError>>(())
        })?;

        assert_tabs_png(&tabs_png);
        assert_navigation_png(&nav_png);
        assert_checkbox_toggle_png(&checkbox_png);
        assert_background_application();
        Ok(())
    }
}

#[cfg(any(target_os = "macos", catalyst))]
fn main() {
    suite::run();
}

#[cfg(not(any(target_os = "macos", catalyst)))]
fn main() {}
