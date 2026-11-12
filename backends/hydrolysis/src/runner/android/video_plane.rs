//! The Android decoder video plane of the GPU video player.
//!
//! `waterui-video-gpu` wraps the picture of a player in an
//! `AndroidVideoSurfaceHost`: a native view the Android backend realizes with
//! a plane the platform's decoder renders into directly. Protected content,
//! A/V tunneling and audio offload need that plane; the player asks the host
//! for its `Surface` on those paths only.
//!
//! The backend's side of the contract has three parts:
//!
//! - the wrapped picture builds as an ordinary child, and the wrapper adds
//!   one platform view at the child's frame ([`VideoPlane`]), placed below
//!   the child in session order like any other platform view;
//! - the Kotlin `VideoSurfaceHostView` mounted for that placement is the Java
//!   host object the player calls (`acquireVideoSurface`,
//!   `deactivateVideoSurface`); the session hands it to the player through
//!   [`AndroidVideoSurfaceBridge::attach_host`] once it exists;
//! - when Android invalidates the plane's `Surface` the session tells the
//!   decoder through [`AndroidVideoSurfaceBridge::surface_destroyed`].
//!
//! The bridge is dropped with the view's node, which is what tells the
//! player its host is gone.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use jni::Env;
use jni::objects::JObject;
use rustc_hash::FxHashMap;
use waterui::Environment;
use waterui_core::metadata::MetadataKey;
use waterui_core::{AnyView, Metadata, Native};
use waterui_video_gpu::{AndroidVideoSurfaceBridge, AndroidVideoSurfaceHost};

use crate::platform_view::{
    PlatformViewPlacement, PlatformViewSink, PlatformViewSource, PlatformViewTable,
};

/// The platform-view kind the Kotlin host registers its decoder-plane factory
/// under. Application kinds are free vocabulary; this one belongs to the
/// backend and equals `VideoSurfaceHostView.KIND`.
const VIDEO_PLANE_KIND: &str = "dev.waterui.hydrolysis.video-surface-host";

/// A player's host unwrapped for layout: the picture is the metadata's
/// content and the bridge waits for the node build, which gives the picture
/// its decoder plane.
///
/// The host is unwrapped when the view is normalized because its wrapped
/// picture is only reachable by consuming it; every layout and build step
/// then sees an ordinary passthrough wrapper.
pub struct VideoPlaneHost {
    bridge: AndroidVideoSurfaceBridge,
}

impl MetadataKey for VideoPlaneHost {}

impl VideoPlaneHost {
    /// The bridge the node build connects its plane to.
    pub(crate) fn into_bridge(self) -> AndroidVideoSurfaceBridge {
        self.bridge
    }
}

/// `host` as a passthrough wrapper over its picture.
pub fn unwrap_host(host: Native<AndroidVideoSurfaceHost>) -> AnyView {
    let (content, bridge) = host.into_inner().into_parts();
    AnyView::new(Metadata {
        content,
        value: VideoPlaneHost { bridge },
    })
}

/// The window's video planes, addressed by placement id from the host's JNI
/// callbacks.
pub struct VideoPlanes {
    sink: PlatformViewSink,
    planes: RefCell<FxHashMap<u64, Weak<AndroidVideoSurfaceBridge>>>,
}

impl VideoPlanes {
    /// An owner placing its planes through `sink`.
    pub(crate) fn new(sink: PlatformViewSink) -> Self {
        Self {
            sink,
            planes: RefCell::default(),
        }
    }

    /// The window's owner, read from the environment a view was built in.
    ///
    /// # Panics
    /// Panics when the window installed none: the session installs it
    /// wherever `video-gpu` is compiled in.
    pub(crate) fn in_environment(env: &Environment) -> Rc<Self> {
        env.get::<Rc<Self>>()
            .expect("the Android session installs the video planes owner")
            .clone()
    }

    /// The Kotlin host object of plane `id` exists: hand it to the player.
    /// A retired plane (its node was dropped first) is ignored.
    pub(crate) fn host_attached(&self, env: &mut Env<'_>, id: u64, host: &JObject<'_>) {
        let Some(bridge) = self.bridge(id) else {
            return;
        };
        // SAFETY: `host` is the Kotlin `VideoSurfaceHostView` this backend
        // ships, which implements the video-surface host contract
        // (`acquireVideoSurface(Z)Landroid/view/Surface;`,
        // `deactivateVideoSurface()V`) in the JVM `env` belongs to.
        if let Err(error) = unsafe { bridge.attach_host(env, host) } {
            fail(&error);
        }
    }

    /// Android invalidated plane `id`'s `Surface`: the decoder must not
    /// render into it any more.
    pub(crate) fn surface_destroyed(&self, id: u64) {
        let Some(bridge) = self.bridge(id) else {
            return;
        };
        if let Err(error) = bridge.surface_destroyed() {
            fail(&error);
        }
    }

    fn bridge(&self, id: u64) -> Option<Rc<AndroidVideoSurfaceBridge>> {
        self.planes.borrow().get(&id).and_then(Weak::upgrade)
    }
}

/// One player's decoder plane: its placement and the bridge the session
/// connects its host through.
pub struct VideoPlane {
    id: u64,
    table: Rc<RefCell<PlatformViewTable>>,
    planes: Rc<VideoPlanes>,
    /// Held for the node's lifetime: dropping it ends the player's host.
    _bridge: Rc<AndroidVideoSurfaceBridge>,
}

impl VideoPlane {
    /// A plane with a fresh placement id, connected to `bridge`.
    pub(crate) fn new(planes: &Rc<VideoPlanes>, bridge: AndroidVideoSurfaceBridge) -> Self {
        let id = planes.sink.next_id();
        let bridge = Rc::new(bridge);
        planes
            .planes
            .borrow_mut()
            .insert(id, Rc::downgrade(&bridge));
        Self {
            id,
            table: Rc::clone(planes.sink.table()),
            planes: Rc::clone(planes),
            _bridge: bridge,
        }
    }

    /// The platform-view table and placement the host mounts the plane's
    /// view through.
    pub(crate) fn placement(&self) -> (Rc<RefCell<PlatformViewTable>>, PlatformViewPlacement) {
        (
            Rc::clone(&self.table),
            PlatformViewPlacement::new(
                self.id,
                PlatformViewSource::Factory {
                    kind: VIDEO_PLANE_KIND.into(),
                },
            ),
        )
    }
}

impl Drop for VideoPlane {
    fn drop(&mut self) {
        self.planes.planes.borrow_mut().remove(&self.id);
    }
}

/// A host call failed: a contract violation panics in a debug build and is
/// logged in a release build, where the player loses its decoder plane.
fn fail(error: &impl std::fmt::Display) {
    debug_assert!(false, "hydrolysis: the video plane failed: {error}");
    tracing::error!(%error, "hydrolysis: the video plane failed");
}
