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
            // Run after launch returns to AppKit's event loop. Foundation
            // run-loop pumping alone does not dispatch the application events
            // that establish the window's native occlusion visibility.
            let _ = mtm;
            libtest_mimic::run(&args, trials()).exit();
        }));
        unreachable!("the native trial runner exits the process");
    }
    libtest_mimic::run(&args, trials()).exit();
}

fn trials() -> Vec<Trial> {
    let tests = vec![
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
        Trial::test(
            "owner_lifetimes::a_dropped_mounted_hierarchy_releases_views_and_remounts",
            || {
                owner_lifetimes::a_dropped_mounted_hierarchy_releases_views_and_remounts();
                Ok(())
            },
        ),
    ];
    #[cfg(all(target_os = "macos", feature = "native-test"))]
    let tests = {
        let mut tests = tests;
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
        tests.extend(gpu_surface::trials());
        tests
    };
    #[cfg(target_os = "ios")]
    let tests = {
        let mut tests = tests;
        tests.extend(tabs::trials());
        tests.extend(controller_bounds::trials());
        tests
    };
    let mut tests = tests;
    tests.extend(migration::trials());
    tests
}

/// The marker the whole suite builds objects under — the real one, on the
/// thread `main` runs on.
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("the custom harness runs cases on the process's main thread")
}

/// `NativeLeaf` mount/watch/bind against real views.
mod leaf {
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
    /// back for reuse.
    pub fn mount_attaches_and_unmount_detaches() {
        let mtm = mtm();
        let parent = HostView::new(mtm, cocoa_ui::Rect::new(0.0, 0.0, 200.0, 100.0));
        let child = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let leaf = NativeLeaf::new(&*child, TestSubView);
        let mounted = leaf.mount(&parent);
        assert_eq!(cocoa_ui::view::subviews(&parent).len(), 1);
        let leaf = mounted.unmount();
        assert!(cocoa_ui::view::superview(leaf.view()).is_none());
        assert_eq!(cocoa_ui::view::subviews(&parent).len(), 0);
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
/// `SceneRenderer`/`SceneEngine` — driven through the failure drain and
/// the completion settlement seam. The ordering the GPU's own timing
/// cannot pin down is delivered by a direct call of the actual
/// `settle_frame_completion` body — reported as a deterministic seam
/// call, not a physical cadence or real GPU submission claim.
#[cfg(all(target_os = "macos", feature = "native-test", feature = "gpu_surface"))]
mod gpu_surface {
    use libtest_mimic::Trial;
    use waterui_apple::native_test_support::gpu_surface::MountedSceneSurface;

    use super::mtm;

    /// The registered trials — the completion/failure seam coverage for
    /// the mounted-surface ownership contract.
    pub fn trials() -> Vec<Trial> {
        vec![
            Trial::test(
                "gpu_surface::routed_failure_drains_idle_owner_once",
                routed_failure_drains_idle_owner_once,
            ),
            Trial::test(
                "gpu_surface::stale_completion_releases_only_its_lease",
                stale_completion_releases_only_its_lease,
            ),
        ]
    }

    /// Pumps the main run loop in small turns until `until` answers or
    /// `seconds` elapse — how a synchronous case awaits the main-queue
    /// work `request_redraw` enqueues. Bounded; a dead queue fails the
    /// case instead of hanging it.
    fn pump_main_until(seconds: f64, until: impl Fn() -> bool) -> bool {
        use cocoa_ui::objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSRunLoop};
        let deadline = NSDate::dateWithTimeIntervalSinceNow(seconds);
        while !until() && deadline.timeIntervalSinceNow() > 0.0 {
            // SAFETY: `NSDefaultRunLoopMode` is a system-owned run-loop mode.
            NSRunLoop::currentRunLoop().runMode_beforeDate(
                unsafe { NSDefaultRunLoopMode },
                &NSDate::dateWithTimeIntervalSinceNow(0.02),
            );
        }
        until()
    }

    /// A routed shared-generation failure reaches an owner that is idle
    /// — never attached, never presented, with a readiness waiter
    /// registered — through the production path: `ScenePart::note_failure`
    /// stores it and `RedrawHandle::request_redraw` enqueues
    /// `handle_redraw_request`, which drains it before the visibility,
    /// in-flight and external gates and settles through `settle_failed`:
    /// the owner is marked failed, the readiness waiter fires exactly
    /// once, the frame is owed, and the recovery watch on the failed
    /// generation arms. Re-producing the sealed generation answers the
    /// retained failure unchanged.
    pub fn routed_failure_drains_idle_owner_once() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted
            .install_scene_renderer()
            .map_err(|error| format!("the production renderer install: {error}"))?;
        let probe = mounted.readiness_probe();
        assert!(!mounted.owner_failed());
        assert!(
            mounted.frame_owed(),
            "registration requests the pending first paint"
        );

        // Route the failure: the generation seals, the participant's
        // owner wake lands on the main queue — the callback itself.
        mounted.fail_scene_generation();
        assert!(mounted.scene_generation_sealed());
        assert!(
            !mounted.owner_failed(),
            "the routed failure sits queued until the owner drains it"
        );

        assert!(
            pump_main_until(2.0, || mounted.owner_failed()),
            "the enqueued handle_redraw_request never drained the routed failure"
        );
        assert!(mounted.frame_owed(), "the failed frame stays owed");
        assert_eq!(
            probe.wakes(),
            1,
            "the registered readiness waiter settles exactly once"
        );
        assert!(
            mounted.context_watch_armed(),
            "the recovery watch arms on the failed generation"
        );
        assert!(
            mounted.sealed_produce_is_cached_error(),
            "the sealed generation never re-produces"
        );
        Ok(())
    }

    /// A stale completion after a genuinely newer publication: the
    /// retained old-generation submission releases only its own lease —
    /// `frame_owed`, `frame_in_flight` — and neither presents nor touches
    /// the newer epoch's readiness, failure flag or watch. The current
    /// generation still settles a completion normally.
    ///
    /// The seam is called directly for deterministic ordering — the real
    /// `settle_frame_completion` body, not a simulated path; no physical
    /// GPU submission/cadence is claimed.
    pub fn stale_completion_releases_only_its_lease() -> Result<(), libtest_mimic::Failed> {
        let mtm = mtm();
        let mounted = pollster::block_on(MountedSceneSurface::mount(mtm))
            .map_err(|error| format!("a mounted SceneView surface: {error}"))?;
        mounted
            .install_scene_renderer()
            .map_err(|error| format!("the production renderer install: {error}"))?;
        let probe = mounted.readiness_probe();
        assert!(
            mounted.begin_in_flight_frame(),
            "the presenter yields a real link-issued DrawableFrame"
        );

        assert!(
            pollster::block_on(mounted.publish_newer_context()),
            "the runtime must publish a genuinely newer context generation"
        );

        // The stale submission's completion runs the real seam: obsolete
        // first — it releases its lease and owes the work, nothing more.
        mounted.settle_submitted_completion();
        assert!(
            !mounted.frame_in_flight(),
            "the stale completion released its DrawableFrame lease"
        );
        assert!(mounted.frame_owed(), "the work is owed on the live epoch");
        assert!(
            !mounted.owner_failed(),
            "a stale completion never marks the newer epoch failed"
        );
        assert_eq!(
            probe.wakes(),
            0,
            "a stale completion never settles the newer epoch's readiness"
        );
        assert!(
            !mounted.context_watch_armed(),
            "a stale completion never installs a watch"
        );
        assert!(
            !mounted.frame_presented(),
            "a stale completion never presents"
        );

        // The current epoch still completes through the same seam.
        assert!(
            mounted.begin_in_flight_frame(),
            "the presenter yields a fresh frame for the live epoch"
        );
        mounted.settle_current_completion();
        assert!(
            mounted.frame_presented(),
            "the live epoch presents and reports readiness"
        );
        assert_eq!(probe.wakes(), 1, "readiness resolves once");
        assert!(!mounted.owner_failed());
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
    use waterui::Identifiable;
    use waterui::component::lazy::Lazy;
    use waterui::prelude::*;
    use waterui::reactive::binding;
    use waterui::reactive::collection::List as ReactiveList;
    use waterui::views::ForEach;
    use waterui_core::layout::ProposalSize;

    use cocoa_ui::objc2_foundation::{NSDate, NSRunLoop};

    use super::{HostView, Label, MainThreadMarker, PlatformView, Retained, leaf, mtm, resolve};

    /// A stable-id row for the `ForEach` membership.
    #[derive(Clone, Copy, Identifiable)]
    struct Row {
        #[id]
        id: u64,
    }

    /// Drains queued main-queue work — binding flushes and enqueued drops
    /// alike land at a real run-loop boundary.
    fn pump() {
        NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.2));
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

    /// Mounts `leaf` on a fresh host whose layout handler frames it —
    /// the handler borrows the child weakly so it never keeps a dead
    /// owner alive, the same edge the production handlers now take.
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
}
