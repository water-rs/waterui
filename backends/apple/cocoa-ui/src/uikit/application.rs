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
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSDictionary, NSObjectProtocol, NSSet, NSString};
use objc2_ui_kit::{
    UIApplication, UIApplicationDelegate, UIApplicationLaunchOptionsKey, UICommand,
    UIMainMenuSystem, UIMainMenuSystemConfiguration, UIMenu, UIMenuBuilder, UIMenuRoot,
    UIResponder, UIScene, UISceneConnectionOptions, UISceneDelegate, UISceneSession, UIWindow,
    UIWindowScene, UIWindowSceneDelegate,
};

#[cfg(target_abi = "macabi")]
use super::catalyst_tracking::MenuTracking;
use super::key_commands::{KeyCommandRegistry, KeyCommands};
use super::window::Window;
use crate::callback::guarded;
use crate::geometry::Size;

type LaunchHandler = Box<dyn FnOnce(MainThreadMarker)>;
type SceneHandler = Rc<dyn Fn(&WindowScene) -> Window>;
type MenuHandler = Box<dyn Fn(&MenuBuilder<'_>)>;
type SessionHandler = Box<dyn Fn(&str)>;

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
    lifecycle: LifecycleHandlers,
}

/// The handlers for the scene lifecycle, each given the persistent
/// identifier of the scene session the event concerns.
#[derive(Default)]
struct LifecycleHandlers {
    did_disconnect: Option<SessionHandler>,
    did_become_active: Option<SessionHandler>,
    will_resign_active: Option<SessionHandler>,
    will_enter_foreground: Option<SessionHandler>,
    did_enter_background: Option<SessionHandler>,
    did_discard_sessions: Option<SessionHandler>,
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
            lifecycle: LifecycleHandlers::default(),
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

    /// Runs `handler` each time `UIKit` builds the main menu — the menu bar
    /// and the keyboard-shortcut list — handing it the builder under edit.
    /// The handler becomes the main menu system's build handler as the
    /// application finishes launching, before `did_finish_launching` runs,
    /// and `UIKit` builds the menu with it at once; so the handler must
    /// cope with content that only later fills in, and a
    /// [`request_main_menu_rebuild`] then asks for another pass.
    pub fn build_menus(mut self, handler: impl Fn(&MenuBuilder<'_>) + 'static) -> Self {
        self.build_menus = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when the system disconnects a scene, with the
    /// scene's [`WindowScene::session_id`]. The system may reconnect the
    /// session later, through the scene handler given to [`new`](Self::new).
    pub fn scene_did_disconnect(mut self, handler: impl Fn(&str) + 'static) -> Self {
        self.lifecycle.did_disconnect = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when a scene becomes active, with its
    /// [`WindowScene::session_id`].
    pub fn scene_did_become_active(mut self, handler: impl Fn(&str) + 'static) -> Self {
        self.lifecycle.did_become_active = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when a scene is about to become inactive, with its
    /// [`WindowScene::session_id`].
    pub fn scene_will_resign_active(mut self, handler: impl Fn(&str) + 'static) -> Self {
        self.lifecycle.will_resign_active = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when a scene is about to enter the foreground, with
    /// its [`WindowScene::session_id`].
    pub fn scene_will_enter_foreground(mut self, handler: impl Fn(&str) + 'static) -> Self {
        self.lifecycle.will_enter_foreground = Some(Box::new(handler));
        self
    }

    /// Runs `handler` when a scene enters the background, with its
    /// [`WindowScene::session_id`].
    pub fn scene_did_enter_background(mut self, handler: impl Fn(&str) + 'static) -> Self {
        self.lifecycle.did_enter_background = Some(Box::new(handler));
        self
    }

    /// Runs `handler` once for each scene session the user discarded, such
    /// as by closing its window, with the session's persistent identifier.
    /// A discarded session never reconnects.
    pub fn scene_sessions_did_discard(mut self, handler: impl Fn(&str) + 'static) -> Self {
        self.lifecycle.did_discard_sessions = Some(Box::new(handler));
        self
    }
}

/// The `UIMenuBuilder` `build_menus` runs against, restricted to the two
/// operations an application menu bar needs, and the scope the bar's key
/// commands are armed in.
pub struct MenuBuilder<'a> {
    builder: &'a ProtocolObject<dyn UIMenuBuilder>,
    key_commands: &'a KeyCommands,
}

impl fmt::Debug for MenuBuilder<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MenuBuilder").finish_non_exhaustive()
    }
}

impl MenuBuilder<'_> {
    /// The scope this build's menus arm their key commands in. It replaces
    /// the previous build's scope once the build returns, retiring that
    /// bar's commands.
    #[must_use]
    pub const fn key_commands(&self) -> &KeyCommands {
        self.key_commands
    }

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

    /// The persistent identifier of the scene's session, stable across the
    /// scene's disconnections and reconnections.
    #[must_use]
    pub fn session_id(&self) -> String {
        self.scene.session().persistentIdentifier().to_string()
    }

    /// The highest rate, in frames per second, the scene's screen can
    /// display.
    ///
    /// # Panics
    ///
    /// If `UIKit` reports a negative rate.
    #[must_use]
    pub fn max_frames_per_second(&self) -> u32 {
        u32::try_from(self.scene.screen().maximumFramesPerSecond())
            .expect("a screen's maximum refresh rate is not negative")
    }

    /// Sets the title the system shows for the scene in the App Switcher and
    /// Stage Manager.
    pub fn set_title(&self, title: &str) {
        self.scene.setTitle(Some(&NSString::from_str(title)));
    }

    /// Whether the system lets the user resize the scene's window, which is
    /// when [`Self::set_size_restrictions`] has an effect. `UIKit` reports
    /// no restrictions object for a scene that always fills the screen.
    #[must_use]
    pub fn supports_size_restrictions(&self) -> bool {
        self.scene.sizeRestrictions().is_some()
    }

    /// Bounds the size, in points, the user can resize the scene's window
    /// to.
    ///
    /// # Panics
    ///
    /// If the scene has no size restrictions; check
    /// [`Self::supports_size_restrictions`] first.
    pub fn set_size_restrictions(&self, minimum: Size, maximum: Size) {
        let restrictions = self
            .scene
            .sizeRestrictions()
            .expect("the scene's window is not resizable");
        restrictions.setMinimumSize(CGSize::new(minimum.width, minimum.height));
        restrictions.setMaximumSize(CGSize::new(maximum.width, maximum.height));
    }

    pub(super) fn native(&self) -> &UIWindowScene {
        &self.scene
    }
}

struct AppDelegateIvars {
    did_finish_launching: Cell<Option<LaunchHandler>>,
    connect_scene: SceneHandler,
    /// Taken when it becomes the main menu system's build handler.
    build_menus: Cell<Option<MenuHandler>>,
    lifecycle: LifecycleHandlers,
    /// Every armed key command's callback; `cocoaUiMenuCommandFired:`
    /// resolves its sender here.
    key_commands: Rc<KeyCommandRegistry>,
    /// The scope the current menu bar's key commands are armed in.
    menu_bar_commands: RefCell<Option<KeyCommands>>,
    /// The `NSMenu` popup Catalyst is tracking, for the trigger that must
    /// end it.
    #[cfg(target_abi = "macabi")]
    menu_tracking: Rc<MenuTracking>,
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
                    build_menus: Cell::new(handlers.build_menus),
                    lifecycle: handlers.lifecycle,
                    key_commands: Rc::default(),
                    menu_bar_commands: RefCell::new(None),
                    #[cfg(target_abi = "macabi")]
                    menu_tracking: Rc::new(MenuTracking::observe()),
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
                if let Some(handler) = self.ivars().build_menus.take() {
                    install_main_menu_builder(self, handler);
                }
                if let Some(handler) = self.ivars().did_finish_launching.take() {
                    handler(self.mtm());
                }
                true
            })
        }

        #[unsafe(method(application:didDiscardSceneSessions:))]
        fn application_did_discard_scene_sessions(
            &self,
            _application: &UIApplication,
            sessions: &NSSet<UISceneSession>,
        ) {
            guarded("application:didDiscardSceneSessions:", || {
                if let Some(handler) = &self.ivars().lifecycle.did_discard_sessions {
                    for session in sessions {
                        handler(&session.persistentIdentifier().to_string());
                    }
                }
            });
        }
    }

    impl AppDelegate {
        // `UIKeyCommand`s cannot carry a block: `UIKit` sends the command's
        // action untargeted up the responder chain, which ends here, with a
        // copy of the armed command as the sender. Its `propertyList` names
        // the callback in the registry (see `key_commands`).
        // SAFETY: see the module safety note.
        #[unsafe(method(cocoaUiMenuCommandFired:))]
        fn menu_command_fired(&self, sender: &UICommand) {
            guarded("cocoaUiMenuCommandFired:", || {
                self.ivars().key_commands.fire(sender);
            });
        }
    }
);

/// Makes `handler` the main menu system's build handler: `UIKit` builds the
/// main menu through it — at once, and on every later rebuild — instead of
/// asking the responder chain's `buildMenuWithBuilder:`. A build handler
/// serves the main menu system alone, never a context menu's.
fn install_main_menu_builder(delegate: &AppDelegate, handler: MenuHandler) {
    let mtm = delegate.mtm();
    let delegate = delegate.retain();
    let block = RcBlock::new(move |builder: NonNull<ProtocolObject<dyn UIMenuBuilder>>| {
        guarded("main menu build handler", || {
            // SAFETY: `UIKit` hands the handler the builder under edit, alive
            // for the call.
            let builder = unsafe { builder.as_ref() };
            delegate.build_main_menu(&*handler, builder);
        });
    });
    // `setBuildConfiguration:buildHandler:` copies the block, and the copy
    // owns `handler` and a reference to the delegate for the life of the
    // process; `block` is released on return.
    UIMainMenuSystem::sharedSystem(mtm)
        .setBuildConfiguration_buildHandler(&UIMainMenuSystemConfiguration::new(mtm), Some(&block));
}

impl AppDelegate {
    /// Builds the main menu with `handler`, arming the bar's key commands in
    /// a fresh scope that replaces the previous build's.
    fn build_main_menu(
        &self,
        handler: &dyn Fn(&MenuBuilder<'_>),
        builder: &ProtocolObject<dyn UIMenuBuilder>,
    ) {
        let ivars = self.ivars();
        let key_commands = KeyCommands::in_registry(Rc::clone(&ivars.key_commands));
        handler(&MenuBuilder {
            builder,
            key_commands: &key_commands,
        });
        // The bar `UIKit` now holds is the one just built: the previous
        // build's commands retire with their scope.
        ivars.menu_bar_commands.replace(Some(key_commands));
    }

    /// The application delegate `UIKit` created from [`run`].
    fn current(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: see the module safety note.
        let delegate = unsafe { UIApplication::sharedApplication(mtm).delegate() }
            .expect("UIKit has not created the application delegate yet");
        AsRef::<AnyObject>::as_ref(&*delegate)
            .downcast_ref::<Self>()
            .expect("the application delegate must be the one `uikit::run` registered")
            .retain()
    }
}

/// The running application's key-command registry.
///
/// # Panics
///
/// If the application was not started through [`run`].
pub(super) fn key_command_registry(mtm: MainThreadMarker) -> Rc<KeyCommandRegistry> {
    Rc::clone(&AppDelegate::current(mtm).ivars().key_commands)
}

/// The running application's record of the `NSMenu` popup Catalyst is
/// tracking.
///
/// # Panics
///
/// If the application was not started through [`run`].
#[cfg(target_abi = "macabi")]
pub(super) fn menu_tracking(mtm: MainThreadMarker) -> Rc<MenuTracking> {
    Rc::clone(&AppDelegate::current(mtm).ivars().menu_tracking)
}

/// Reports `scene`'s session identifier to the handler `pick` selects from
/// the application's lifecycle handlers, when the application set one.
fn report_scene(
    mtm: MainThreadMarker,
    scene: &UIScene,
    pick: fn(&LifecycleHandlers) -> Option<&SessionHandler>,
) {
    let delegate = AppDelegate::current(mtm);
    if let Some(handler) = pick(&delegate.ivars().lifecycle) {
        handler(&scene.session().persistentIdentifier().to_string());
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

        #[unsafe(method(sceneDidDisconnect:))]
        fn scene_did_disconnect(&self, scene: &UIScene) {
            guarded("sceneDidDisconnect:", || {
                report_scene(self.mtm(), scene, |l| l.did_disconnect.as_ref());
            });
        }

        #[unsafe(method(sceneDidBecomeActive:))]
        fn scene_did_become_active(&self, scene: &UIScene) {
            guarded("sceneDidBecomeActive:", || {
                report_scene(self.mtm(), scene, |l| l.did_become_active.as_ref());
            });
        }

        #[unsafe(method(sceneWillResignActive:))]
        fn scene_will_resign_active(&self, scene: &UIScene) {
            guarded("sceneWillResignActive:", || {
                report_scene(self.mtm(), scene, |l| l.will_resign_active.as_ref());
            });
        }

        #[unsafe(method(sceneWillEnterForeground:))]
        fn scene_will_enter_foreground(&self, scene: &UIScene) {
            guarded("sceneWillEnterForeground:", || {
                report_scene(self.mtm(), scene, |l| l.will_enter_foreground.as_ref());
            });
        }

        #[unsafe(method(sceneDidEnterBackground:))]
        fn scene_did_enter_background(&self, scene: &UIScene) {
            guarded("sceneDidEnterBackground:", || {
                report_scene(self.mtm(), scene, |l| l.did_enter_background.as_ref());
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
