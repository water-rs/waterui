//! The `AppKit` preview entry's own suite — `preview::run` end to end in
//! a process no other suite shares.
//!
//! `native`'s scroll cases order real windows on screen, so a suite that
//! must prove the process never does cannot live there. Every run trial
//! drives `preview::run` against a real run-configuration file, so
//! `compose`, the `CFRunLoop` stop, the PNG write and the error path are
//! all exercised. The windowless proof is live: a self-re-arming
//! main-queue probe samples the pid's `CGWindowList` and `NSApp.windows`
//! — while the capture window is alive — and the workspace's frontmost
//! application is compared before and after the run.
//!
//! `run` is once per process — `startup` installs the panic hook, the
//! tracing subscriber and the executors once — so every run trial owns a
//! process: the one-case-per-process runner provides that.

// The suite only exists on `AppKit` — the preview entry does too. A
// target must still own a `main` on every other OS, so the whole suite
// sits behind the gate and the entry point stays unconditional.
#[cfg(target_os = "macos")]
mod suite {

    use std::cell::{Cell, RefCell};
    use std::ffi::c_void;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use cocoa_ui::objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSWorkspace};
    use cocoa_ui::objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFNumberType};
    use cocoa_ui::objc2_core_graphics::{
        CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID, kCGWindowIsOnscreen,
        kCGWindowNumber, kCGWindowOwnerPID,
    };
    use cocoa_ui::{MainThreadMarker, main_queue};
    use libtest_mimic::{Arguments, Trial};
    use waterui::prelude::*;
    use waterui::{AnyView, ResourceContext};
    use waterui_apple::preview::PreviewError;
    use waterui_preview_protocol::run::{
        PREVIEW_RUN_CONFIG_ENV, PreviewRunConfig, PreviewRunMode, RunConfigError,
    };

    /// The mimic entry — `main` below dispatches here on `AppKit`.
    pub fn run() {
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

    /// The registered trials — one PNG per image run, written under
    /// `target/native-test-preview/` for visual review.
    fn trials() -> Vec<Trial> {
        vec![
            Trial::test("preview::captures_the_issue_view_set_windowless", || {
                let assets = output_dir("view_set").join("assets");
                capture_run("view_set", 390.0, 760.0, move || {
                    issue_view_set(&assets)
                });
                assert_asset_rendered(&output_dir("view_set").join("view_set.png"));
                Ok(())
            }),
            #[cfg(feature = "gpu_surface")]
            Trial::test("preview::captures_gpu_surfaces_windowless", || {
                capture_run("gpu_set", 390.0, 760.0, gpu_set);
                Ok(())
            }),
            Trial::test("preview::rejects_a_scenario_run", || {
                let dir = output_dir("scenario");
                let result = run_with_config(
                    &dir,
                    &PreviewRunConfig {
                        width: 848.0,
                        height: 120.0,
                        mode: PreviewRunMode::Scenario {
                            output_dir: dir.clone(),
                            captures_ms: vec![0],
                            events: vec![],
                        },
                    },
                    {
                        let assets = dir.join("assets");
                        move || issue_view_set(&assets)
                    },
                );
                assert!(
                    matches!(result, Err(PreviewError::UnsupportedMode("scenario"))),
                    "a scenario run answers UnsupportedMode, got {result:?}"
                );
                Ok(())
            }),
            Trial::test("preview::rejects_a_semantic_run", || {
                let dir = output_dir("semantic");
                let result = run_with_config(
                    &dir,
                    &PreviewRunConfig {
                        width: 848.0,
                        height: 120.0,
                        mode: PreviewRunMode::Semantic,
                    },
                    {
                        let assets = dir.join("assets");
                        move || issue_view_set(&assets)
                    },
                );
                assert!(
                    matches!(result, Err(PreviewError::UnsupportedMode("semantic"))),
                    "a semantic run answers UnsupportedMode, got {result:?}"
                );
                Ok(())
            }),
            Trial::test("preview::errors_when_the_run_config_is_missing", || {
                // SAFETY: the trial owns its process on `main` — nothing else
                // reads the environment while the variable changes.
                unsafe { std::env::remove_var(PREVIEW_RUN_CONFIG_ENV) };
                let error = PreviewRunConfig::load_from_env()
                    .expect_err("a missing variable answers a typed error");
                assert!(
                    matches!(error, RunConfigError::MissingEnvVar),
                    "the missing variable reports MissingEnvVar, got {error}"
                );
                Ok(())
            }),
            Trial::test("preview::errors_when_the_run_config_is_malformed", || {
                let dir = output_dir("malformed");
                fs::create_dir_all(&dir).expect("the preview output directory is creatable");
                let config_path = dir.join("run-config.json");
                fs::write(&config_path, "{ not json").expect("the malformed file writes");
                // SAFETY: the trial owns its process on `main`.
                unsafe { std::env::set_var(PREVIEW_RUN_CONFIG_ENV, &config_path) };
                let error = PreviewRunConfig::load_from_env()
                    .expect_err("malformed JSON answers a typed error");
                assert!(
                    matches!(error, RunConfigError::Json { .. }),
                    "the malformed file reports Json, got {error}"
                );
                Ok(())
            }),
        ]
    }

    /// One image run, end to end: writes the run-configuration file `run`
    /// would be handed, lets `run` reload it through `PREVIEW_RUN_CONFIG_ENV`,
    /// arms the live windowless probe around the whole run, then asserts the
    /// window, focus and output contracts.
    fn capture_run(
        slug: &str,
        width: f32,
        height: f32,
        view: impl FnOnce() -> AnyView + 'static,
    ) {
        let mtm = mtm();
        let frontmost = frontmost_application();
        assert_ne!(
            frontmost,
            Some(i64::from(std::process::id().cast_signed())),
            "the suite must not already be the frontmost application"
        );

        let dir = output_dir(slug);
        let output = dir.join(format!("{slug}.png"));
        let probe = WindowProbe::arm(mtm);
        let result = run_with_config(
            &dir,
            &PreviewRunConfig {
                width,
                height,
                mode: PreviewRunMode::Image {
                    output: output.clone(),
                },
            },
            view,
        );
        probe.stop();
        assert!(result.is_ok(), "the preview run succeeds, got {result:?}");
        probe.assert_windowless();
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
        assert_eq!(
            frontmost_application(),
            frontmost,
            "the preview run must never change the frontmost application"
        );
        let bytes = fs::metadata(&output)
            .unwrap_or_else(|e| panic!("the capture PNG exists at {}: {e}", output.display()))
            .len();
        assert!(bytes > 0, "the capture PNG is not empty");
    }

    /// Drives `preview::run` once against `config`, written to a real
    /// run-configuration file and reloaded through `PREVIEW_RUN_CONFIG_ENV` —
    /// the path a generated preview binary takes. Also materializes the image
    /// asset's directory: the suite's `ImageAsset` resolves `image.png`
    /// against the `ResourceContext` `resources` gives the run.
    fn run_with_config(
        dir: &Path,
        config: &PreviewRunConfig,
        view: impl FnOnce() -> AnyView + 'static,
    ) -> Result<(), PreviewError> {
        fs::create_dir_all(dir).expect("the preview output directory is creatable");
        let config_path = dir.join("run-config.json");
        fs::write(
            &config_path,
            serde_json::to_string_pretty(config).expect("a run configuration serializes"),
        )
        .expect("the run configuration writes");
        // SAFETY: the trial owns its process on `main` — `run` has not
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

    /// The directory suite artifacts land in — under `target`, so the run's
    /// own build output is all it leaves behind.
    fn output_dir(slug: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/native-test-preview")
            .join(slug)
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

    /// Asserts the produced PNG contains the generated asset's orange —
    /// the decoded `Image`'s surface presented real pixels. `ImageAsset`'s
    /// own `Photo` load is asynchronous and may still be in flight at
    /// capture time, which is why the row carries both.
    fn assert_asset_rendered(output: &Path) {
        use cocoa_ui::objc2_app_kit::{NSBitmapImageRep, NSColorSpace};
        use cocoa_ui::objc2_foundation::{NSData, NSString};

        let path = NSString::from_str(&output.to_string_lossy());
        let data = NSData::dataWithContentsOfFile(&path).expect("the capture PNG reads back");
        let rep = NSBitmapImageRep::imageRepWithData(&data).expect("the capture PNG decodes");
        let srgb = NSColorSpace::sRGBColorSpace();
        let mut orange = 0_usize;
        for y in 0..rep.pixelsHigh() {
            for x in 0..rep.pixelsWide() {
                let Some(color) = rep.colorAtX_y(x, y) else {
                    continue;
                };
                let Some(color) = color.colorUsingColorSpace(&srgb) else {
                    continue;
                };
                let (mut red, mut green, mut blue, mut alpha) = (0.0, 0.0, 0.0, 0.0);
                // SAFETY: every out-pointer names a live `CGFloat`.
                unsafe {
                    color.getRed_green_blue_alpha(
                        &raw mut red,
                        &raw mut green,
                        &raw mut blue,
                        &raw mut alpha,
                    );
                }
                if red > 0.8 && (0.35..0.65).contains(&green) && blue < 0.3 && alpha > 0.9 {
                    orange += 1;
                }
            }
        }
        assert!(
            orange > 500,
            "the generated image asset renders its orange pixels, found {orange}"
        );
    }

    /// `pid` of the workspace's frontmost application — `NSWorkspace`'s
    /// answer for "who holds focus".
    fn frontmost_application() -> Option<i64> {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|app| i64::from(app.processIdentifier()))
    }

    /// Live evidence the run stays windowless: a self-re-arming main-queue
    /// probe samples the pid's window state on every drain — the capture
    /// window is sampled while it is alive — until the run returns and
    /// [`WindowProbe::stop`] cuts the chain.
    struct WindowProbe {
        /// Every sample the probe recorded.
        samples: Rc<RefCell<Vec<WindowSample>>>,
        /// Set to make the next queued sample not re-arm.
        stop: Rc<Cell<bool>>,
    }

    /// One windowless sample: this pid's onscreen windows and the windows
    /// `NSApp` tracks at one main-queue drain.
    struct WindowSample {
        /// `kCGWindowNumber` of every pid window that is on screen.
        onscreen: Vec<i64>,
        /// How many `NSWindow`s `NSApp` tracked at the sample — the capture
        /// window included while it lives.
        windows: usize,
        /// Of `windows`, the count answering `isVisible`.
        visible: usize,
    }

    impl WindowProbe {
        /// Enqueues the first sample on the main queue; each re-arms the next
        /// until [`Self::stop`].
        fn arm(mtm: MainThreadMarker) -> Self {
            let probe = Self {
                samples: Rc::new(RefCell::new(Vec::new())),
                stop: Rc::new(Cell::new(false)),
            };
            Self::sample(mtm, Rc::clone(&probe.samples), Rc::clone(&probe.stop));
            probe
        }

        /// Enqueues one sample on the main queue and re-arms for the next
        /// drain while the probe lives.
        fn sample(
            mtm: MainThreadMarker,
            samples: Rc<RefCell<Vec<WindowSample>>>,
            stop: Rc<Cell<bool>>,
        ) {
            main_queue::enqueue_local(mtm, move |mtm| {
                if stop.get() {
                    return;
                }
                samples.borrow_mut().push(WindowSample::now(mtm));
                Self::sample(mtm, Rc::clone(&samples), Rc::clone(&stop));
            });
        }

        /// Ends the probe: anything still queued sees the flag and does not
        /// re-arm.
        fn stop(&self) {
            self.stop.set(true);
        }

        /// Every sample must show the pid owning no onscreen window and no
        /// visible `NSWindow`, and the probe must have observed the capture
        /// window — a run whose samples are empty or windowless proves
        /// nothing.
        fn assert_windowless(&self) {
            let samples = self.samples.borrow();
            let onscreen: Vec<i64> = samples
                .iter()
                .flat_map(|sample| sample.onscreen.iter().copied())
                .collect();
            assert!(
                onscreen.is_empty(),
                "the capture must never order a window on screen; windows {onscreen:?} of this process are on screen"
            );
            let visible: Vec<usize> = samples
                .iter()
                .filter(|sample| sample.visible > 0)
                .map(|sample| sample.visible)
                .collect();
            assert!(
                visible.is_empty(),
                "no window the process owns may be visible during the run; counts {visible:?}"
            );
            assert!(
                samples.iter().any(|sample| sample.windows > 0),
                "the live probe must observe the capture window while it is alive"
            );
        }
    }

    impl WindowSample {
        /// Reads this pid's `CGWindowList` onscreen windows and every
        /// `NSWindow` `NSApp` tracks — windows that never ordered in are
        /// tracked there too.
        fn now(mtm: MainThreadMarker) -> Self {
            let windows = NSApplication::sharedApplication(mtm).windows();
            let visible = windows.iter().filter(|window| window.isVisible()).count();
            Self {
                onscreen: onscreen_window_numbers(),
                windows: windows.len(),
                visible,
            }
        }
    }

    /// `kCGWindowNumber` of every onscreen window this pid owns — empty when
    /// the process holds only windows it never ordered.
    fn onscreen_window_numbers() -> Vec<i64> {
        let windows = CGWindowListCopyWindowInfo(
            CGWindowListOption(
                CGWindowListOption::OptionAll.0 | CGWindowListOption::ExcludeDesktopElements.0,
            ),
            kCGNullWindowID,
        )
        .expect("the window server answers the window list");
        let pid = i64::from(std::process::id().cast_signed());
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
    fn issue_view_set(assets_dir: &Path) -> AnyView {
        use waterui::form::picker::picker;
        use waterui::gradient::Gradient;
        use waterui::shape::{Circle, Rectangle, RoundedRectangle, ShapeExt};

        // `image.png` is generated into `assets_dir` by `run_with_config`
        // before `run` mounts this view — the same directory the run's
        // explicit `ResourceContext` mounts as `Bundle::main()`'s root.
        // `ImageAsset` resolves there but loads through `Photo`'s
        // asynchronous decode, which this first-presented-frames capture
        // does not wait on, so the row also mounts the decoded `Image`:
        // its `SceneView` presents a real first frame that
        // `wait_for_presented` covers deterministically.
        let image = waterui::media::Image::from_encoded(
            &fs::read(assets_dir.join("image.png")).expect("the generated asset reads"),
        )
        .expect("the generated asset decodes");
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
        hstack((text("Count: "), stepper("Count", &count), spacer())),
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
            image.resizable().size(48.0, 48.0),
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
