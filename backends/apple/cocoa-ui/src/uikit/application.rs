//! The application's entry point, its delegate, and its scene delegate.
//!
//! # Safety
//!
//! The `unsafe` here defines the two delegate classes `UIKit` instantiates by
//! name, initializes them, and reads the application's delegate back. Every
//! delegate method has the signature its protocol declares, and `UIKit` sends
//! them on the main thread. The application delegate is the object `UIKit`
//! created from [`run`]'s class name and keeps for the life of the process.
//!
//! `UIKit` creates both delegates itself, inside `UIApplicationMain`, so the
//! handlers passed to [`run`] cannot reach them as arguments. They cross that
//! call through one main-thread slot, which the application delegate empties
//! when `UIKit` creates it; the scene delegate then reads them from the
//! application delegate.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
};
use objc2_foundation::{NSDictionary, NSObjectProtocol, NSString};
use objc2_ui_kit::{
    UIApplication, UIApplicationDelegate, UIApplicationLaunchOptionsKey, UIMainMenuSystem, UIMenu,
    UIMenuBuilder, UIMenuRoot, UIResponder, UIScene, UISceneConnectionOptions, UISceneDelegate,
    UISceneSession, UIWindow, UIWindowScene, UIWindowSceneDelegate,
};

use super::window::Window;
use crate::callback::guarded;

type LaunchHandler = Box<dyn FnOnce(MainThreadMarker)>;
type SceneHandler = Rc<dyn Fn(&WindowScene) -> Window>;
type MenuHandler = Rc<dyn Fn(&MenuBuilder<'_>)>;

thread_local! {
    /// The handlers [`run`] hands to the application delegate `UIKit`
    /// creates; see the module documentation for why this slot exists.
    static PENDING_HANDLERS: RefCell<Option<ApplicationHandlers>> = const { RefCell::new(None) };
}

/// What the application does at each point of its life, given to [`run`].
#[must_use = "handlers do nothing until they are passed to `uikit::run`"]
pub struct ApplicationHandlers {
    did_finish_launching: Option<LaunchHandler>,
    connect_scene: SceneHandler,
    build_menus: Option<MenuHandler>,
}

impl ApplicationHandlers {
    /// Handlers that answer every window scene the system connects with the
    /// window `connect_scene` builds for it.
    ///
    /// The window is kept for as long as its scene is connected.
    pub fn new(connect_scene: impl Fn(&WindowScene) -> Window + 'static) -> Self {
        Self {
            did_finish_launching: None,
            connect_scene: Rc::new(connect_scene),
            build_menus: None,
        }
    }

    /// Runs `handler` once the application has launched, before any scene is
    /// connected.
    pub fn did_finish_launching(
        mut self,
        handler: impl FnOnce(MainThreadMarker) + 'static,
    ) -> Self {
        self.did_finish_launching = Some(Box::new(handler));
        self
    }

    /// Runs `handler` each time `UIKit` rebuilds the application's menus —
    /// `application:buildMenuWith:` — handing it the builder under edit.
    /// `UIKit` calls it before `did_finish_launching` finishes its first turn,
    /// so the handler must cope with content that only later fills in; a
    /// [`request_main_menu_rebuild`] then asks for another pass.
    pub fn build_menus(mut self, handler: impl Fn(&MenuBuilder<'_>) + 'static) -> Self {
        self.build_menus = Some(Rc::new(handler));
        self
    }
}

/// The `UIMenuBuilder` `build_menus` runs against, restricted to the two
/// operations an application menu bar needs.
pub struct MenuBuilder<'a> {
    builder: &'a ProtocolObject<dyn UIMenuBuilder>,
}

impl fmt::Debug for MenuBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuBuilder").finish_non_exhaustive()
    }
}

impl MenuBuilder<'_> {
    /// Whether a menu identified `identifier` already exists.
    #[must_use]
    pub fn contains(&self, identifier: &str) -> bool {
        self.builder
            .menuForIdentifier(&NSString::from_str(identifier))
            .is_some()
    }

    /// Appends `menu` at the end of the root menu bar.
    pub fn insert_at_root_end(&self, menu: &UIMenu) {
        // SAFETY: `UIMenuRoot` is a constant `NSString` UIKit publishes.
        let root = unsafe { UIMenuRoot };
        self.builder
            .insertChildMenu_atEndOfMenuForIdentifier(menu, root);
    }

    /// Replaces the menu `identifier` names with `menu`.
    pub fn replace(&self, identifier: &str, menu: &UIMenu) {
        self.builder
            .replaceMenuForIdentifier_withMenu(&NSString::from_str(identifier), menu);
    }
}

/// Flags the main menu system for rebuild — the `build_menus` handler
/// fires on the next pass.
pub fn request_main_menu_rebuild(mtm: MainThreadMarker) {
    // SAFETY: the main system exists for the life of the application.
    unsafe { UIMainMenuSystem::mainSystem(mtm) }.setNeedsRebuild();
}

impl fmt::Debug for ApplicationHandlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApplicationHandlers")
            .field("did_finish_launching", &self.did_finish_launching.is_some())
            .finish_non_exhaustive()
    }
}

/// Starts the application and runs its event loop for the rest of the
/// process, calling `handlers` as its events occur.
///
/// # Panics
///
/// If called a second time.
pub fn run(mtm: MainThreadMarker, handlers: ApplicationHandlers) -> ! {
    PENDING_HANDLERS.with(|slot| {
        assert!(
            slot.replace(Some(handlers)).is_none(),
            "uikit::run was called twice"
        );
    });
    let delegate_class = NSString::from_class(AppDelegate::class());
    // `Info.plist` names `SceneDelegate` as the scene delegate class, so
    // `UIKit` instantiates it by name; touching the class registers it.
    SceneDelegate::class();
    UIApplication::main(None, Some(&delegate_class), mtm)
}

/// The scene the system connected for one of the application's windows.
#[derive(Debug, Clone)]
pub struct WindowScene {
    scene: Retained<UIWindowScene>,
}

impl WindowScene {
    /// Proof that the scene handler runs on the main thread.
    #[must_use]
    pub fn main_thread(&self) -> MainThreadMarker {
        self.scene.mtm()
    }

    pub(super) fn native(&self) -> &UIWindowScene {
        &self.scene
    }
}

struct AppDelegateIvars {
    did_finish_launching: Cell<Option<LaunchHandler>>,
    connect_scene: SceneHandler,
    build_menus: Option<MenuHandler>,
}

define_class!(
    // SAFETY: `UIResponder` asks a subclass to initialize through `init`,
    // which the override below does, and the class does not implement `Drop`.
    #[unsafe(super(UIResponder))]
    #[name = "CocoaUiAppDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppDelegateIvars]
    struct AppDelegate;

    impl AppDelegate {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            guarded("AppDelegate init", || {
                let handlers = PENDING_HANDLERS
                    .with(RefCell::take)
                    .expect("UIKit created the application delegate outside `uikit::run`");
                let this = this.set_ivars(AppDelegateIvars {
                    did_finish_launching: Cell::new(handlers.did_finish_launching),
                    connect_scene: handlers.connect_scene,
                    build_menus: handlers.build_menus,
                });
                // SAFETY: `init` is `UIResponder`'s designated initializer.
                unsafe { msg_send![super(this), init] }
            })
        }
    }

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIResponder` subclass.
    unsafe impl NSObjectProtocol for AppDelegate {}

    // SAFETY: see the module safety note.
    unsafe impl UIApplicationDelegate for AppDelegate {
        #[unsafe(method(application:didFinishLaunchingWithOptions:))]
        fn application_did_finish_launching(
            &self,
            _application: &UIApplication,
            _options: Option<&NSDictionary<UIApplicationLaunchOptionsKey, AnyObject>>,
        ) -> bool {
            guarded("application:didFinishLaunchingWithOptions:", || {
                if let Some(handler) = self.ivars().did_finish_launching.take() {
                    handler(self.mtm());
                }
                true
            })
        }
    }

    impl AppDelegate {
        // Registered outside the protocol impl: `application:buildMenuWith:`
        // exists in `UIApplicationDelegate` only on Mac Catalyst, and the
        // debug protocol check rejects methods the generated trait does not
        // list.
        // SAFETY: see the module safety note.
        #[unsafe(method(application:buildMenuWith:))]
        fn application_build_menu(
            &self,
            _application: &UIApplication,
            builder: &ProtocolObject<dyn UIMenuBuilder>,
        ) {
            guarded("application:buildMenuWith:", || {
                if let Some(handler) = &self.ivars().build_menus {
                    handler(&MenuBuilder { builder });
                }
            });
        }
    }
);

impl AppDelegate {
    /// The application delegate `UIKit` created from [`run`].
    fn current(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: see the module safety note.
        let delegate = unsafe { UIApplication::sharedApplication(mtm).delegate() }
            .expect("UIKit reached a scene delegate before creating the application delegate");
        AsRef::<AnyObject>::as_ref(&*delegate)
            .downcast_ref::<Self>()
            .expect("the application delegate must be the one `uikit::run` registered")
            .retain()
    }
}

#[derive(Default)]
struct SceneDelegateIvars {
    window: RefCell<Option<Retained<UIWindow>>>,
}

define_class!(
    // SAFETY: `UIResponder` asks a subclass to initialize through `init`,
    // which the override below does, and the class does not implement `Drop`.
    #[unsafe(super(UIResponder))]
    // The name the application's `Info.plist` gives as the scene delegate
    // class.
    #[name = "SceneDelegate"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SceneDelegateIvars]
    struct SceneDelegate;

    impl SceneDelegate {
        // SAFETY: see the module safety note.
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(SceneDelegateIvars::default());
            // SAFETY: `init` is `UIResponder`'s designated initializer.
            unsafe { msg_send![super(this), init] }
        }
    }

    // SAFETY: `NSObjectProtocol` asks nothing of a `UIResponder` subclass.
    unsafe impl NSObjectProtocol for SceneDelegate {}

    // SAFETY: see the module safety note.
    unsafe impl UISceneDelegate for SceneDelegate {
        #[unsafe(method(scene:willConnectToSession:options:))]
        fn scene_will_connect(
            &self,
            scene: &UIScene,
            _session: &UISceneSession,
            _options: &UISceneConnectionOptions,
        ) {
            guarded("scene:willConnectToSession:options:", || {
                let scene = scene
                    .downcast_ref::<UIWindowScene>()
                    .unwrap_or_else(|| panic!("the application scene is not a window scene: {scene:?}"));
                let connect_scene = Rc::clone(&AppDelegate::current(self.mtm()).ivars().connect_scene);
                let window = connect_scene(&WindowScene {
                    scene: scene.retain(),
                });
                self.ivars().window.replace(Some(window.into_native()));
            });
        }
    }

    // SAFETY: see the module safety note.
    unsafe impl UIWindowSceneDelegate for SceneDelegate {
        #[unsafe(method_id(window))]
        fn window(&self) -> Option<Retained<UIWindow>> {
            self.ivars().window.borrow().clone()
        }

        #[unsafe(method(setWindow:))]
        fn set_window(&self, window: Option<&UIWindow>) {
            self.ivars().window.replace(window.map(Message::retain));
        }
    }
);
