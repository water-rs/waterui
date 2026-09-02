//! Instance-scoped Android video-surface handoff.

use std::sync::Arc;

use async_channel::{Receiver, Sender};
use jni::{
    Env, JavaVM, jni_sig, jni_str,
    objects::{Global, JObject, JValue, JValueOwned},
};
use waterkit_video::{
    AndroidDrmContext, AndroidPlaybackContext, AndroidProtectedSurface, AndroidVideoSurface,
    VideoError,
};
use waterui_core::{AnyView, Environment, Native, NativeView, View};

/// Native wrapper that adds an Android decoder video plane to one GPU surface.
#[doc(hidden)]
pub struct AndroidVideoSurfaceHost {
    content: AnyView,
    bridge: AndroidVideoSurfaceBridge,
}

impl std::fmt::Debug for AndroidVideoSurfaceHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AndroidVideoSurfaceHost")
            .finish_non_exhaustive()
    }
}

impl AndroidVideoSurfaceHost {
    pub(crate) fn new(content: impl View, bridge: AndroidVideoSurfaceBridge) -> Self {
        Self {
            content: AnyView::new(content),
            bridge,
        }
    }

    /// Consumes the native wrapper into its rendered child and surface bridge.
    #[doc(hidden)]
    pub fn into_parts(self) -> (AnyView, AndroidVideoSurfaceBridge) {
        (self.content, self.bridge)
    }
}

impl NativeView for AndroidVideoSurfaceHost {}

impl View for AndroidVideoSurfaceHost {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }
}

/// Android-backend endpoint for one video surface host.
#[doc(hidden)]
pub struct AndroidVideoSurfaceBridge {
    sender: Sender<AndroidVideoSurfacePort>,
    lifecycle_sender: Sender<AndroidVideoSurfaceLifecycle>,
    lifecycle_receiver: Receiver<AndroidVideoSurfaceLifecycle>,
}

impl std::fmt::Debug for AndroidVideoSurfaceBridge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AndroidVideoSurfaceBridge")
            .finish_non_exhaustive()
    }
}

impl AndroidVideoSurfaceBridge {
    /// Retains the Java host and publishes it to this player instance.
    ///
    /// # Safety
    ///
    /// `host` must implement the private WaterUI Android video-surface host
    /// contract and belong to the JVM represented by `env`.
    ///
    /// # Errors
    ///
    /// Returns a platform error if the JVM or global reference cannot be
    /// retained, or when the owning player has already been destroyed.
    #[doc(hidden)]
    pub unsafe fn attach_host(
        &self,
        env: &mut Env<'_>,
        host: &JObject<'_>,
    ) -> Result<(), VideoError> {
        let vm = env
            .get_java_vm()
            .map_err(|error| VideoError::Platform(format!("access Android JavaVM: {error}")))?;
        let host = env.new_global_ref(host).map_err(|error| {
            VideoError::Platform(format!("retain Android video surface host: {error}"))
        })?;
        self.sender
            .try_send(AndroidVideoSurfacePort {
                vm: Arc::new(vm),
                host,
                lifecycle: self.lifecycle_receiver.clone(),
            })
            .map_err(|error| {
                VideoError::Platform(format!(
                    "attach Android video surface host to player: {error}"
                ))
            })
    }

    /// Notifies the decoder thread that Android invalidated the active Surface.
    ///
    /// # Errors
    ///
    /// Returns a platform error when the owning player is already gone.
    #[doc(hidden)]
    pub fn surface_destroyed(&self) -> Result<(), VideoError> {
        self.lifecycle_sender
            .try_send(AndroidVideoSurfaceLifecycle::Destroyed)
            .map_err(|error| {
                VideoError::Platform(format!(
                    "notify Android decoder of Surface destruction: {error}"
                ))
            })
    }
}

pub struct AndroidVideoSurfaceReceiver {
    receiver: Receiver<AndroidVideoSurfacePort>,
}

impl std::fmt::Debug for AndroidVideoSurfaceReceiver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AndroidVideoSurfaceReceiver")
            .finish_non_exhaustive()
    }
}

impl Clone for AndroidVideoSurfaceReceiver {
    fn clone(&self) -> Self {
        Self {
            receiver: self.receiver.clone(),
        }
    }
}

impl AndroidVideoSurfaceReceiver {
    pub(crate) const fn receiver(&self) -> &Receiver<AndroidVideoSurfacePort> {
        &self.receiver
    }
}

pub struct AndroidVideoSurfacePort {
    vm: Arc<JavaVM>,
    host: Global<JObject<'static>>,
    lifecycle: Receiver<AndroidVideoSurfaceLifecycle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndroidVideoSurfaceLifecycle {
    Destroyed,
    HostDisposed,
}

impl std::fmt::Debug for AndroidVideoSurfacePort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AndroidVideoSurfacePort")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct AttachedSurfaceError(VideoError);

impl std::fmt::Display for AttachedSurfaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for AttachedSurfaceError {}

impl From<jni::errors::Error> for AttachedSurfaceError {
    fn from(error: jni::errors::Error) -> Self {
        Self(VideoError::Platform(error.to_string()))
    }
}

impl AndroidVideoSurfacePort {
    pub(crate) fn playback_context(&self) -> Result<AndroidPlaybackContext, VideoError> {
        // SAFETY: `with_env` attaches this port's own `JavaVM` — the application
        // JVM that owns every media object built from this context.
        self.with_env(|env| unsafe { AndroidPlaybackContext::from_jni(env) })
    }

    pub(crate) fn drm_context(&self) -> Result<AndroidDrmContext, VideoError> {
        // SAFETY: as above, `env` belongs to the application JVM that owns the
        // media objects this DRM context is used with.
        self.with_env(|env| unsafe { AndroidDrmContext::from_jni(env) })
    }

    pub(crate) fn acquire_protected(&self) -> Result<AndroidProtectedSurface, VideoError> {
        let surface = self.acquire(true)?;
        // SAFETY: `acquire(true)` asks the host for its secure surface, which the
        // host creates from a `SurfaceView` whose secure flag is set before the
        // window is attached — the invariant Android cannot report back.
        Ok(unsafe { AndroidProtectedSurface::from_video_surface(surface) })
    }

    pub(crate) fn acquire_clear(&self) -> Result<AndroidVideoSurface, VideoError> {
        self.acquire(false)
    }

    fn acquire(&self, secure: bool) -> Result<AndroidVideoSurface, VideoError> {
        self.discard_prior_surface_destruction()?;
        self.with_env(|env| {
            let surface = env
                .call_method(
                    &self.host,
                    jni_str!("acquireVideoSurface"),
                    jni_sig!("(Z)Landroid/view/Surface;"),
                    &[JValue::Bool(secure)],
                )
                .and_then(JValueOwned::l)
                .map_err(|error| {
                    VideoError::Platform(format!("acquire Android video Surface: {error}"))
                })?;
            if surface.is_null() {
                return Err(VideoError::Platform(String::from(
                    "Android video surface host returned a null Surface",
                )));
            }
            // SAFETY: `surface` is the non-null `android.view.Surface` this host
            // just returned from `acquireVideoSurface`, retained for the playback
            // host's lifetime, and `env` is the JVM it came from.
            unsafe { AndroidVideoSurface::from_jni(env, &surface) }
        })
    }

    pub(crate) fn poll_lifecycle(&self) -> Option<AndroidVideoSurfaceLifecycle> {
        match self.lifecycle.try_recv() {
            Ok(event) => Some(event),
            Err(async_channel::TryRecvError::Empty) => None,
            Err(async_channel::TryRecvError::Closed) => {
                Some(AndroidVideoSurfaceLifecycle::HostDisposed)
            }
        }
    }

    pub(crate) async fn wait_for_lifecycle(&self) -> AndroidVideoSurfaceLifecycle {
        self.lifecycle
            .recv()
            .await
            .unwrap_or(AndroidVideoSurfaceLifecycle::HostDisposed)
    }

    pub(crate) fn deactivate(&self) -> Result<(), VideoError> {
        self.with_env(|env| {
            env.call_method(
                &self.host,
                jni_str!("deactivateVideoSurface"),
                jni_sig!("()V"),
                &[],
            )
            .map_err(|error| {
                VideoError::Platform(format!("deactivate Android video Surface: {error}"))
            })?;
            Ok(())
        })
    }

    fn with_env<T>(
        &self,
        operation: impl FnOnce(&mut Env<'_>) -> Result<T, VideoError>,
    ) -> Result<T, VideoError> {
        self.vm
            .attach_current_thread(|env| operation(env).map_err(AttachedSurfaceError))
            .map_err(|error| error.0)
    }

    fn discard_prior_surface_destruction(&self) -> Result<(), VideoError> {
        loop {
            match self.lifecycle.try_recv() {
                Ok(AndroidVideoSurfaceLifecycle::Destroyed) => {}
                Ok(AndroidVideoSurfaceLifecycle::HostDisposed)
                | Err(async_channel::TryRecvError::Closed) => {
                    return Err(VideoError::Platform(String::from(
                        "Android video surface host was disposed before Surface activation",
                    )));
                }
                Err(async_channel::TryRecvError::Empty) => return Ok(()),
            }
        }
    }
}

pub fn video_surface_channel() -> (AndroidVideoSurfaceBridge, AndroidVideoSurfaceReceiver) {
    let (sender, receiver) = async_channel::unbounded();
    let (lifecycle_sender, lifecycle_receiver) = async_channel::unbounded();
    (
        AndroidVideoSurfaceBridge {
            sender,
            lifecycle_sender,
            lifecycle_receiver,
        },
        AndroidVideoSurfaceReceiver { receiver },
    )
}
