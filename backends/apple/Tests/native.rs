//! Native tests for the Rust `AppKit`/`UIKit` backend — real platform
//! objects on their actual main thread. Metal presentation trials use a real
//! `AppKit` application event loop and window on the test machine.
//!
//! These cases create `NSView`/`NSWindow`/`UIView` objects, which
//! `MainThreadMarker`-protected APIs only allow on the process's actual
//! main thread. The stock harness runs cases on worker threads, so this
//! target is `harness = false`: [`libtest_mimic`] gives it the libtest CLI
//! that nextest enumerates (`--list`, `--exact`, one process per case), and
//! `test_threads = 1` makes it run every case on `main`, where
//! [`MainThreadMarker::new`] answers `Some`.
//!
//! Everything here goes through the backend's public typed surfaces —
//! `dispatch::install`/`dispatch::render`, `windows::bind_root_window`,
//! `contract::NativeLeaf` — plus the `cocoa-ui` kit API, so the suite
//! exercises exactly what a host embedding the backend could.

// The suite only exists on the crate's supported targets.
#![cfg(any(target_os = "macos", target_os = "ios"))]

use cocoa_ui::{MainThreadMarker, PlatformView, Retained};
use libtest_mimic::{Arguments, Trial};

mod migration;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::{HostView, Label};
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::{HostView, Label};

fn main() {
    let mut args = Arguments::from_args();
    // `AppKit`/`UIKit` objects may only be built on the real main thread;
    // `run` executes sequentially in the calling thread at one thread.
    args.test_threads = Some(1);
    // Process-global startup (executors, tracing dispatcher) installs
    // once here on the same true main thread the trials run on — the
    // gpu-surface fixture mounts through it.
    #[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
    waterui_apple::native_test_support::gpu_surface::initialize_process();
    #[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
    if !args.list {
        use cocoa_ui::appkit::{ActivationPolicy, Application, ApplicationHandlers};
        let app = Application::shared(mtm());
        let _policy_accepted = app.set_activation_policy(ActivationPolicy::Regular);
        app.run(ApplicationHandlers::new().did_finish_launching(move |mtm| {
            // Run within the AppKit application lifecycle. Trials dispatch
            // native application events; a GCD block around the whole suite
            // would prevent reentrant main-queue completion delivery.
            let _ = mtm;
            libtest_mimic::run(&args, trials()).exit();
        }));
        unreachable!("the native trial runner exits the process");
    }
    libtest_mimic::run(&args, trials()).exit();
}

fn trials() -> Vec<Trial> {
    let mut tests = base_trials();
    #[cfg(all(target_os = "macos", feature = "native-test"))]
    {
        tests.extend([
            Trial::test("window::manager_installs_into_the_environment", || {
                window::manager_installs_into_the_environment(mtm());
                Ok(())
            }),
            Trial::test("window::bind_root_window_wires_a_live_window", || {
                window::bind_root_window_wires_a_live_window(mtm());
                Ok(())
            }),
        ]);
        #[cfg(feature = "gpu_surface")]
        {
            tests.extend(gpu_surface::trials());
            tests.extend(filtered::trials());
        }
    }
    #[cfg(target_os = "ios")]
    {
        tests.extend(tabs::trials());
        tests.extend(controller_bounds::trials());
    }
    tests.extend(migration::trials());
    tests.extend(owner_lifetimes::trials());
    tests
}

fn base_trials() -> Vec<Trial> {
    vec![
        Trial::test("leaf::mount_attaches_and_unmount_detaches", || {
            leaf::mount_attaches_and_unmount_detaches();
            Ok(())
        }),
        Trial::test("leaf::dropping_mounted_detaches_the_view", || {
            leaf::dropping_mounted_detaches_the_view();
            Ok(())
        }),
        Trial::test("leaf::bind_applies_now_and_on_every_change", || {
            leaf::bind_applies_now_and_on_every_change();
            Ok(())
        }),
        Trial::test("leaf::mounting_installs_the_intrinsic_measure", || {
            leaf::mounting_installs_the_intrinsic_measure();
            Ok(())
        }),
        Trial::test("leaf::a_layout_pass_applies_the_handler_frame", || {
            leaf::a_layout_pass_applies_the_handler_frame();
            Ok(())
        }),
        Trial::test("resolve::unit_view_maps_to_a_hidden_empty_host", || {
            resolve::unit_view_maps_to_a_hidden_empty_host();
            Ok(())
        }),
        Trial::test("resolve::a_string_maps_to_the_text_leaf", || {
            resolve::a_string_maps_to_the_text_leaf();
            Ok(())
        }),
        Trial::test("resolve::spacer_maps_to_a_stretching_host", || {
            resolve::spacer_maps_to_a_stretching_host();
            Ok(())
        }),
        Trial::test("resolve::opacity_metadata_wraps_the_child", || {
            resolve::opacity_metadata_wraps_the_child();
            Ok(())
        }),
        Trial::test("resolve::an_unclaimed_metadata_view_panics", || {
            resolve::an_unclaimed_metadata_view_panics();
            Ok(())
        }),
        Trial::test("resolve::ignorable_metadata_renders_its_content", || {
            resolve::ignorable_metadata_renders_its_content();
            Ok(())
        }),
        Trial::test("resolve::a_native_with_fallback_resolves_to_it", || {
            resolve::a_native_with_fallback_resolves_to_it();
            Ok(())
        }),
        Trial::test(
            "resolve::unclaimed_wrappers_panic_while_claimed_render",
            || {
                resolve::unclaimed_wrappers_panic_while_claimed_render();
                Ok(())
            },
        ),
        Trial::test(
            "picture::a_laid_out_picture_rasterizes_at_its_bounds",
            || {
                picture::a_laid_out_picture_rasterizes_at_its_bounds();
                Ok(())
            },
        ),
    ]
}

/// The marker the whole suite builds objects under — the real one, on the
/// thread `main` runs on.
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("the custom harness runs cases on the process's main thread")
}

/// `NativeLeaf` mount/watch/bind against real views.
mod leaf {
    use std::cell::Cell;
    use std::rc::Rc;

    use waterui::reactive::binding;
    use waterui_apple::contract::NativeLeaf;
    use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

    use super::{HostView, Label, MainThreadMarker, PlatformView, mtm};

    /// A fixed-size leaf: the smallest `SubView` the mount path needs.
    pub struct TestSubView;

    impl SubView for TestSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(Size::new(40.0, 20.0))
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    /// Mounting adds the leaf's view to the parent's subview list, and the
    /// returned `Mounted`'s `unmount` detaches it again and hands the leaf
    /// back for reuse: mounted again, the handlers its component installed
    /// before the first mount still run.
    pub fn mount_attaches_and_unmount_detaches() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let _window = attach(mtm, &parent);
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let laid_out = Rc::new(Cell::new(false));
        child.set_layout_handler({
            let laid_out = Rc::clone(&laid_out);
            move |_| laid_out.set(true)
        });
        let leaf = NativeLeaf::new(&*child, TestSubView);
        let mounted = leaf.mount(&parent);
        assert_eq!(cocoa_ui::view::subviews(&parent).len(), 1);
        let leaf = mounted.unmount();
        assert!(cocoa_ui::view::superview(leaf.view()).is_none());
        assert_eq!(cocoa_ui::view::subviews(&parent).len(), 0);

        let _mounted = leaf.mount(&parent);
        laid_out.set(false);
        child.set_needs_layout();
        child.layout_if_needed();
        assert!(laid_out.get(), "a moved leaf keeps its layout handler");
    }

    /// Dropping a `Mounted` — how a container releases a replaced child —
    /// detaches the view from its superview before releasing the leaf.
    pub fn dropping_mounted_detaches_the_view() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let child = Label::new(mtm);
        let child_view: &PlatformView = &child;
        let mounted = NativeLeaf::new(child_view, TestSubView).mount(&parent);
        let view = cocoa_ui::view::retain_base(mounted.view());
        drop(mounted);
        assert!(cocoa_ui::view::superview(&view).is_none());
    }

    /// `bind` applies the current value immediately and every later write —
    /// the path every reactive property takes into its platform object.
    pub fn bind_applies_now_and_on_every_change() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mut leaf = NativeLeaf::new(&*host, TestSubView);
        let target = cocoa_ui::view::retain_base(leaf.view());
        let flag = binding(false);
        leaf.bind(&flag, move |value| {
            cocoa_ui::view::set_hidden(&target, value);
        });
        assert!(!cocoa_ui::view::is_hidden(&host));
        flag.set(true);
        assert!(cocoa_ui::view::is_hidden(&host));
    }

    /// A `HostView` leaf mirrors its layout face onto the view's intrinsic
    /// measure only once mounted — before it, the view answers exactly what
    /// an unattached kit host answers.
    pub fn mounting_installs_the_intrinsic_measure() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let unattached = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let leaf = NativeLeaf::new(&*child, TestSubView);
        assert_eq!(
            cocoa_ui::view::fitting_size(leaf.view()),
            cocoa_ui::view::fitting_size(&unattached)
        );
        let _mounted = leaf.mount(&parent);
        let fitting = cocoa_ui::view::fitting_size(&child);
        assert_eq!(fitting, cocoa_ui::geometry::Size::new(40.0, 20.0));
    }

    /// A host inside a real (never shown) window runs its layout pass, and
    /// the handler's frames land on the children — the bridge every
    /// container leans on.
    pub fn a_layout_pass_applies_the_handler_frame() {
        let mtm = mtm();
        let host = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mounted = NativeLeaf::new(&*child, TestSubView).mount(&host);
        let child_view = cocoa_ui::view::retain_base(mounted.view());
        host.set_layout_handler(move |host| {
            let host_view: &PlatformView = host;
            cocoa_ui::view::set_frame(&child_view, cocoa_ui::view::bounds(host_view));
        });
        let _window = attach(mtm, &host);
        host.set_needs_layout();
        host.layout_if_needed();
        // `set_content_view` resizes the host to the window's content
        // area, so the expected child frame is the host's real bounds —
        // the handler must land that frame verbatim.
        let expected = cocoa_ui::view::bounds(&host);
        let frame = cocoa_ui::view::frame(mounted.view());
        assert_eq!(frame.size, expected.size);
    }

    /// Puts `content` inside a real window that is never ordered in — the
    /// smallest environment in which the frameworks still run their full
    /// layout path.
    #[cfg(target_os = "macos")]
    pub fn attach(mtm: MainThreadMarker, content: &PlatformView) -> cocoa_ui::appkit::Window {
        let window = cocoa_ui::appkit::Window::new(
            mtm,
            cocoa_ui::Rect::new(0.0, 0.0, 640.0, 480.0),
            cocoa_ui::appkit::WindowStyle::TITLED | cocoa_ui::appkit::WindowStyle::CLOSABLE,
        );
        window.set_content_view(content);
        window
    }

    /// `UIKit` does not need a scene for `layoutSubviews` to run; the
    /// window exists so `window`-dependent paths see a real one.
    #[cfg(target_os = "ios")]
    pub fn attach(
        mtm: MainThreadMarker,
        content: &PlatformView,
    ) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIWindow> {
        use cocoa_ui::objc2_ui_kit::UIWindow;
        use objc2::{MainThreadOnly, msg_send};

        // SAFETY: `initWithFrame:` is `UIWindow`'s plain initializer and
        // `mtm` proves the main-thread confinement the harness provides.
        let window: cocoa_ui::Retained<UIWindow> = unsafe {
            msg_send![
                UIWindow::alloc(mtm),
                initWithFrame: objc2_core_foundation::CGRect::new(
                    objc2_core_foundation::CGPoint::new(0.0, 0.0),
                    objc2_core_foundation::CGSize::new(390.0, 844.0),
                )
            ]
        };
        window.addSubview(content);
        // Size the content to the window's bounds the way
        // `set_content_view` does on macOS — `addSubview` alone leaves it
        // at its zero frame, so surfaces that read their viewport (lazy
        // containers especially) would see an empty window.
        cocoa_ui::view::set_frame(content, cocoa_ui::view::bounds(&window));
        window
    }
}

/// View → leaf mapping through `dispatch::render` — the typed entry point
/// a host reaches. A view nobody claims panics (there is no foreign caller
/// to hand it back to), so a spurious empty render could not masquerade as
/// a pass.
mod resolve {
    use waterui::filter::Opacity;
    use waterui::layout::Spacer;
    use waterui::reactive::{SignalExt, binding};
    use waterui_apple::contract::NativeLeaf;
    use waterui_backend_core::{AnyView, Environment, View};
    use waterui_core::layout::{ProposalSize, Size, StretchAxis};
    use waterui_core::metadata::MetadataKey;
    use waterui_core::{IgnorableMetadata, Metadata, Native, NativeView};

    use super::{HostView, Label, Retained, mtm};

    /// A `Metadata` key no handler is registered for — an honest miss,
    /// never fabricated.
    struct Unregistered;

    impl MetadataKey for Unregistered {}

    /// A `NativeView` no handler is registered for.
    struct UnclaimedNative;

    impl NativeView for UnclaimedNative {}

    /// The minimum environment a real render needs: `dispatch::install`
    /// performs the backend's half of the embedding contract (dispatcher,
    /// window manager, realizations); the theme slots text resolves
    /// through are the framework's.
    pub fn env() -> Environment {
        use waterui::graphics::color::WorkingColor;
        use waterui::text::font::{Body, Caption, FontSlot, Subheadline};

        let mut env = Environment::new();
        waterui_apple::dispatch::install(&mut env);
        waterui::theme::install_color_scheme(
            &mut env,
            binding(waterui::theme::ColorScheme::Light).computed(),
        );
        let black = || binding(WorkingColor::BLACK).computed();
        waterui::theme::install_color_signal::<waterui::theme::color::Foreground>(
            &mut env,
            black(),
        );
        // The richer fixtures (list rows, stacked text) resolve muted and
        // accent roles plus the caption/subheadline slots — install them so
        // a theme miss can't masquerade as a render failure.
        waterui::theme::install_color_signal::<waterui::theme::color::MutedForeground>(
            &mut env,
            black(),
        );
        waterui::theme::install_color_signal::<waterui::theme::color::Accent>(&mut env, black());
        waterui::theme::install_font_signal::<Body>(&mut env, binding(Body::DEFAULT).computed());
        waterui::theme::install_font_signal::<Caption>(
            &mut env,
            binding(Caption::DEFAULT).computed(),
        );
        waterui::theme::install_font_signal::<Subheadline>(
            &mut env,
            binding(Subheadline::DEFAULT).computed(),
        );
        env
    }

    /// Renders `view` through the typed dispatch entry point, main thread,
    /// fresh env. Panics when nothing claims the view — the typed
    /// contract's answer to a miss.
    pub fn render(view: impl View) -> NativeLeaf {
        let _mtm = mtm();
        waterui_apple::dispatch::render(AnyView::new(view), &env())
    }

    /// Renders `view`, reporting whether `render` panicked instead of
    /// producing a leaf — for the cases asserting the miss path itself.
    fn render_or_panic(view: impl View) -> Option<NativeLeaf> {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(view))).ok()
    }

    /// `()` lands as a hidden `HostView` that measures zero and answers
    /// `is_empty` — the leaf a stack ignores.
    pub fn unit_view_maps_to_a_hidden_empty_host() {
        let leaf = render(());
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(cocoa_ui::view::is_hidden(&view));
        assert!(leaf.layout().is_empty());
        assert_eq!(
            leaf.layout()
                .measure(ProposalSize::new(Some(100.0), Some(100.0)))
                .size,
            Size::new(0.0, 0.0)
        );
    }

    /// A `&'static str` expands `Str` → `Native<Str>` → the text leaf: a
    /// `HostView` wrapper holding a real kit `Label` with the string's
    /// attributed text on it — and the layout face still answers measure.
    pub fn a_string_maps_to_the_text_leaf() {
        let leaf = render("hello waterui");
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(!cocoa_ui::view::is_hidden(&view));
        let subviews = cocoa_ui::view::subviews(&view);
        assert_eq!(subviews.len(), 1);
        let label = subviews[0]
            .downcast_ref::<Label>()
            .expect("the text leaf mounts a kit label");
        let text = label
            .source_text()
            .expect("the label carries attributed text");
        assert_eq!(text.string().to_string(), "hello waterui");
        let measured = leaf.layout().measure(ProposalSize::UNSPECIFIED);
        assert!(measured.size.width > 0.0);
        assert!(measured.size.height > 0.0);
    }

    /// `Native<Spacer>` maps to a transparent host whose layout face
    /// stretches on the enclosing stack's main axis.
    pub fn spacer_maps_to_a_stretching_host() {
        let leaf = render(Spacer::new(12.0));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(!cocoa_ui::view::is_hidden(&view));
        assert_eq!(leaf.layout().stretch_axis(), StretchAxis::MainAxis);
        assert_eq!(leaf.layout().priority(), i32::MIN);
    }

    /// `Metadata<Opacity>` is claimed by its handler: the wrapper is a
    /// `HostView` at the declared alpha with the content mounted inside.
    pub fn opacity_metadata_wraps_the_child() {
        let leaf = render(Metadata::new((), Opacity::new(0.5)));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert_eq!(cocoa_ui::view::alpha(&view).to_bits(), 0.5f64.to_bits());
        let subviews = cocoa_ui::view::subviews(&view);
        assert_eq!(subviews.len(), 1);
        let primary = cocoa_ui::view::primary_content(&view)
            .expect("the wrapper forwards its primary content");
        assert_eq!(Retained::as_ptr(&primary), Retained::as_ptr(&subviews[0]));
    }

    /// A `Metadata` nobody claims panics in `body()` — on the typed
    /// contract nothing catches it, so the panic propagates out of
    /// `render` itself.
    pub fn an_unclaimed_metadata_view_panics() {
        assert!(render_or_panic(Metadata::new((), Unregistered)).is_none());
    }

    /// `IgnorableMetadata` is transparent: its `body()` returns the
    /// content, so an unregistered key renders through to the content's
    /// leaf.
    pub fn ignorable_metadata_renders_its_content() {
        let leaf = render(IgnorableMetadata::new((), Unregistered));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert!(cocoa_ui::view::is_hidden(&view));
    }

    /// `Native::with_fallback` is the honest port of "not claimed here":
    /// the dispatcher expands to the embedded fallback and resolves it —
    /// here, to the spacer host — inside the one `render` call.
    pub fn a_native_with_fallback_resolves_to_it() {
        let leaf = render(Native::new(UnclaimedNative).with_fallback(Spacer::new(8.0)));
        let view = cocoa_ui::view::retain_base(leaf.view());
        assert!(view.downcast_ref::<HostView>().is_some());
        assert_eq!(leaf.layout().stretch_axis(), StretchAxis::MainAxis);
    }

    /// The typed unclaimed-view contract: a view whose `body()` panics —
    /// `Metadata`/`Native` wrappers nobody claims — propagates the panic
    /// out of `render`; composable views resolve.
    pub fn unclaimed_wrappers_panic_while_claimed_render() {
        assert!(render_or_panic(Metadata::new((), Unregistered)).is_none());
        assert!(render_or_panic(Native::new(UnclaimedNative)).is_none());
        render(());
        render(Spacer::new(8.0));
        render(IgnorableMetadata::new((), Unregistered));
    }
}

/// `Native<Picture>` rasterization: the bitmap is sized to the laid-out
/// bounds times the backing scale, and a relayout re-rasterizes — a picture
/// stretched past its declared size shows pixels rasterized for that size,
/// not an upscale of the declared-size bitmap.
mod picture {
    use std::cell::Cell;
    use std::rc::Rc;

    #[cfg(target_os = "macos")]
    use cocoa_ui::appkit::ImageView;
    #[cfg(target_os = "ios")]
    use cocoa_ui::uikit::ImageView;
    use kurbo::Shape;
    use waterui::graphics::color::WorkingColor;
    use waterui::graphics::draw::Draw;
    use waterui::graphics::picture::Picture;
    use waterui::reactive::constant;
    use waterui_core::layout::Size;

    use super::{HostView, PlatformView, mtm};

    /// The pixel dimensions of the bitmap the leaf's image view shows.
    #[cfg(target_os = "macos")]
    fn image_pixels(view: &PlatformView) -> (usize, usize) {
        let view = view
            .downcast_ref::<ImageView>()
            .expect("the picture leaf mounts the kit image view");
        let image = view.image().expect("the picture leaf paints an image");
        let rep = image
            .representations()
            .firstObject()
            .expect("the raster image carries one rep");
        (
            usize::try_from(rep.pixelsWide()).expect("a bitmap has nonnegative pixels"),
            usize::try_from(rep.pixelsHigh()).expect("a bitmap has nonnegative pixels"),
        )
    }

    /// The pixel dimensions of the bitmap the leaf's image view shows.
    #[cfg(target_os = "ios")]
    fn image_pixels(view: &PlatformView) -> (usize, usize) {
        let view = view
            .downcast_ref::<ImageView>()
            .expect("the picture leaf mounts the kit image view");
        let image = view.image().expect("the picture leaf paints an image");
        // SAFETY: `CGImage` is a plain property read on the main thread.
        let cg = unsafe { image.CGImage() }.expect("the raster image is a CGImage");
        (
            cocoa_ui::objc2_core_graphics::CGImage::width(Some(&cg)),
            cocoa_ui::objc2_core_graphics::CGImage::height(Some(&cg)),
        )
    }

    /// A `Picture` laid out at twice its declared size rasterizes at twice
    /// the pixel size: the leaf re-rasterizes when its bounds change rather
    /// than upscaling the bitmap painted for the declared size (#1565).
    pub fn a_laid_out_picture_rasterizes_at_its_bounds() {
        let mtm = mtm();
        let declared = Size::new(100.0, 50.0);
        let picture = Picture::new(
            declared,
            constant(Picture::record(|scene| {
                scene.fill(
                    kurbo::Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1),
                    WorkingColor::BLACK,
                );
            })),
        );
        let leaf = crate::resolve::render(picture);

        let host = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 400.0, 300.0));
        let mounted = leaf.mount(&host);
        let child = cocoa_ui::view::retain_base(mounted.view());
        let child_size = Rc::new(Cell::new(declared));
        host.set_layout_handler({
            let child_size = Rc::clone(&child_size);
            move |_host| {
                let size = child_size.get();
                cocoa_ui::view::set_frame(
                    &child,
                    cocoa_ui::Rect::new(0.0, 0.0, f64::from(size.width), f64::from(size.height)),
                );
            }
        });

        let _window = crate::leaf::attach(mtm, &host);
        // Two flushes: the parent's pass frames the child, the second runs
        // the child's own layout hook on its new bounds.
        host.set_needs_layout();
        host.layout_if_needed();
        host.layout_if_needed();
        let pixels_at_declared = image_pixels(mounted.view());
        assert!(
            pixels_at_declared.0 > 0 && pixels_at_declared.1 > 0,
            "the declared-size bitmap exists"
        );

        child_size.set(Size::new(declared.width * 2.0, declared.height * 2.0));
        host.set_needs_layout();
        host.layout_if_needed();
        host.layout_if_needed();

        assert_eq!(
            image_pixels(mounted.view()),
            (pixels_at_declared.0 * 2, pixels_at_declared.1 * 2),
            "laid out at twice its declared size, the rasterized bitmap must double"
        );
    }
}

/// `TabsLayout` chrome the iOS backend owns — the `UITabAccessory` bottom
/// slot and `tabBarMinimizeBehavior` — against a real `UITabBarController`.
/// All cases run on the true main thread under the harness and reach the
/// controller the way a host does: through the view hierarchy's responder
/// chain.
#[cfg(target_os = "ios")]
mod tabs {
    use cocoa_ui::objc2_ui_kit::{
        NSDirectionalRectEdge, UIScrollView, UITabBarController, UITabBarMinimizeBehavior,
    };
    use cocoa_ui::uikit::view_controller::owning_controller;
    use cocoa_ui::uikit::{Label, NavContentController};
    use cocoa_ui::{PlatformView, Retained, view};
    use waterui::navigation::{NavigationStack, NavigationView, Tab, TabBarMinimizeBehavior, Tabs};
    use waterui::prelude::{label, scroll, text, vstack};
    use waterui::reactive::binding;
    use waterui_apple::contract::NativeLeaf;

    use super::resolve;

    /// The `tabs::` trials, kept beside their fixtures so the top-level
    /// registry stays a one-line extension per platform.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test("tabs::pane_scroll_view_associates_for_collapse", || {
                pane_scroll_view_associates_for_collapse();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::panes_without_surfaces_answer_none", || {
                panes_without_surfaces_answer_none();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::pane_scroll_view_tracks_nav_pushes", || {
                pane_scroll_view_tracks_nav_pushes();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::pane_scroll_view_tracks_dynamic_replacement", || {
                pane_scroll_view_tracks_dynamic_replacement();
                Ok(())
            }),
            libtest_mimic::Trial::test("tabs::bottom_accessory_mounts_into_the_controller", || {
                bottom_accessory_mounts_into_the_controller();
                Ok(())
            }),
            libtest_mimic::Trial::test(
                "tabs::accessory_layout_centers_the_measured_answer",
                || {
                    accessory_layout_centers_the_measured_answer();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::binding_updates_preserve_the_accessory_mount",
                || {
                    binding_updates_preserve_the_accessory_mount();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::tab_selection_does_not_rebuild_the_accessory",
                || {
                    tab_selection_does_not_rebuild_the_accessory();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::dropping_the_leaf_releases_accessory_watchers",
                || {
                    dropping_the_leaf_releases_accessory_watchers();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "tabs::each_minimize_behavior_maps_to_the_uikit_property",
                || {
                    each_minimize_behavior_maps_to_the_uikit_property();
                    Ok(())
                },
            ),
        ]
    }

    /// Tab identity for the fixtures.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Pane {
        One,
        Two,
    }

    /// The rendered leaf's first subview is the controller's root view; a
    /// `UIViewController` sits on the responder chain right after its own
    /// view — the same reach `adopt_controllers` relies on.
    fn tab_bar_controller(leaf: &NativeLeaf) -> Retained<UITabBarController> {
        let controller_view = view::subviews(leaf.view())
            .into_iter()
            .next()
            .expect("the tabs host carries the controller's view");
        owning_controller(&controller_view)
            .and_then(|responder| responder.downcast::<UITabBarController>().ok())
            .expect("the tabs leaf mounts a UITabBarController")
    }

    /// First `Label`'s text in the subtree, depth-first.
    fn label_text(view: &PlatformView) -> Option<String> {
        for sub in view::subviews(view) {
            if let Some(label) = sub.downcast_ref::<Label>()
                && let Some(attributed) = label.source_text()
            {
                return Some(attributed.string().to_string());
            }
            if let Some(found) = label_text(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// First `Label` in the subtree, retained for reads after the leaf
    /// that mounted it is gone.
    fn first_label(view: &PlatformView) -> Option<Retained<PlatformView>> {
        for sub in view::subviews(view) {
            if sub.downcast_ref::<Label>().is_some() {
                return Some(sub);
            }
            if let Some(found) = first_label(&sub) {
                return Some(found);
            }
        }
        None
    }

    /// Two plain tabs with `accessory` in the bottom slot.
    fn tabs(
        accessory: impl waterui_backend_core::View,
    ) -> (waterui::reactive::Binding<Pane>, NativeLeaf) {
        let pane = binding(Pane::One);
        let leaf = resolve::render(
            Tabs::new(
                &pane,
                vec![
                    Tab::container(Pane::One, label("One"), || text("pane one")),
                    Tab::container(Pane::Two, label("Two"), || text("pane two")),
                ],
            )
            .bottom_accessory(accessory),
        );
        (pane, leaf)
    }

    /// The shared `bottom_accessory` view lands in the controller's own
    /// accessory slot: `UIKit` reports a `UITabAccessory` whose content
    /// view carries the rendered text at a natural height.
    pub fn bottom_accessory_mounts_into_the_controller() {
        let (_pane, leaf) = tabs(text("Now Playing"));
        let accessory = tab_bar_controller(&leaf)
            .bottomAccessory()
            .expect("the shared accessory view installs a UITabAccessory");
        let content = accessory.contentView();
        assert_eq!(label_text(&content).as_deref(), Some("Now Playing"));
        assert!(view::fitting_size(&content).height > 0.0);
    }

    /// A `Binding` inside the accessory updates the mounted text in place
    /// — the same `UITabAccessory` keeps serving the capsule.
    pub fn binding_updates_preserve_the_accessory_mount() {
        let track = binding(String::from("first"));
        let (_pane, leaf) = tabs(text!("{track}"));
        let controller = tab_bar_controller(&leaf);
        let accessory = controller.bottomAccessory().expect("installed");
        let content = accessory.contentView();
        // `text!` wraps interpolations in bidi isolates — match the payload.
        assert!(label_text(&content).is_some_and(|text| text.contains("first")));
        track.set(String::from("second"));
        assert!(label_text(&content).is_some_and(|text| text.contains("second")));
        let after = controller.bottomAccessory().expect("still installed");
        assert_eq!(Retained::as_ptr(&after), Retained::as_ptr(&accessory));
    }

    /// Selecting the other tab switches panes through the existing
    /// controller — the accessory mount is untouched.
    pub fn tab_selection_does_not_rebuild_the_accessory() {
        let (pane, leaf) = tabs(text("Now Playing"));
        let controller = tab_bar_controller(&leaf);
        let accessory = controller.bottomAccessory().expect("installed");
        pane.set(Pane::Two);
        assert_eq!(controller.selectedIndex(), 1);
        let after = controller.bottomAccessory().expect("still installed");
        assert_eq!(Retained::as_ptr(&after), Retained::as_ptr(&accessory));
    }

    /// Dropping the leaf uninstalls the capsule and releases the mounted
    /// subtree's watchers — the controller itself is owned by containment,
    /// not by the leaf, so a retained handle stays valid but must report
    /// no accessory, and no later write may reach the old label.
    pub fn dropping_the_leaf_releases_accessory_watchers() {
        let track = binding(String::from("first"));
        let (label, controller) = {
            let (_pane, leaf) = tabs(text!("{track}"));
            let controller = tab_bar_controller(&leaf);
            let label = {
                let content = controller
                    .bottomAccessory()
                    .expect("installed")
                    .contentView();
                track.set(String::from("second"));
                first_label(&content).expect("the accessory mounts a label")
            };
            drop(leaf);
            (label, controller)
        };
        assert!(controller.bottomAccessory().is_none());
        track.set(String::from("third"));
        let text = label
            .downcast_ref::<Label>()
            .and_then(Label::source_text)
            .expect("the retained label");
        let text = text.string().to_string();
        assert!(text.contains("second") && !text.contains("third"));
    }

    /// Every shared minimize-behavior variant maps onto the `UIKit`
    /// property one-to-one — no translation layer of our own.
    pub fn each_minimize_behavior_maps_to_the_uikit_property() {
        for (shared, native) in [
            (
                TabBarMinimizeBehavior::Automatic,
                UITabBarMinimizeBehavior::Automatic,
            ),
            (
                TabBarMinimizeBehavior::Never,
                UITabBarMinimizeBehavior::Never,
            ),
            (
                TabBarMinimizeBehavior::OnScrollDown,
                UITabBarMinimizeBehavior::OnScrollDown,
            ),
            (
                TabBarMinimizeBehavior::OnScrollUp,
                UITabBarMinimizeBehavior::OnScrollUp,
            ),
        ] {
            let pane = binding(Pane::One);
            let leaf = resolve::render(
                Tabs::new(
                    &pane,
                    vec![Tab::container(Pane::One, label("One"), || text("one"))],
                )
                .minimize_behavior(shared),
            );
            assert_eq!(tab_bar_controller(&leaf).tabBarMinimizeBehavior(), native);
        }
    }

    /// Every `UIScrollView` in the subtree, depth-first — each fixture
    /// pane mounts exactly one, so the association can only be that
    /// surface.
    fn scroll_views_in(view: &PlatformView) -> Vec<Retained<UIScrollView>> {
        let mut found = Vec::new();
        for sub in view::subviews(view) {
            if let Some(scroll) = sub.downcast_ref::<UIScrollView>() {
                found.push(scroll.into());
            }
            found.extend(scroll_views_in(&sub));
        }
        found
    }

    /// `UIKit` only auto-associates a scroll view that is the
    /// controller's own root, so the leaf associates each pane's declared
    /// scroll surface for the bottom edge: a scroll mounted as the root,
    /// or one the pane's container declares as a candidate.
    pub fn pane_scroll_view_associates_for_collapse() {
        let pane = binding(Pane::One);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![
                Tab::container(Pane::One, label("One"), || {
                    scroll(vstack((text("one"), text("two"), text("three"))))
                }),
                Tab::container(Pane::Two, label("Two"), || {
                    vstack((text("aside"), scroll(text("nested"))))
                }),
            ],
        ));
        let controllers = tab_bar_controller(&leaf)
            .viewControllers()
            .expect("tabs installed");
        for (index, controller) in controllers.iter().enumerate() {
            let root = controller.view().expect("the pane's root view");
            let scrolls = scroll_views_in(&root);
            assert_eq!(
                scrolls.len(),
                1,
                "pane {index} mounts exactly one scroll surface"
            );
            let associated = controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .expect("the pane's scroll view associates for the bottom edge");
            assert_eq!(Retained::as_ptr(&associated), Retained::as_ptr(&scrolls[0]));
        }
    }

    /// A scroll-free pane answers exactly `None`, and a scrolled pane
    /// associates nothing for any edge but the bottom one — the tab bar
    /// only tracks a surface it actually gets.
    pub fn panes_without_surfaces_answer_none() {
        let pane = binding(Pane::One);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![
                Tab::container(Pane::One, label("One"), || scroll(text("scrollable"))),
                Tab::container(Pane::Two, label("Two"), || text("static")),
            ],
        ));
        let controllers = tab_bar_controller(&leaf)
            .viewControllers()
            .expect("tabs installed");
        let scrolled = controllers.objectAtIndex(0);
        assert!(
            scrolled
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .is_some(),
            "the scrolled pane associates for the bottom edge"
        );
        for edge in [
            NSDirectionalRectEdge::Top,
            NSDirectionalRectEdge::Leading,
            NSDirectionalRectEdge::Trailing,
        ] {
            assert!(
                scrolled.contentScrollViewForEdge(edge).is_none(),
                "edge {edge:?} associates nothing"
            );
        }
        assert!(
            controllers
                .objectAtIndex(1)
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .is_none(),
            "a scroll-free pane associates nothing"
        );
    }

    /// A `Dynamic` pane replacement re-answers through the same declared
    /// chain: swap in fresh scroll content and the next query reports
    /// the new surface; swap to scroll-free content and the answer is
    /// `None` exactly. The replaced scroll deallocates once the pools
    /// drain — nothing in the tab controller retains it.
    pub fn pane_scroll_view_tracks_dynamic_replacement() {
        use cocoa_ui::objc2::rc::{Weak, autoreleasepool};
        use waterui::component::Dynamic;
        use waterui_backend_core::AnyView;

        let scrolled = binding(true);
        let content = scrolled.clone();
        let pane = binding(Pane::One);
        // Render inside a bounded pool: the leaf's own `Retained`/`Rc`
        // ownership survives, while every autoreleased temporary the
        // mount produced drains now — the weak read later must not meet
        // a render-time retainer in the harness's outer pool.
        let leaf = autoreleasepool(|_| {
            resolve::render(Tabs::new(
                &pane,
                vec![Tab::container(Pane::One, label("One"), move || {
                    Dynamic::watch(content.clone(), |scrolled| {
                        if scrolled {
                            AnyView::new(scroll(text("surface")))
                        } else {
                            AnyView::new(text("plain"))
                        }
                    })
                })],
            ))
        });
        let controller = autoreleasepool(|_| {
            tab_bar_controller(&leaf)
                .viewControllers()
                .expect("tabs installed")
                .objectAtIndex(0)
        });
        // Every association/query temporary drains before the swap: the
        // weak read afterwards must not meet an autoreleased retainer.
        let weak_old = autoreleasepool(|_| {
            let old_scrolls = scroll_views_in(&controller.view().expect("the pane's root view"));
            assert_eq!(old_scrolls.len(), 1, "the fixture mounts one scroll");
            let associated = controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .expect("the scroll associates for the bottom edge");
            assert_eq!(
                Retained::as_ptr(&associated),
                Retained::as_ptr(&old_scrolls[0])
            );
            Weak::from_retained(&old_scrolls[0])
        });

        // Replacing the pane's content inside a bounded pool keeps UIKit
        // autorelease temporaries from holding the old surface past the
        // swap — the weak read happens after the drain.
        autoreleasepool(|_| {
            scrolled.set(false);
        });
        assert!(
            autoreleasepool(|_| controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .is_none()),
            "a scroll-free replacement answers None"
        );
        assert!(
            weak_old.load().is_none(),
            "the replaced scroll is deallocated — nothing retained it"
        );

        // Swapping scroll content back in mounts a fresh surface the next
        // query reports — not the released one.
        autoreleasepool(|_| {
            scrolled.set(true);
            let new_scrolls = scroll_views_in(&controller.view().expect("the pane's root view"));
            assert_eq!(new_scrolls.len(), 1, "the replacement mounts one scroll");
            let associated = controller
                .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
                .expect("the new scroll associates for the bottom edge");
            assert_eq!(
                Retained::as_ptr(&associated),
                Retained::as_ptr(&new_scrolls[0])
            );
        });
    }

    /// The association answers the *current* surface on every `UIKit`
    /// query, not a pointer captured at install: a native push reports
    /// the pushed page's scroll and a pop reports the root's again.
    /// Each known scroll reference comes straight off the page
    /// controller's root — the fixture never re-walks the declared
    /// chain.
    pub fn pane_scroll_view_tracks_nav_pushes() {
        /// The `UINavigationController` owning a view in the subtree,
        /// depth-first — either the view's own controller is the nav
        /// controller, or the page controller answers one.
        fn nav_controller_in(
            view: &PlatformView,
        ) -> Option<Retained<cocoa_ui::objc2_ui_kit::UINavigationController>> {
            if let Some(controller) = owning_controller(view) {
                if let Ok(nav) = controller.clone().downcast() {
                    return Some(nav);
                }
                if let Some(nav) = controller.navigationController() {
                    return Some(nav);
                }
            }
            for sub in view::subviews(view) {
                if let Some(found) = nav_controller_in(&sub) {
                    return Some(found);
                }
            }
            None
        }

        let mtm = super::mtm();
        let pane = binding(Pane::One);
        let leaf = resolve::render(Tabs::new(
            &pane,
            vec![Tab::container(Pane::One, label("One"), || {
                NavigationStack::new(NavigationView::new("Root", scroll(text("root"))))
            })],
        ));
        let controller = tab_bar_controller(&leaf)
            .viewControllers()
            .expect("tabs installed")
            .objectAtIndex(0);
        let root = controller.view().expect("the pane's root view");
        let nav = nav_controller_in(&root).expect("the pane mounts a stack");
        let root_scrolls = scroll_views_in(
            &nav.topViewController()
                .expect("the root page")
                .view()
                .expect("the root page's view"),
        );
        assert_eq!(root_scrolls.len(), 1, "the root page mounts one scroll");
        let root_scroll = &root_scrolls[0];
        let associated = controller
            .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
            .expect("the root scroll associates for the bottom edge");
        assert_eq!(Retained::as_ptr(&associated), Retained::as_ptr(root_scroll));

        // A native push installs a page whose root view *is* its scroll
        // surface — the same shape `UITableView` pages take — and the
        // next query must answer it rather than the root's.
        let pushed_scroll = UIScrollView::new(mtm);
        let pushed = NavContentController::new(mtm, &pushed_scroll);
        nav.pushViewController_animated(&pushed, false);
        let associated = controller
            .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
            .expect("the pushed scroll associates for the bottom edge");
        assert_eq!(
            Retained::as_ptr(&associated),
            Retained::as_ptr(&pushed_scroll)
        );

        // A native pop returns the association to the root's surface.
        nav.popViewControllerAnimated(false);
        let associated = controller
            .contentScrollViewForEdge(NSDirectionalRectEdge::Bottom)
            .expect("the root scroll associates again");
        assert_eq!(Retained::as_ptr(&associated), Retained::as_ptr(root_scroll));
    }

    /// The accessory's child answers the measure itself — the host
    /// centers that answer inside bounds and never clamps it: wider
    /// bounds center it, tighter bounds let it overflow centered. A
    /// stretching child fills the bounds it measured back.
    pub fn accessory_layout_centers_the_measured_answer() {
        fn child_frame(
            content: &PlatformView,
            size: cocoa_ui::geometry::Size,
        ) -> cocoa_ui::geometry::Rect {
            view::set_frame(
                content,
                cocoa_ui::geometry::Rect::new(0.0, 0.0, size.width, size.height),
            );
            content.setNeedsLayout();
            content.layoutIfNeeded();
            view::frame(&view::subviews(content)[0])
        }

        let (_pane, leaf) = tabs(text("Now Playing"));
        let accessory = tab_bar_controller(&leaf)
            .bottomAccessory()
            .expect("installed");
        let content = accessory.contentView();
        let intrinsic = view::fitting_size(&content);
        assert!(intrinsic.width > 0.0 && intrinsic.height > 0.0);

        // Wider bounds center the answer inside the host — it is not
        // force-filled. Tighter bounds keep it unclamped: the answer
        // overflows, still centered, so both a clamp and a fill would
        // fail the same assertions.
        let wide = cocoa_ui::geometry::Size::new(intrinsic.width * 2.0, intrinsic.height * 2.0);
        let wide_frame = child_frame(&content, wide);
        assert!(wide_frame.size.width < wide.width);
        assert!((wide_frame.origin.x - (wide.width - wide_frame.size.width) / 2.0).abs() < 0.01);
        assert!((wide_frame.origin.y - (wide.height - wide_frame.size.height) / 2.0).abs() < 0.01);

        let tight = cocoa_ui::geometry::Size::new(intrinsic.width / 2.0, intrinsic.height / 2.0);
        let tight_frame = child_frame(&content, tight);
        assert!(
            tight_frame.size.width > tight.width || tight_frame.size.height > tight.height,
            "the measured answer overflows the offered bounds instead of clamping"
        );
        assert!((tight_frame.origin.x - (tight.width - tight_frame.size.width) / 2.0).abs() < 0.01);
        assert!(
            (tight_frame.origin.y - (tight.height - tight_frame.size.height) / 2.0).abs() < 0.01
        );

        // A stretching child measures back to the bounds it was offered
        // and fills the host.
        let (_pane, leaf) = tabs(scroll(text("stretched")));
        let accessory = tab_bar_controller(&leaf)
            .bottomAccessory()
            .expect("installed");
        let content = accessory.contentView();
        let frame = child_frame(&content, wide);
        assert_eq!(frame.size, wide);
    }
}

/// Window lifecycle on a real, never-shown `NSWindow` — only reachable
/// because the harness runs on the true main thread, which is the only
/// place `-[NSWindow init]` is legal. The assertion bodies live in the
/// crate's `native-test` feature, which owns the private reach
/// into `windows` and `embedding`.
#[cfg(all(target_os = "macos", feature = "native-test"))]
mod window {
    pub use waterui_apple::native_test_support::{
        bind_root_window_wires_a_live_window, manager_installs_into_the_environment,
    };
}

/// GPU-surface ownership regression coverage (#1725): a real mounted
/// `SceneView` — production `build_surface`, `SurfaceState`,
/// `SceneRenderer`/`SceneEngine` — driven through the capturable
/// sequence, the publication wait and the readiness contract. Event
/// orderings the GPU's own timing cannot pin down are delivered by
/// calling the production settle bodies directly in the order under
/// test — reported as deterministic seam calls, not physical cadence or
/// real link deliveries.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod gpu_surface {
    use std::rc::Rc;

    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::ErrorLog;
    use waterui_apple::native_test_support::gpu_surface::MountedSceneSurface;

    use super::mtm;

    /// The registered trials — the settle-ordering and completion
    /// coverage for the mounted-surface ownership contract.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "gpu_surface::failed_render_settles_readiness_once",
                failed_render_settles_readiness_once,
            ),
            Trial::test(
                "gpu_surface::external_capture_submission_completes_ok",
                external_capture_submission_completes_ok,
            ),
            Trial::test(
                "gpu_surface::failure_inside_a_capture_scope_still_balances",
                failure_inside_a_capture_scope_still_balances,
            ),
            Trial::test(
                "gpu_surface::publication_park_inside_a_capture_scope_stays_balanced",
                publication_park_inside_a_capture_scope_stays_balanced,
            ),
            Trial::test(
                "gpu_surface::capture_opening_over_a_parked_wait_stays_balanced",
                capture_opening_over_a_parked_wait_stays_balanced,
            ),
            Trial::test(
                "gpu_surface::capture_opening_over_a_failed_hold_balances",
                capture_opening_over_a_failed_hold_balances,
            ),
            Trial::test(
                "gpu_surface::a_failed_child_answers_the_terminal_capture_outcome_once",
                a_failed_child_answers_the_terminal_capture_outcome_once,
            ),
            Trial::test(
                "gpu_surface::a_failed_batch_settles_its_requester_once",
                a_failed_batch_settles_its_requester_once,
            ),
            Trial::test(
                "gpu_surface::subpixel_extents_round_up_and_empty_bounds_detach",
                subpixel_extents_round_up_and_empty_bounds_detach,
            ),
            Trial::test(
                "gpu_surface::an_unwritten_production_drops_the_frame_unpresented",
                an_unwritten_production_drops_the_frame_unpresented,
            ),
            Trial::test(
                "gpu_surface::an_animating_scene_presents_its_frame",
                an_animating_scene_presents_its_frame,
            ),
            Trial::test(
                "gpu_surface::a_deferred_external_frame_requests_a_redraw",
                a_deferred_external_frame_requests_a_redraw,
            ),
            Trial::test(
                "gpu_surface::view_render_answers_gpu_failure_over_a_failed_child",
                view_render_answers_gpu_failure_over_a_failed_child,
            ),
            Trial::test(
                "gpu_surface::view_render_succeeds_over_a_healthy_gpu_child",
                view_render_succeeds_over_a_healthy_gpu_child,
            ),
        ]
    }

    /// A typed frame-path failure settling on the owner resolves its
    /// readiness exactly once — the waiter registered through the real
    /// `Capturable::register_waiter` fires, a waiter armed after the
    /// settle never wakes — and the failed owner stops offering its
    /// first frame. The settle is driven through `settle_failed` on the
    /// live generation — the entry the render `Err` arm takes (no
    /// renderable texture extent produces that `Err` itself: the
    /// platform's maximum texture dimension equals the surface's
    /// rejection bound).
    pub fn failed_render_settles_readiness_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_fixture_window();
        let probe = mounted.readiness_probe();
        mounted.settle_failure(mounted.current_generation());
        assert_eq!(
            probe.wakes(),
            1,
            "the registered readiness waiter settles exactly once"
        );
        assert!(
            !mounted.frame_presented(),
            "a failed frame presents no receipt"
        );
        assert!(
            !mounted.first_paint_participation(),
            "a failed surface stops offering its first frame"
        );
        let late = mounted.readiness_probe();
        assert_eq!(
            late.wakes(),
            0,
            "a settled owner never arms a later readiness waiter"
        );
        Ok(())
    }

    /// The live submission completion contract on a runner where the
    /// display link never paces: a real external capture renders and
    /// submits through the production sequence —
    /// `render_to_metal_texture` → `submit_with_completion` →
    /// `on_submitted_work_done` → `submission_validity` — and answers
    /// `Ok(())` on a healthy context, never a synthesized deferral.
    /// Onscreen `PresentedFrame` readiness is verified on
    /// physical-device runs, where the link delivers.
    pub fn external_capture_submission_completes_ok() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted
            .install_scene_renderer()
            .map_err(|error| format!("the production renderer install: {error}"))?;
        assert!(
            mounted.capture_external_once().is_ok(),
            "the live submission answers Ok through the shared validity contract"
        );
        assert!(
            mounted.first_paint_participation(),
            "the capture scope restored the presenting surface"
        );
        Ok(())
    }

    /// A batch failure routed to the owner inside an open capture scope
    /// lands its `settle_failed` on the main queue without disturbing
    /// the scope: `end_external_rendering` still matches its `begin`,
    /// and the failed hold survives the scope's close — the surface
    /// keeps refusing a first frame after it.
    pub fn failure_inside_a_capture_scope_still_balances() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        let probe = mounted.readiness_probe();
        mounted.begin_capture(Rc::new(|| {}));
        mounted.route_failure(mounted.current_generation());
        assert!(
            mounted.pump_main(5.0, || !mounted.first_paint_participation()),
            "the routed settle lands inside the open scope"
        );
        mounted.end_capture(true);
        assert_eq!(
            probe.wakes(),
            1,
            "the settle inside the scope resolved readiness once"
        );
        assert!(
            !mounted.first_paint_participation(),
            "the failed hold survives the scope's close"
        );
        Ok(())
    }

    /// A publication wait armed inside an open capture scope: the scope
    /// still closes balanced, a render against the parked surface
    /// answers `Err(CaptureError::Deferred)` — the park owes its next
    /// frame to the publication wake, not to a retry on the same context
    /// — and the surface keeps offering its first frame, since a park is
    /// not a failure.
    pub fn publication_park_inside_a_capture_scope_stays_balanced()
    -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.begin_capture(Rc::new(|| {}));
        mounted.park_on(mounted.current_generation());
        mounted.end_capture(true);
        assert!(
            matches!(
                mounted.capture_external_once(),
                Err(cocoa_ui::capture::CaptureError::Deferred)
            ),
            "a render against the parked hold answers Err(CaptureError::Deferred)"
        );
        assert!(
            mounted.first_paint_participation(),
            "a parked surface still offers its first frame"
        );
        Ok(())
    }

    /// A capture scope opening over an already-parked wait: the scope
    /// balances, the render inside it answers `Err(CaptureError::Deferred)`
    /// rather than retrying the parked frame, and the surface keeps
    /// offering its first frame.
    pub fn capture_opening_over_a_parked_wait_stays_balanced() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.park_on(mounted.current_generation());
        mounted.begin_capture(Rc::new(|| {}));
        assert!(
            matches!(
                mounted.render_capture_frame(64, 64),
                Err(cocoa_ui::capture::CaptureError::Deferred)
            ),
            "a render over the parked hold answers Err(CaptureError::Deferred)"
        );
        mounted.end_capture(true);
        assert!(
            mounted.first_paint_participation(),
            "a parked surface still offers its first frame"
        );
        Ok(())
    }

    /// A capture scope opening over a settled failure: the scope
    /// balances — `end_external_rendering` matches — the render inside
    /// answers `Err(CaptureError::Failed)` carrying the settled typed
    /// failure rather than retrying the failed frame, and the failed
    /// hold is untouched by the scope's open and close.
    pub fn capture_opening_over_a_failed_hold_balances() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_fixture_window();
        mounted.settle_failure(mounted.current_generation());
        mounted.begin_capture(Rc::new(|| {}));
        assert!(
            matches!(
                mounted.render_capture_frame(64, 64),
                Err(cocoa_ui::capture::CaptureError::Failed(_))
            ),
            "a render over the failed hold answers Err(CaptureError::Failed)"
        );
        mounted.end_capture(true);
        assert!(
            !mounted.first_paint_participation(),
            "the failed hold survives the scope's open and close"
        );
        Ok(())
    }

    /// The terminal capture contract for a failed child: a settled
    /// failure inside an open capture scope makes the render's
    /// completion answer `Err(CaptureError::Failed)` — the typed surface
    /// failure itself — once, without waiting on a redraw the failed
    /// context generation can never issue.
    pub fn a_failed_child_answers_the_terminal_capture_outcome_once()
    -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        mounted.begin_capture(Rc::new(|| {}));
        mounted.settle_failure(mounted.current_generation());
        let Err(cocoa_ui::capture::CaptureError::Failed(error)) =
            mounted.render_capture_frame(64, 64)
        else {
            return Err(
                "the failed surface must answer the terminal outcome, not a deferral".into(),
            );
        };
        assert!(
            matches!(
                error.downcast_ref::<waterui_graphics::gpu::runtime::HostedLayerError>(),
                Some(waterui_graphics::gpu::runtime::HostedLayerError::Surface(
                    waterui_graphics::cherenkov::SurfaceError::TooLarge { .. }
                ))
            ),
            "the terminal outcome carries the settled typed failure: {error}"
        );
        mounted.end_capture(true);
        Ok(())
    }

    /// The requesting owner of a failed batch settles exactly once:
    /// the test drives the settle-then-routed-copy ordering directly —
    /// `settle_failure` plays `produce`'s `Err` arm and `route_failure`
    /// plays the copy `EngineGeneration::settle_failed` delivers through
    /// the requester's sink; it never runs `produce`. The routed settle
    /// is a no-op on the same generation — one failure log, one
    /// readiness resolution, and the `Failed` hold stands.
    pub fn a_failed_batch_settles_its_requester_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        let probe = mounted.readiness_probe();
        let (log, errors) = ErrorLog::new("waterui_apple::components::gpu_surface");
        let generation = mounted.current_generation();
        let drained = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        tracing::subscriber::with_default(log, || {
            // The requester's own `Err` arm — `produce`'s synchronous settle.
            mounted.settle_failure(generation);
            // The routed copy the batch delivered back through the
            // requester's own sink — must land as a no-op. A marker block
            // queued behind it proves the drain: the main queue is FIFO.
            mounted.route_failure(generation);
            {
                let drained = drained.clone();
                cocoa_ui::main_queue::enqueue(move |_| {
                    drained.store(true, std::sync::atomic::Ordering::SeqCst);
                });
            }
            assert!(
                mounted.pump_main(5.0, || drained.load(std::sync::atomic::Ordering::SeqCst)),
                "the routed settle drains off the main queue"
            );
        });
        assert_eq!(
            errors.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the requester of a failed batch logs its settle exactly once"
        );
        assert_eq!(
            probe.wakes(),
            1,
            "the requester of a failed batch resolves readiness exactly once"
        );
        assert!(
            !mounted.first_paint_participation(),
            "the failed hold survives the routed copy"
        );
        Ok(())
    }

    /// The drawable extent is whole device pixels, never a truncated
    /// zero: a positive sub-pixel bound rounds up through
    /// `initialize_gpu` — the production door layout and backing changes
    /// run — and a genuinely empty view stops presenting entirely
    /// instead of carrying a zero size beside a live drawable
    /// configuration; restored bounds re-enter through the same door.
    pub fn subpixel_extents_round_up_and_empty_bounds_detach() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_fixture_window();
        // 0.25 pt truncates to 0 device pixels at 1x, 2x and 3x alike —
        // only ceil-rounding produces a nonzero drawable.
        mounted.set_view_frame(0.25, 0.25);
        mounted.initialize_gpu();
        assert_eq!(
            mounted.drawable_size(),
            Some((1, 1)),
            "0.25 pt of bounds still yields one device pixel of drawable"
        );

        // Genuinely empty: the attach epoch drops — no zero-size drawable
        // configuration survives beside it, so the surface stops
        // offering a frame to capture or reveal.
        mounted.set_view_frame(0.0, 0.0);
        let bounds = mounted.view_bounds();
        assert_eq!(
            (bounds.width, bounds.height),
            (0.0, 0.0),
            "the zero bounds write lands"
        );
        mounted.initialize_gpu();
        assert_eq!(
            mounted.drawable_size(),
            None,
            "a genuinely empty view carries no live drawable configuration"
        );

        // Restored bounds re-enter through the same `attach` door.
        mounted.set_view_frame(0.25, 0.25);
        mounted.initialize_gpu();
        assert_eq!(
            mounted.drawable_size(),
            Some((1, 1)),
            "restored bounds re-attach through initialize_gpu"
        );
        Ok(())
    }

    /// An onscreen frame whose scene was invalidated after the batch —
    /// `present` answers `Next::At` and the drawable target is never
    /// written — is dropped unpresented and the frame re-owed, so the
    /// recycled drawable never reaches the screen. The trial produces
    /// the shared batch at the delivered timestamp from a sibling
    /// surface on the same environment, invalidates the sibling's scene,
    /// then issues a real drawable checked out of a standalone
    /// `CAMetalLayer` (a link-bound layer forbids `nextDrawable`):
    /// `render_drawable` itself runs the guard, and the bound
    /// renderer's `wrote_target` report proves the guard saw the
    /// unwritten frame.
    ///
    /// The invalidation is what makes the order deterministic: the
    /// sibling's link may or may not deliver a first-paint frame during
    /// the window spin, but between `invalidate_scene` and
    /// `deliver_frame` there is no run-loop turn for any queued work —
    /// link delivery or redraw — to re-prepare the scene before the
    /// guard reads it.
    pub fn an_unwritten_production_drops_the_frame_unpresented() -> Result<(), libtest_mimic::Failed>
    {
        let mtm = mtm();
        let (producer, surface) = pollster::block_on(MountedSceneSurface::mount_pair(mtm))
            .map_err(|error| format!("two mounted SceneView surfaces: {error}"))?;
        producer
            .install_scene_renderer()
            .map_err(|error| format!("the producing surface's renderer: {error}"))?;
        surface.set_view_frame(64.0, 64.0);
        surface.ensure_attached();
        assert_eq!(
            surface.presenter_pixel_format(),
            Some(surface.capture_pixel_format()),
            "the presenter's layer carries the surface's declared presentation format"
        );
        let media_time = cocoa_ui::objc2_quartz_core::CACurrentMediaTime();
        let target_time = surface.map_frame_time(media_time);
        producer
            .render_scene_frame_at(target_time)
            .map_err(|error| format!("the producing surface's frame: {error}"))?;
        surface.invalidate_scene();
        assert!(
            surface.deliver_frame(media_time),
            "the standalone layer issued a drawable frame"
        );
        assert_eq!(
            surface.wrote_target(target_time),
            Some(false),
            "the bound renderer reported the target unwritten"
        );
        assert!(
            surface.frame_owed(),
            "the unwritten frame stays owed for the next production"
        );
        Ok(())
    }

    /// A scene whose `build_scene` always answers `true` —
    /// self-animating content — still produces the delivered
    /// timestamp: the batch's `again` only schedules the next frame, so
    /// the bound renderer reports the target written and the frame takes
    /// the submit path instead of dropping unwritten and owed. The
    /// drawable checked out of the standalone layer can mint no
    /// receipt — the bound renderer's `wrote_target` report and the
    /// cleared owed flag prove the frame was submitted for presentation.
    pub fn an_animating_scene_presents_its_frame() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let surface = pollster::block_on(MountedSceneSurface::mount_animating(mtm))
            .map_err(|error| format!("a mounted animating SceneView surface: {error}"))?;
        surface.set_view_frame(64.0, 64.0);
        surface.ensure_attached();
        let media_time = cocoa_ui::objc2_quartz_core::CACurrentMediaTime();
        let target_time = surface.map_frame_time(media_time);
        assert!(
            surface.deliver_frame(media_time),
            "the standalone layer issued a drawable frame"
        );
        assert_eq!(
            surface.wrote_target(target_time),
            Some(true),
            "the animating scene's delivered frame wrote its target"
        );
        assert!(
            !surface.frame_owed(),
            "the produced frame was submitted, not re-owed"
        );
        Ok(())
    }

    /// `ViewRenderer::render` over a GPU child on a sealed-failure
    /// generation resolves `Err(RenderError::Gpu(_))` — the Arc'd
    /// `HostedLayerError` stays in `source()` — instead of hanging or
    /// panicking: the central capture's terminal `Failed` arm answers
    /// the typed outcome for a surface that can never produce a frame
    /// on the failed context generation.
    pub fn view_render_answers_gpu_failure_over_a_failed_child() -> Result<(), libtest_mimic::Failed>
    {
        use waterui_apple::dispatch;
        use waterui_apple::native_test_support::gpu_surface::{fixture_env, fixture_scene_view};
        use waterui_apple::native_test_support::view_renderer::{
            GpuRuntime, HostedLayerError, install_service, scene_engine,
        };
        use waterui_backend_core::AnyView;
        use waterui_core::view_renderer::{RenderError, RenderSize, ViewRenderer};
        use waterui_graphics::cherenkov::SurfaceError;

        let (runtime, mut env) = pollster::block_on(async {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            let env = fixture_env(runtime.clone());
            Ok::<_, String>((runtime, env))
        })
        .map_err(|error| format!("the fixture environment: {error}"))?;
        dispatch::install(&mut env);
        install_service(&mut env);

        // Seal the shared generation the rendered leaf's `SceneView`
        // mounts on — the same typed carrier a real surface rejection
        // produces.
        let generation = scene_engine(&env)
            .generation(&runtime, &runtime.context())
            .map_err(|error| format!("the shared engine generation: {error}"))?;
        let max = runtime.context().device().limits().max_texture_dimension_2d;
        generation.seal_failure_for_test(HostedLayerError::Surface(SurfaceError::TooLarge {
            width: u32::MAX,
            height: u32::MAX,
            max,
        }));

        let renderer = env
            .get::<ViewRenderer>()
            .ok_or("the installed ViewRenderer service")?;
        // `block_on_main`, not `pollster`: the capture's completions
        // land on `DispatchQueue::main()`, which only a turning run
        // loop services.
        let Err(RenderError::Gpu(cause)) = waterui_apple::native_test_support::block_on_main(
            30.0,
            renderer.render(
                AnyView::new(fixture_scene_view()),
                RenderSize::new(64.0, 64.0),
            ),
        ) else {
            return Err(
                "a capture over the failed GPU child resolves Err(RenderError::Gpu)".into(),
            );
        };
        // The Arc'd `HostedLayerError` must stay reachable down the
        // `source()` chain through the `GpuSurfaceFailed` wrapper.
        let mut link: &dyn std::error::Error = &*cause;
        let reachable = loop {
            if let Some(HostedLayerError::Surface(..)) = link.downcast_ref::<HostedLayerError>() {
                break true;
            }
            let Some(next) = link.source() else {
                break false;
            };
            link = next;
        };
        assert!(
            reachable,
            "the Gpu cause's chain reaches the settled HostedLayerError"
        );
        Ok(())
    }

    /// A windowless `ViewRenderer::render` over a healthy `SceneView`
    /// resolves `Ok` with real pixels: the offscreen capture window
    /// declares `DynamicRange::Standard` for the hosted subtree (the
    /// render target is RGBA8), so the leaf's GPU surface resolves a
    /// concrete range instead of panicking on the missing display.
    pub fn view_render_succeeds_over_a_healthy_gpu_child() -> Result<(), libtest_mimic::Failed> {
        use waterui_apple::dispatch;
        use waterui_apple::native_test_support::gpu_surface::{fixture_env, fixture_scene_view};
        use waterui_apple::native_test_support::view_renderer::{GpuRuntime, install_service};
        use waterui_backend_core::AnyView;
        use waterui_core::view_renderer::{RenderSize, ViewRenderer};

        let (_runtime, mut env) = pollster::block_on(async {
            let runtime = GpuRuntime::new().await.map_err(|error| error.to_string())?;
            let env = fixture_env(runtime.clone());
            Ok::<_, String>((runtime, env))
        })
        .map_err(|error| format!("the fixture environment: {error}"))?;
        dispatch::install(&mut env);
        install_service(&mut env);

        let renderer = env
            .get::<ViewRenderer>()
            .ok_or("the installed ViewRenderer service")?;
        let result = waterui_apple::native_test_support::block_on_main(
            30.0,
            renderer.render(
                AnyView::new(fixture_scene_view()),
                RenderSize::new(64.0, 64.0),
            ),
        )
        .map_err(|error| format!("a render over the healthy GPU child resolves Ok: {error}"))?;
        // The rendered leaf is measured detached, so the capture rasters
        // at the backing scale a windowless view reports.
        let scale =
            cocoa_ui::view::backing_scale_factor(&cocoa_ui::PlatformView::new(super::mtm()));
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a 64-point extent at a backing scale is a small positive pixel count"
        )]
        let side = (64.0 * scale).ceil() as u32;
        assert_eq!(
            (result.width, result.height),
            (side, side),
            "the raster is the 64-point proposal at the backing scale"
        );
        let (texels, rest) = result.rgba_data.as_chunks::<4>();
        assert!(rest.is_empty(), "the raster is whole RGBA8 texels");
        assert_eq!(
            texels.len(),
            side as usize * side as usize,
            "the raster holds one RGBA8 texel per pixel"
        );
        assert!(
            texels.iter().all(|texel| *texel == [255, 255, 255, 255]),
            "every texel is the fixture's opaque white fill"
        );
        Ok(())
    }

    /// A deferred external frame replays through the open capture
    /// scope's own redraw notification: the production
    /// `defer_unwritten_external_frame` owes the frame and requests the
    /// redraw `handle_redraw_request` turns into the capture's `redraw`
    /// callback — the deferred capture re-renders without relying on an
    /// unrelated invalidation.
    pub fn a_deferred_external_frame_requests_a_redraw() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted.ensure_attached();
        let redraws = Rc::new(std::cell::Cell::new(0_u32));
        mounted.begin_capture({
            let redraws = Rc::clone(&redraws);
            Rc::new(move || {
                redraws.set(redraws.get() + 1);
            })
        });
        mounted.defer_external_frame();
        assert!(
            mounted.frame_owed(),
            "the deferred frame stays owed for the replay"
        );
        assert!(
            mounted.pump_main(5.0, || redraws.get() > 0),
            "the deferred frame replays through the capture's redraw notification"
        );
        mounted.end_capture(false);
        Ok(())
    }
}

/// Filtered-view settlement over a failed GPU-surface child: a real
/// filtered leaf mounted through the production `build_filtered_parts`
/// over a real `SceneView` child — the same `FilteredState`, capturable
/// registration and link wiring a mount installs. After the child
/// settles a typed failure, the carrier its own capturable answers a
/// parent capture with — `CaptureError::Failed` — is fed through the
/// filtered leaf's captured-frame completion exactly as
/// `finish_captured_frame` would deliver it, and the settle contract
/// runs against it: one error log, readiness resolves so capture
/// waiters never hang, and the link parks until a new context
/// generation publishes instead of retrying the failed one forever.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod filtered {
    use std::rc::Rc;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    use cocoa_ui::capture::CaptureError;
    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::ErrorLog;
    use waterui_apple::native_test_support::filtered::MountedFilteredSurface;
    use waterui_apple::native_test_support::{MAIN_QUEUE_DEADLINE, pump_main_until};
    use waterui_graphics::gpu::runtime::HostedLayerError;

    use super::mtm;

    /// The registered trials — the failed-child settle contract and the
    /// zero-bounds detach.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "filtered::a_view_over_a_failed_child_settles_once",
                a_view_over_a_failed_child_settles_once,
            ),
            Trial::test(
                "filtered::a_collapsed_view_detaches_and_presents_again",
                a_collapsed_view_detaches_and_presents_again,
            ),
            Trial::test(
                "filtered::a_deferred_capture_pauses_the_link_until_the_child_redraws",
                a_deferred_capture_pauses_the_link_until_the_child_redraws,
            ),
            Trial::test(
                "filtered::a_view_collapsed_mid_capture_renders_transparent",
                a_view_collapsed_mid_capture_renders_transparent,
            ),
        ]
    }

    /// A parent capture that snapshots a filtered view while it has area,
    /// then submits after a layout pass collapsed it, completes with the
    /// view's region transparent — what the screen shows for a collapsed
    /// view — instead of capturing zero-bounds content.
    pub fn a_view_collapsed_mid_capture_renders_transparent() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let filtered = pollster::block_on(MountedFilteredSurface::mount_filling(mtm))
            .map_err(|error| format!("a mounted filling filtered surface: {error}"))?;
        filtered.ensure_attached();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "the filtered view presents its first frame"
        );
        let parent = filtered.parent_capture();

        let control = parent.start();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || control.landed()),
            "the control capture completes"
        );
        let outcome = control.take();
        assert!(
            matches!(outcome, Some(Ok(()))),
            "the control capture lands, got {outcome:?}"
        );
        assert!(
            parent.pixels().iter().all(|pixel| pixel[3] == u8::MAX),
            "an uncollapsed filtered view fills its region of the parent's target"
        );

        let collapsed = parent.start();
        filtered.set_host_frame(0.0, 0.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || collapsed.landed()),
            "the capture over the view collapsed after its prepare completes"
        );
        let outcome = collapsed.take();
        assert!(
            matches!(outcome, Some(Ok(()))),
            "a capture over a view collapsed between prepare and submit lands, got {outcome:?}"
        );
        assert!(
            parent.pixels().iter().all(|pixel| *pixel == [0; 4]),
            "the collapsed view's region of the parent's target is transparent"
        );
        Ok(())
    }

    /// A frame whose capture defers on a live context — a nested filter
    /// whose effect setup has not landed — leaves the display link paused
    /// rather than recapturing every vsync, and the nested setup's redraw
    /// re-arms it so the owed frame lands once the deferral clears.
    pub fn a_deferred_capture_pauses_the_link_until_the_child_redraws()
    -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let (filtered, gate) =
            pollster::block_on(MountedFilteredSurface::mount_over_pending_filter(mtm))
                .map_err(|error| format!("a mounted nested filtered surface: {error}"))?;
        filtered.ensure_attached();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.nested_joined_capture()
                && !filtered.render_in_flight()),
            "the first frame's capture defers on the nested filter's pending setup"
        );
        assert!(
            filtered.link_paused(),
            "a live-context deferral leaves the link paused — no display-rate recapture"
        );
        assert!(
            !filtered.frame_presented(),
            "the deferred frame never presented"
        );

        gate.release();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "the nested setup's redraw re-arms the link and the owed frame lands"
        );
        Ok(())
    }

    /// A filtered view sizes its hidden content with its own bounds, and
    /// a host collapsed to zero detaches — no presenter, so no link
    /// remains to issue a frame over content with nothing to capture —
    /// until restored bounds re-attach it and it presents again.
    pub fn a_collapsed_view_detaches_and_presents_again() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let filtered = pollster::block_on(MountedFilteredSurface::mount(mtm))
            .map_err(|error| format!("a mounted filtered surface: {error}"))?;
        filtered.ensure_attached();
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "the attached filtered view presents its first frame"
        );
        assert_eq!(
            filtered.child.view_bounds(),
            filtered.host_bounds(),
            "the hidden content is framed with the host"
        );

        filtered.set_host_frame(0.0, 0.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !filtered.attached()
                && !filtered.has_presenter()),
            "a zero-sized filtered view detaches and drops its presenter"
        );

        // 0.4 pt is positive but under one device pixel at 1x and 2x: it
        // rounds up to whole pixels, attaches and presents a real frame.
        filtered.set_host_frame(0.4, 0.4);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "a positive sub-pixel filtered view attaches and presents"
        );
        // `n` is the ceiling of `extent` exactly when `n - 1 < extent <= n`.
        let extent = 0.4 * filtered.backing_scale();
        let (width, height) = filtered.input_size();
        for pixels in [width, height] {
            assert!(
                pixels > 0 && f64::from(pixels - 1) < extent && extent <= f64::from(pixels),
                "{extent} device pixels of bounds round up to whole pixels, got {width}x{height}"
            );
        }

        filtered.set_host_frame(0.0, 0.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || !filtered.attached()
                && !filtered.has_presenter()),
            "the sub-pixel view collapses and detaches again"
        );

        filtered.set_host_frame(64.0, 64.0);
        assert!(
            pump_main_until(MAIN_QUEUE_DEADLINE, || filtered.frame_presented()),
            "restored bounds re-attach the filtered view and it presents again"
        );
        Ok(())
    }

    /// The carrier a parent capture receives from the settled child —
    /// `Err(CaptureError::Failed)` carrying the child's typed failure,
    /// read through the real `begin_capture` → `render_capture_frame` →
    /// `end_capture` scope.
    fn failed_child_carrier(
        child: &waterui_apple::native_test_support::gpu_surface::MountedSceneSurface,
    ) -> Arc<dyn std::error::Error + Send + Sync> {
        child.begin_capture(Rc::new(|| {}));
        let outcome = child.render_capture_frame(64, 64);
        child.end_capture(false);
        match outcome {
            Err(CaptureError::Failed(error)) => error,
            _ => panic!("a settled child answers Failed to a parent capture"),
        }
    }

    /// A filtered view drawing on screen over a failed child settles the
    /// terminal failure once: readiness resolves for the waiter a
    /// first-paint collect queued, the link parks and no render remains
    /// in flight, and a subsequent parent capture over the filtered
    /// view answers `Err(Failed)` with the child's typed error rather
    /// than deferring forever. A repeat settle for the same generation
    /// logs no additional error.
    pub fn a_view_over_a_failed_child_settles_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let filtered = pollster::block_on(MountedFilteredSurface::mount(mtm))
            .map_err(|error| format!("a mounted filtered surface: {error}"))?;
        filtered.ensure_attached();
        let probe = filtered.readiness_probe();
        let generation = filtered.current_generation();

        // The child genuinely failed — settle it, then read the carrier
        // its own capturable answers a parent capture with.
        filtered.child.settle_failure(generation);
        let first = failed_child_carrier(&filtered.child);

        let (log, errors) = ErrorLog::new("waterui_apple::components::filtered");
        tracing::subscriber::with_default(log, || {
            filtered.settle_failed_capture(generation, first);
            // A later redraw of the failed child lands the same
            // terminal outcome on the same generation — logged once.
            let repeat = failed_child_carrier(&filtered.child);
            filtered.settle_failed_capture(generation, repeat);
        });
        assert_eq!(
            errors.load(Ordering::SeqCst),
            1,
            "the filtered leaf logs its failed settle exactly once"
        );
        assert_eq!(
            probe.wakes(),
            1,
            "the filtered leaf resolves its readiness waiter exactly once"
        );
        assert!(
            !filtered.first_paint_participation(),
            "the filtered view leaves first-paint waiting until a new context rebinds"
        );
        assert!(
            filtered.link_paused(),
            "the display link parks until a new context generation publishes"
        );
        assert!(
            !filtered.render_in_flight(),
            "no further render is in flight after the settle"
        );
        filtered.request_render();
        assert!(
            filtered.link_paused() && !filtered.render_in_flight(),
            "a scheduled frame can no longer take off on the failed generation"
        );

        // A parent capture over the filtered view reads the terminal
        // outcome — the child's typed failure, not a deferral.
        match filtered.capture_outcome() {
            Err(CaptureError::Failed(error)) => assert!(
                error.downcast_ref::<HostedLayerError>().is_some(),
                "the forwarded outcome carries the child's typed failure: {error}"
            ),
            other => {
                return Err(format!(
                    "the filtered view must answer Failed to a parent capture, got {other:?}"
                )
                .into());
            }
        }
        Ok(())
    }
}

/// `ViewController` boundary semantics (#1689): under real `UIKit`
/// containment — a `UIViewController` parent, `addChild`/`didMove`, a
/// native container smaller than the window — an embedded controller's
/// host view keeps the bounds its parent assigned through layout, resize
/// and reparenting, while a `window_root` controller still fills its
/// window. The old `viewWillLayoutSubviews` window-bounds override fails
/// the embedded trial on this same harness.
#[cfg(target_os = "ios")]
mod controller_bounds {
    use cocoa_ui::objc2_ui_kit::{UIView, UIViewController, UIWindow};
    use cocoa_ui::uikit::{ViewController, view_controller as vc, window_root};
    use cocoa_ui::{PlatformView, view};
    use objc2::{MainThreadOnly, msg_send};

    use super::{HostView, Retained, mtm};

    /// The `controller_bounds::` trials.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "controller_bounds::embedded_child_keeps_parent_bounds",
                || {
                    embedded_child_keeps_parent_bounds();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test("controller_bounds::window_root_host_fills_window", || {
                window_root_host_fills_window();
                Ok(())
            }),
        ]
    }

    fn window(frame: cocoa_ui::Rect) -> Retained<UIWindow> {
        // SAFETY: `initWithFrame:` is `UIWindow`'s plain initializer and
        // `mtm` proves the main-thread confinement the harness provides.
        unsafe {
            msg_send![
                UIWindow::alloc(mtm()),
                initWithFrame: cocoa_ui::objc2_core_foundation::CGRect::from(frame)
            ]
        }
    }

    fn view_controller() -> Retained<UIViewController> {
        // SAFETY: plain `UIViewController` init on the main thread.
        unsafe { msg_send![UIViewController::alloc(mtm()), init] }
    }

    fn plain_view(frame: cocoa_ui::Rect) -> Retained<UIView> {
        // SAFETY: plain `UIView` init on the main thread.
        unsafe {
            msg_send![
                UIView::alloc(mtm()),
                initWithFrame: cocoa_ui::objc2_core_foundation::CGRect::from(frame)
            ]
        }
    }

    /// Tears a window down after the trial: the child controllers leave
    /// their parents through the normal containment callbacks, the root is
    /// cleared and the window hidden.
    fn teardown(window: &UIWindow, children: &[&UIViewController]) {
        for child in children {
            vc::will_move_to_parent(child);
            child.view().unwrap().removeFromSuperview();
            vc::remove_from_parent(child);
        }
        window.setRootViewController(None);
        window.setHidden(true);
        window.layoutIfNeeded();
    }

    /// A `ViewController` embedded as a child of a real
    /// `UIViewController` inside a 340x460 native container — the
    /// `WaterUIHostController` shape. Its host view must keep the bounds
    /// the container assigns through layout, resize and reparenting: the
    /// unmodified window-bounds override would reset it to the window's.
    fn embedded_child_keeps_parent_bounds() {
        let mtm = mtm();
        let window = window(cocoa_ui::Rect::new(0.0, 0.0, 390.0, 844.0));
        let parent = view_controller();
        window.setRootViewController(Some(&parent));

        let container = plain_view(cocoa_ui::Rect::new(50.0, 140.0, 340.0, 460.0));
        parent.view().unwrap().addSubview(&container);

        let embedded = cocoa_ui::Rect::new(0.0, 0.0, 340.0, 460.0);
        let controller = ViewController::new(mtm, HostView::new(mtm, embedded));
        let host: &PlatformView = controller.host_view();

        vc::add_child(&parent, &controller);
        host.setFrame(cocoa_ui::objc2_core_foundation::CGRect::from(embedded));
        host.setAutoresizingMask(
            cocoa_ui::objc2_ui_kit::UIViewAutoresizing::FlexibleWidth
                | cocoa_ui::objc2_ui_kit::UIViewAutoresizing::FlexibleHeight,
        );
        container.addSubview(host);
        vc::did_move_to_parent(&controller);

        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        assert_eq!(
            view::frame(host).size,
            embedded.size,
            "embedded host must keep the parent-assigned bounds"
        );

        // A container resize flows through the normal layout path.
        container.setFrame(cocoa_ui::objc2_core_foundation::CGRect::from(
            cocoa_ui::Rect::new(50.0, 140.0, 300.0, 220.0),
        ));
        window.layoutIfNeeded();
        assert_eq!(
            view::frame(host).size,
            cocoa_ui::geometry::Size::new(300.0, 220.0),
            "resized embedded host tracks its container, not the window"
        );

        // Reparent under a second native container through the full
        // containment sequence.
        let other = plain_view(cocoa_ui::Rect::new(0.0, 0.0, 340.0, 460.0));
        parent.view().unwrap().addSubview(&other);
        vc::will_move_to_parent(&controller);
        host.removeFromSuperview();
        other.addSubview(host);
        host.setFrame(cocoa_ui::objc2_core_foundation::CGRect::from(
            cocoa_ui::Rect::new(0.0, 0.0, 340.0, 460.0),
        ));
        vc::did_move_to_parent(&controller);
        window.layoutIfNeeded();
        assert_eq!(
            view::frame(host).size,
            cocoa_ui::geometry::Size::new(340.0, 460.0),
            "reparented embedded host keeps the parent's assignment"
        );

        teardown(&window, &[&controller]);
    }

    /// The same controller type at a true window root — a `window_root`
    /// host view — still fills its window through layout.
    fn window_root_host_fills_window() {
        let mtm = mtm();
        let window = window(cocoa_ui::Rect::new(0.0, 0.0, 390.0, 844.0));
        let controller = ViewController::new(mtm, window_root(mtm));
        window.setRootViewController(Some(&controller));
        window.makeKeyAndVisible();
        window.layoutIfNeeded();
        let host: &PlatformView = controller.host_view();
        assert_eq!(
            view::frame(host).size,
            cocoa_ui::geometry::Size::new(390.0, 844.0),
            "window_root host must fill its window"
        );
        teardown(&window, &[]);
    }
}

/// Mounted-owner lifetimes (#1575): a real dispatcher-mounted hierarchy —
/// `.size`/`padding`/`hstack`/`zstack` containers plus a lazy `ForEach`
/// membership — must release its owners, child guards and host views when
/// the mounted tree drops. Weak handles into the tree's views die after
/// queued work drains, post-owner writes cease naturally, and a remount
/// of the same view still measures, binds and recycles membership.
mod owner_lifetimes {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use waterui::Identifiable;
    use waterui::component::lazy::Lazy;
    use waterui::prelude::*;
    use waterui::reactive::binding;
    use waterui::reactive::collection::List as ReactiveList;
    use waterui::views::ForEach;
    use waterui_core::layout::ProposalSize;

    use super::{HostView, Label, MainThreadMarker, PlatformView, Retained, leaf, mtm, resolve};

    /// A stable-id row for the `ForEach` membership.
    #[derive(Clone, Copy, Identifiable)]
    struct Row {
        #[id]
        id: u64,
    }

    /// Drains queued main-queue work — binding flushes and enqueued drops
    /// alike land at a real run-loop boundary. A sentinel block enqueued
    /// behind everything already queued bounds the wait: the main queue is
    /// FIFO, so the sentinel running means the work ahead of it ran.
    fn pump() {
        assert!(
            waterui_apple::native_test_support::drain_main_queue(mtm()),
            "the main queue must drain inside its deadline"
        );
    }

    /// Every `Label` payload in the subtree, depth-first — the observable
    /// end of the binding path on both kits.
    fn label_texts(view: &PlatformView, out: &mut Vec<String>) {
        for sub in cocoa_ui::view::subviews(view) {
            if let Some(label) = sub.downcast_ref::<Label>()
                && let Some(attributed) = label.source_text()
            {
                out.push(attributed.string().to_string());
            }
            label_texts(&sub, out);
        }
    }

    /// A weak handle for every view in the mounted subtree. `load`
    /// answering `None` is the observable proof the owner that retained
    /// the view is gone.
    fn weak_views(view: &PlatformView, out: &mut Vec<objc2::rc::Weak<PlatformView>>) {
        out.push(objc2::rc::Weak::new(&cocoa_ui::view::retain_base(view)));
        for sub in cocoa_ui::view::subviews(view) {
            weak_views(&sub, out);
        }
    }

    /// The hierarchy the repairs cover: `.size`/`padding` wrappers around
    /// `hstack`/`zstack` containers plus a lazy `ForEach` membership — one
    /// tree holding every repaired owner class.
    fn hierarchy(
        track: &waterui::reactive::Binding<String>,
        items: &ReactiveList<Row>,
    ) -> impl View {
        let track = track.clone();
        let items = items.clone();
        zstack((
            hstack((text!("{track}"), spacer()))
                .padding()
                .size(320.0, 60.0),
            Lazy::vstack(ForEach::new(items, |row: Row| {
                text(format!("row {}", row.id))
            })),
        ))
    }

    #[cfg(target_os = "macos")]
    use cocoa_ui::objc2_app_kit::{NSColor as PlatformColor, NSForegroundColorAttributeName};
    use cocoa_ui::objc2_core_graphics::CGColor;
    use cocoa_ui::objc2_foundation::NSAttributedString;
    #[cfg(target_os = "ios")]
    use cocoa_ui::objc2_ui_kit::{NSForegroundColorAttributeName, UIColor as PlatformColor};
    use waterui::graphics::color::WorkingColor;

    /// The attributed text's native foreground attribute read as its
    /// four extended-linear-P3 channels — the semantic value the
    /// default-foreground watcher writes, compared against the working
    /// color it was given rather than a rebuild pointer.
    fn assert_foreground(attributed: &NSAttributedString, expected: WorkingColor) {
        // SAFETY: a null effective-range pointer is permitted.
        let value = unsafe {
            attributed.attribute_atIndex_effectiveRange(
                NSForegroundColorAttributeName,
                0,
                std::ptr::null_mut(),
            )
        }
        .expect("the track chunk carries a foreground attribute");
        let color = value
            .downcast::<PlatformColor>()
            .expect("the foreground attribute is a platform color");
        #[cfg(target_os = "macos")]
        let cg = color.CGColor();
        #[cfg(target_os = "ios")]
        // SAFETY: the retained UIKit color is read on the actual main thread.
        let cg = unsafe { color.CGColor() };
        assert_eq!(CGColor::number_of_components(Some(&cg)), 4);
        // SAFETY: the color owns the four components asserted above.
        let actual = unsafe { std::slice::from_raw_parts(CGColor::components(Some(&cg)), 4) };
        for (actual, expected) in actual.iter().zip(expected.components) {
            assert!(
                (actual - f64::from(expected)).abs() < 1e-4,
                "native foreground channel {actual} differs from working channel {expected}"
            );
        }
    }

    /// The first `Label` inside the subtree — the observable end of the
    /// per-chunk signal path this fix touches.
    fn first_label(view: &PlatformView) -> Option<Retained<Label>> {
        for sub in cocoa_ui::view::subviews(view) {
            if let Some(label) = sub.downcast_ref::<Label>() {
                return Some(label.into());
            }
            if let Some(label) = first_label(&sub) {
                return Some(label);
            }
        }
        None
    }

    /// The class names of every subtree view that still lives — the
    /// observable list of real retainers a drop failed to release.
    fn surviving_classes(weaks: &[objc2::rc::Weak<PlatformView>]) -> Vec<String> {
        weaks
            .iter()
            .filter_map(|weak| {
                weak.load()
                    .map(|view| view.class().name().to_string_lossy().into_owned())
            })
            .collect()
    }

    /// Mounts `leaf` on a fresh host whose layout handler frames it.
    /// The host is a bare `HostView` that no leaf owns, so nothing clears
    /// its handlers; the handler borrows the child weakly so dropping the
    /// `Mounted` alone decides whether the child's views survive.
    fn mount_hosted(
        mtm: MainThreadMarker,
        leaf_inst: waterui_apple::contract::NativeLeaf,
    ) -> (Retained<HostView>, waterui_apple::contract::Mounted) {
        let parent = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let mounted = leaf_inst.mount(&parent);
        let child_view: objc2::rc::Weak<PlatformView> =
            objc2::rc::Weak::new(&cocoa_ui::view::retain_base(mounted.view()));
        parent.set_layout_handler(move |host| {
            let host_view: &PlatformView = host;
            if let Some(child) = child_view.load() {
                cocoa_ui::view::set_frame(&child, cocoa_ui::view::bounds(host_view));
            }
        });
        (parent, mounted)
    }

    /// One consolidated regression for #1575: mount the repaired owner
    /// classes in a single tree against ONE explicit environment, prove
    /// live updates still land, prove the drop releases the whole
    /// subtree, then prove a remount on the same env keeps working. The
    /// mount, the submitted work, the drop and the queued drain all live
    /// inside bounded `autoreleasepool`s — only `Weak` handles escape
    /// them, so a surviving read outside proves a real retainer, never a
    /// pooled temporary (mirrors the ownership fixture's idiom).
    pub fn a_dropped_mounted_hierarchy_releases_views_and_remounts() {
        let mtm = mtm();
        // The environment, binding, membership and per-chunk signal all
        // live outside the pools — a weak read that still answers `Some`
        // afterwards names a real retainer, not a pooled autorelease.
        let mut env = resolve::env();
        let track = binding(String::from("first"));
        let items = ReactiveList::from(vec![Row { id: 1 }, Row { id: 2 }]);
        // The theme `Foreground` slot backs every unstyled chunk — the
        // `label_leaf` default watcher this fix touches. Reinstall it
        // from a binding the test keeps, so a live update is reachable.
        let foreground = binding(waterui::graphics::color::WorkingColor::BLACK);
        waterui::theme::install_color_signal::<waterui::theme::color::Foreground>(
            &mut env,
            foreground.computed(),
        );

        let weaks = objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(hierarchy(&track, &items)),
                &env,
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);
            parent.set_needs_layout();
            parent.layout_if_needed();
            pump();
            parent.layout_if_needed();

            let mut weaks = Vec::new();
            weak_views(mounted.view(), &mut weaks);
            assert!(weaks.len() > 3, "the fixture must mount a real hierarchy");
            assert!(weaks.iter().all(|w| w.load().is_some()));

            // Submitted work while alive: the binding write lands on the
            // mounted text and the pushed member mounts through membership.
            track.set(String::from("second"));
            items.push(Row { id: 3 });
            parent.layout_if_needed();
            pump();
            parent.layout_if_needed();
            let mut texts = Vec::new();
            label_texts(mounted.view(), &mut texts);
            assert!(texts.iter().any(|t| t.contains("second")));
            assert!(texts.iter().any(|t| t.contains("row 3")));

            // The default-foreground signal path still updates while
            // live: a new theme foreground lands in the label's
            // attributed text as the native foreground attribute — the
            // `Weak`-borrowing watcher answers instead of being frozen.
            let label = first_label(mounted.view()).expect("the track label mounted");
            let before = label.source_text().expect("attributed text");
            assert_foreground(&before, WorkingColor::BLACK);
            foreground.set(WorkingColor::WHITE);
            pump();
            let after = label.source_text().expect("attributed text");
            assert_foreground(&after, WorkingColor::WHITE);

            drop(mounted);
            pump();
            weaks
        });

        let survivors = surviving_classes(&weaks);
        assert!(
            survivors.is_empty(),
            "mounted child views survived the owner drop: {survivors:?}"
        );

        // Post-owner callbacks cease naturally: a later write reaches no
        // dead owner and cannot panic through a cleared weak edge.
        objc2::rc::autoreleasepool(|_| {
            track.set(String::from("third"));
            foreground.set(waterui::graphics::color::WorkingColor::BLACK);
            pump();
        });

        // Remount on the same env: measurement, binding and membership
        // all still work, and the remounted tree releases too.
        let weaks = objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(hierarchy(&track, &items)),
                &env,
            );
            assert!(
                leaf_inst
                    .layout()
                    .measure(ProposalSize::new(Some(400.0), Some(200.0)))
                    .size
                    .width
                    > 0.0,
                "the remounted hierarchy still answers measure"
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);
            parent.set_needs_layout();
            parent.layout_if_needed();
            track.set(String::from("fourth"));
            items.push(Row { id: 4 });
            pump();
            parent.layout_if_needed();
            pump();
            let mut texts = Vec::new();
            label_texts(mounted.view(), &mut texts);
            assert!(texts.iter().any(|t| t.contains("fourth")));
            assert!(texts.iter().any(|t| t.contains("row 4")));
            let mut weaks = Vec::new();
            weak_views(mounted.view(), &mut weaks);
            drop(mounted);
            pump();
            weaks
        });
        let survivors = surviving_classes(&weaks);
        assert!(
            survivors.is_empty(),
            "the remounted hierarchy must release the same way: {survivors:?}"
        );
    }

    /// The views a scroll surface hosts as mounted content — the document's
    /// subviews on `AppKit`; the `HostView`-topped subviews on `UIKit`.
    /// The kit's own chrome (clip/document views, scroll indicators) is
    /// the scroll view's own property and legitimately survives with it.
    #[cfg(target_os = "macos")]
    fn scroll_content_subviews(scroll: &PlatformView) -> Vec<Retained<PlatformView>> {
        scroll
            .downcast_ref::<cocoa_ui::appkit::ScrollView>()
            .and_then(cocoa_ui::appkit::ScrollView::document_view)
            .map(|document| cocoa_ui::view::subviews(&document))
            .unwrap_or_default()
    }

    /// The `UIKit` scroll surface mounts the child on the scroll view
    /// itself; scroll indicators are not `HostView`s.
    #[cfg(target_os = "ios")]
    fn scroll_content_subviews(scroll: &PlatformView) -> Vec<Retained<PlatformView>> {
        cocoa_ui::view::subviews(scroll)
            .into_iter()
            .filter(|sub| sub.downcast_ref::<HostView>().is_some())
            .collect()
    }

    /// The trials this module registers — the same submodule-registry
    /// shape `migration`/`tabs`/`controller_bounds` use.
    pub fn trials() -> Vec<libtest_mimic::Trial> {
        vec![
            libtest_mimic::Trial::test(
                "owner_lifetimes::a_dropped_mounted_hierarchy_releases_views_and_remounts",
                || {
                    a_dropped_mounted_hierarchy_releases_views_and_remounts();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "owner_lifetimes::a_released_scroll_leaf_frees_its_content",
                || {
                    a_released_scroll_leaf_frees_its_content();
                    Ok(())
                },
            ),
            libtest_mimic::Trial::test(
                "owner_lifetimes::a_scroll_leaf_dropped_inside_its_own_layout_handler_does_not_abort",
                || {
                    a_scroll_leaf_dropped_inside_its_own_layout_handler_does_not_abort();
                    Ok(())
                },
            ),
        ]
    }

    /// The leaf's scroll surface as the kit type the handler API lives on,
    /// retained so the borrow the `Mounted` would tie up is released.
    #[cfg(target_os = "macos")]
    fn kit_scroll_view(view: &PlatformView) -> Retained<cocoa_ui::appkit::ScrollView> {
        cocoa_ui::view::retain_base(view)
            .downcast::<cocoa_ui::appkit::ScrollView>()
            .expect("a scroll leaf's view is the kit ScrollView")
    }
    /// The leaf's scroll surface as the kit type the handler API lives on,
    /// retained so the borrow the `Mounted` would tie up is released.
    #[cfg(target_os = "ios")]
    fn kit_scroll_view(view: &PlatformView) -> Retained<cocoa_ui::uikit::ScrollView> {
        cocoa_ui::view::retain_base(view)
            .downcast::<cocoa_ui::uikit::ScrollView>()
            .expect("a scroll leaf's view is the kit ScrollView")
    }

    /// #1908: a released scroll leaf frees its content. The scroll view's
    /// handler slots hold `ScrollContent` — which owns the mounted child
    /// leaf — so while the leaf lives, the scroll view retains the whole
    /// content subtree through its handlers. The `HandlerTeardown` guard
    /// in the leaf's keepalive clears the slots at the leaf's release
    /// boundary, the same teardown mounted `HostView` handlers get since
    /// #1860. Retaining the scroll view past the drop proves the slots,
    /// not the view's deallocation, do the release: everything the
    /// handlers pinned dies even though the view they belong to lives.
    pub fn a_released_scroll_leaf_frees_its_content() {
        let mtm = mtm();
        let env = resolve::env();

        let (_scroll_view, content_weaks) = objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(scroll(vstack((text!("top"), text!("bottom"))))),
                &env,
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);
            parent.set_needs_layout();
            parent.layout_if_needed();

            // While alive the layout handler framed the mounted content.
            let content = scroll_content_subviews(mounted.view());
            assert_eq!(content.len(), 1, "the scroll mounts one child view");
            let frame = cocoa_ui::view::frame(&content[0]);
            assert!(
                frame.size.width > 0.0 && frame.size.height > 0.0,
                "the layout handler must frame the mounted content while alive"
            );

            // Retain the scroll view itself past the leaf's drop, then take
            // weak handles into the mounted content subtree.
            let scroll_view = cocoa_ui::view::retain_base(mounted.view());
            let mut content_weaks = Vec::new();
            for sub in content {
                weak_views(&sub, &mut content_weaks);
            }
            assert!(
                content_weaks.iter().all(|weak| weak.load().is_some()),
                "the mounted content must be alive before the drop"
            );

            drop(mounted);
            (scroll_view, content_weaks)
        });

        // The scroll view lives — this test retains it — but every view
        // its handlers kept in the content subtree died with the leaf.
        let survivors = surviving_classes(&content_weaks);
        assert!(
            survivors.is_empty(),
            "scroll content views survived the leaf drop: {survivors:?}"
        );
    }

    /// A `clear_handlers` reached while its own handler is running — the
    /// re-entrant teardown `callback::emit`'s borrow release makes legal.
    /// The layout handler takes the mounted leaf out of a cell and drops
    /// it: the leaf's `HandlerTeardown` guard clears this same slot
    /// mid-callback. Borrowing the slot across the call would hit a live
    /// borrow here and `guarded` would abort the process; the surviving
    /// asserts are the proof it did not.
    pub fn a_scroll_leaf_dropped_inside_its_own_layout_handler_does_not_abort() {
        let mtm = mtm();
        let env = resolve::env();

        objc2::rc::autoreleasepool(|_| {
            let leaf_inst = waterui_apple::dispatch::render(
                waterui_backend_core::AnyView::new(scroll(text!("body"))),
                &env,
            );
            let (parent, mounted) = mount_hosted(mtm, leaf_inst);
            let _window = leaf::attach(mtm, &parent);

            let scroll_view = kit_scroll_view(mounted.view());
            // The leaf is the handler's to drop, in a cell it can empty
            // on its first run.
            let leaf_cell = Rc::new(RefCell::new(Some(mounted)));
            let runs = Rc::new(Cell::new(0u32));
            scroll_view.set_layout_handler({
                let leaf_cell = Rc::clone(&leaf_cell);
                let runs = Rc::clone(&runs);
                move |_| {
                    runs.set(runs.get() + 1);
                    drop(leaf_cell.borrow_mut().take());
                }
            });

            scroll_view.set_needs_layout();
            scroll_view.layout_if_needed();
            assert!(runs.get() >= 1, "the layout handler must run");
            assert!(
                leaf_cell.borrow().is_none(),
                "the handler must have dropped the leaf"
            );

            // The clear the drop performed is observable: a later layout
            // pass reaches an empty slot and runs nothing more.
            let observed = runs.get();
            scroll_view.set_needs_layout();
            scroll_view.layout_if_needed();
            assert_eq!(runs.get(), observed, "a cleared slot must not run again");
        });
    }
}
