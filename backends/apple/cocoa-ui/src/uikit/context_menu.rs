//! Contextual menus: a [`UIContextMenuInteraction`] on a view, and the
//! accessory window that may float above it while the menu is displayed.
//!
//! [`ContextMenu`] installs the interaction with delegate answers the
//! handler supplies: which menu to show, which preview to lift, and the
//! open/close hooks the accessory presentation hangs off.
//! [`AccessoryOverlay`] is the pass-through window an accessory is
//! presented in, anchored to the lifted preview.
//!
//! # Safety
//!
//! The `unsafe` here defines the delegate class `UIKit` calls and the
//! preview `UIViewController`. `UIKit` holds the delegate weakly, so
//! [`ContextMenu`] owns it for as long as the interaction is installed.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSObjectProtocol, NSString};
use objc2_ui_kit::{
    UIContextMenuConfiguration, UIContextMenuInteraction, UIContextMenuInteractionAnimating,
    UIContextMenuInteractionDelegate, UIInteraction, UIMenu, UIPreviewParameters, UIPreviewTarget,
    UITargetedPreview, UIView, UIViewController, UIWindow,
};

use crate::callback::guarded;
use crate::geometry::{Point, Rect, Size, anchored_frame};
use crate::uikit::host_view::{HitTest, HostView};

/// The air around an accessory, between it and the preview or the window
/// edge.
const GAP: f64 = 8.0;
const EDGE_MARGIN: f64 = 8.0;

/// What [`ContextMenu`] asks its owner when the interaction begins: the
/// menu to show and, optionally, the view controller to lift as preview.
#[derive(Debug)]
pub struct ContextMenuConfiguration {
    /// The menu to present.
    pub menu: Retained<UIMenu>,
    /// The preview to lift, when the menu has a custom one.
    pub preview: Option<Retained<UIViewController>>,
}

/// The configuration a menu interaction asks for.
type ConfigurationHandler = Rc<dyn Fn(&UIView) -> Option<ContextMenuConfiguration>>;
/// The preview a menu interaction asks for.
type PreviewHandler = Rc<dyn Fn(&UIView) -> Option<Retained<UITargetedPreview>>>;

/// The answers a [`ContextMenu`] delegates.
pub struct ContextMenuHandlers {
    /// The configuration for a menu at a point, or `None` to refuse it.
    pub configuration: ConfigurationHandler,
    /// The preview lifted while the menu highlights or dismisses.
    pub preview: PreviewHandler,
    /// Called as the menu begins displaying.
    pub will_display: Rc<dyn Fn(&UIView)>,
    /// Called as the menu ends.
    pub will_end: Rc<dyn Fn(&UIView)>,
}

impl fmt::Debug for ContextMenuHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextMenuHandlers")
            .finish_non_exhaustive()
    }
}

/// The ivars of a [`ContextMenuDelegate`]: the handler set it forwards to.
#[derive(Default)]
pub struct ContextMenuDelegateIvars {
    handlers: RefCell<Option<Rc<ContextMenuHandlers>>>,
}

impl fmt::Debug for ContextMenuDelegateIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextMenuDelegateIvars")
            .field("handlers", &self.handlers.borrow().is_some())
            .finish()
    }
}

define_class!(
    // SAFETY: `NSObject` asks a subclass to initialize through `init`, which
    // `ContextMenuDelegate::new` does, and the class does not implement
    // `Drop`.
    #[unsafe(super(objc2_foundation::NSObject))]
    #[name = "CocoaUiContextMenuDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ContextMenuDelegateIvars]
    #[derive(Debug)]
    /// The `UIContextMenuInteractionDelegate` of a [`ContextMenu`].
    struct ContextMenuDelegate;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for ContextMenuDelegate {}

    // SAFETY: the methods `UIKit` calls through the delegate protocol are
    // implemented with their declared signatures, on the main thread.
    unsafe impl UIContextMenuInteractionDelegate for ContextMenuDelegate {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(contextMenuInteraction:configurationForMenuAtLocation:))]
        fn configuration_for_menu_at_location(
            &self,
            interaction: &UIContextMenuInteraction,
            _location: objc2_core_foundation::CGPoint,
        ) -> Option<Retained<UIContextMenuConfiguration>> {
            guarded("ContextMenuDelegate configurationForMenuAtLocation", || {
                let mtm = MainThreadMarker::from(self);
                let view = interaction.view()?;
                let handlers = self.ivars().handlers.borrow().clone()?;
                let configuration = (handlers.configuration)(&view)?;
                let preview = configuration.preview;
                let menu = configuration.menu;
                let preview_block = RcBlock::new(move || {
                    preview
                        .as_ref()
                        .map_or(core::ptr::null_mut(), |controller| {
                            Retained::autorelease_return(controller.clone())
                        })
                });
                let action_block =
                    RcBlock::new(move |_actions| Retained::autorelease_return(menu.clone()));
                // SAFETY: the configuration retains both blocks, which own
                // the preview and menu values for its life.
                Some(unsafe {
                    UIContextMenuConfiguration::configurationWithIdentifier_previewProvider_actionProvider(
                        None,
                        RcBlock::into_raw(preview_block),
                        RcBlock::into_raw(action_block),
                        mtm,
                    )
                })
            })
        }

        // SAFETY: see the module safety note.
        #[allow(deprecated)]
        #[unsafe(method_id(contextMenuInteraction:previewForHighlightingMenuWithConfiguration:))]
        fn preview_for_highlighting(
            &self,
            interaction: &UIContextMenuInteraction,
            _configuration: &UIContextMenuConfiguration,
        ) -> Option<Retained<UITargetedPreview>> {
            guarded("ContextMenuDelegate previewForHighlighting", || {
                let handlers = self.ivars().handlers.borrow().clone()?;
                let view = interaction.view()?;
                (handlers.preview)(&view)
            })
        }

        // SAFETY: see the module safety note.
        #[allow(deprecated)]
        #[unsafe(method_id(contextMenuInteraction:previewForDismissingMenuWithConfiguration:))]
        fn preview_for_dismissing(
            &self,
            interaction: &UIContextMenuInteraction,
            _configuration: &UIContextMenuConfiguration,
        ) -> Option<Retained<UITargetedPreview>> {
            guarded("ContextMenuDelegate previewForDismissing", || {
                let handlers = self.ivars().handlers.borrow().clone()?;
                let view = interaction.view()?;
                (handlers.preview)(&view)
            })
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(contextMenuInteraction:willDisplayMenuForConfiguration:animator:))]
        fn will_display(
            &self,
            interaction: &UIContextMenuInteraction,
            _configuration: &UIContextMenuConfiguration,
            _animator: Option<&ProtocolObject<dyn UIContextMenuInteractionAnimating>>,
        ) {
            guarded("ContextMenuDelegate willDisplay", || {
                let Some(handlers) = self.ivars().handlers.borrow().clone() else {
                    return;
                };
                if let Some(view) = interaction.view() {
                    (handlers.will_display)(&view);
                }
            });
        }

        // SAFETY: see the module safety note.
        #[unsafe(method(contextMenuInteraction:willEndForConfiguration:animator:))]
        fn will_end(
            &self,
            interaction: &UIContextMenuInteraction,
            _configuration: &UIContextMenuConfiguration,
            _animator: Option<&ProtocolObject<dyn UIContextMenuInteractionAnimating>>,
        ) {
            guarded("ContextMenuDelegate willEnd", || {
                let Some(handlers) = self.ivars().handlers.borrow().clone() else {
                    return;
                };
                if let Some(view) = interaction.view() {
                    (handlers.will_end)(&view);
                }
            });
        }
    }
);

impl ContextMenuDelegate {
    fn new(mtm: MainThreadMarker, handlers: ContextMenuHandlers) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ContextMenuDelegateIvars {
            handlers: RefCell::new(Some(Rc::new(handlers))),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// A [`UIContextMenuInteraction`] installed on a view plus the delegate
/// that answers it.
///
/// `UIKit` holds the delegate weakly; keep this value for as long as the
/// menu can be presented.
#[derive(Debug)]
pub struct ContextMenu {
    interaction: Retained<UIContextMenuInteraction>,
    _delegate: Retained<ContextMenuDelegate>,
}

impl ContextMenu {
    /// Installs a context-menu interaction on `view`, answered by
    /// `handlers`.
    #[must_use]
    pub fn install(view: &UIView, handlers: ContextMenuHandlers) -> Self {
        let mtm = MainThreadMarker::from(view);
        let delegate = ContextMenuDelegate::new(mtm, handlers);
        // SAFETY: `delegate` answers the delegate protocol.
        let interaction = {
            UIContextMenuInteraction::initWithDelegate(
                UIContextMenuInteraction::alloc(mtm),
                ProtocolObject::from_ref(&*delegate),
            )
        };
        view.addInteraction(ProtocolObject::from_ref(&*interaction));
        Self {
            interaction,
            _delegate: delegate,
        }
    }

    /// Dismisses the presented menu, if any.
    pub fn dismiss(&self) {
        self.interaction.dismissMenu();
    }
}

/// The ivars of a [`PreviewViewController`]: the view it presents and the
/// size it prefers.
pub struct PreviewControllerIvars {
    content: RefCell<Option<Retained<UIView>>>,
}

impl fmt::Debug for PreviewControllerIvars {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreviewControllerIvars").finish()
    }
}

impl Default for PreviewControllerIvars {
    fn default() -> Self {
        Self {
            content: RefCell::new(None),
        }
    }
}

define_class!(
    // SAFETY: `UIViewController` asks a subclass to initialize through
    // `initWithNibName:bundle:`, which `PreviewViewController::new` does,
    // and the class does not implement `Drop`.
    #[unsafe(super(UIViewController))]
    #[name = "CocoaUiPreviewViewController"]
    #[thread_kind = MainThreadOnly]
    #[ivars = PreviewControllerIvars]
    #[derive(Debug)]
    /// A view controller whose view is the context menu's preview content,
    /// at the content's ideal size.
    struct PreviewViewController;

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIViewController`
    // subclass.
    unsafe impl NSObjectProtocol for PreviewViewController {}

    impl PreviewViewController {
        // SAFETY: see the module safety note.
        #[unsafe(method(loadView))]
        fn load_view(&self) {
            guarded("PreviewViewController loadView", || {
                let content = self.ivars().content.borrow().clone();
                if let Some(content) = content {
                    self.setView(Some(&content));
                } else {
                    // SAFETY: see the module safety note.
                    let _: () = unsafe { msg_send![super(self), loadView] };
                }
            });
        }
    }
);

/// A preview controller for a context menu's lifted preview: `view` becomes
/// its whole content at `preferred_size`.
#[must_use]
pub fn preview_controller(
    mtm: MainThreadMarker,
    view: Retained<UIView>,
    preferred_size: Size,
) -> Retained<UIViewController> {
    let this = PreviewViewController::alloc(mtm).set_ivars(PreviewControllerIvars {
        content: RefCell::new(Some(view)),
    });
    // SAFETY: `initWithNibName:bundle:` is `UIViewController`'s designated
    // initializer; no nib means the view comes from `loadView`.
    let controller: Retained<PreviewViewController> = unsafe {
        msg_send![super(this), initWithNibName: None::<&NSString>, bundle: None::<&objc2_foundation::NSBundle>]
    };
    controller.setPreferredContentSize(preferred_size.into());
    // A `PreviewViewController` is a `UIViewController`.
    controller.into_super()
}

/// A targeted preview lifting `view`.
#[must_use]
pub fn targeted_preview(view: &UIView) -> Retained<UITargetedPreview> {
    let mtm = MainThreadMarker::from(view);
    // `view` is a live view.
    UITargetedPreview::initWithView(UITargetedPreview::alloc(mtm), view)
}

/// A targeted preview lifting `view`, retargeted to `center` in
/// `container`'s coordinates.
#[must_use]
pub fn targeted_preview_at(
    view: &UIView,
    container: &UIView,
    center: Point,
) -> Retained<UITargetedPreview> {
    let mtm = MainThreadMarker::from(view);
    let parameters = UIPreviewParameters::new(mtm);
    // The target names a live container view and a point in its
    // coordinates.
    let target = {
        UIPreviewTarget::initWithContainer_center(
            UIPreviewTarget::alloc(mtm),
            container,
            center.into(),
        )
    };
    // `view` is a live view and `parameters`/`target` are valid.
    {
        UITargetedPreview::initWithView_parameters_target(
            UITargetedPreview::alloc(mtm),
            view,
            &parameters,
            &target,
        )
    }
}

/// `view`'s bounds in window coordinates.
#[must_use]
pub fn bounds_in_window(view: &UIView) -> Rect {
    view.convertRect_toView(view.bounds(), None).into()
}

/// A window above the context menu's that only the accessory hit-tests:
/// hits on the platter's own surface fall through to the host window, where
/// the menu container reads them as dismiss taps.
#[derive(Debug)]
pub struct AccessoryOverlay {
    window: Retained<UIWindow>,
    _platter: Retained<HostView>,
}

impl AccessoryOverlay {
    /// Presents `accessory` in a window above `source`'s, anchored to
    /// `preview_frame` (in window coordinates). `ideal_size` measures the
    /// accessory on every layout pass, so a view that changes size
    /// re-anchors itself.
    #[must_use]
    pub fn present(
        source: &UIView,
        accessory: &UIView,
        preview_frame: Rect,
        ideal_size: impl Fn() -> Size + 'static,
    ) -> Option<Self> {
        let host_window = source.window()?;
        let scene = host_window.windowScene()?;
        let mtm = MainThreadMarker::from(source);
        let window = UIWindow::initWithWindowScene(UIWindow::alloc(mtm), &scene);
        window.setFrame(host_window.frame());
        window.setWindowLevel(host_window.windowLevel() + 1.0);
        window.setBackgroundColor(Some(&objc2_ui_kit::UIColor::clearColor()));

        let controller = UIViewController::new(mtm);
        window.setRootViewController(Some(&controller));

        let platter = HostView::new(mtm, window.bounds().into());
        platter.set_hit_test_handler(|view, _point| {
            let _ = view;
            HitTest::PassIfSelf
        });
        platter.set_layout_handler({
            let accessory: Retained<UIView> = Retained::from(accessory);
            move |view| {
                accessory.setFrame(
                    anchored_frame(
                        preview_frame,
                        ideal_size(),
                        view.bounds().into(),
                        GAP,
                        EDGE_MARGIN,
                    )
                    .into(),
                );
            }
        });
        crate::view::add_subview(&platter, accessory);
        if let Some(root) = controller.view() {
            root.addSubview(&platter);
        }
        window.setHidden(false);
        Some(Self {
            window,
            _platter: platter,
        })
    }

    /// Takes the overlay off screen.
    pub fn dismiss(&self) {
        self.window.setHidden(true);
    }
}
