//! Instance-scoped Android video-surface handoff.

use std::sync::Arc;

use async_channel::{Receiver, Sender};
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JObject},
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
        env: &mut JNIEnv<'_>,
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
    host: GlobalRef,
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

impl AndroidVideoSurfacePort {
    pub(crate) fn playback_context(&self) -> Result<AndroidPlaybackContext, VideoError> {
        let mut env = self.vm.attach_current_thread().map_err(|error| {
            VideoError::Platform(format!("attach Android media context thread: {error}"))
        })?;
        unsafe { AndroidPlaybackContext::from_jni(&mut env) }
    }

    pub(crate) fn drm_context(&self) -> Result<AndroidDrmContext, VideoError> {
        let mut env = self
            .vm
            .attach_current_thread()
            .map_err(|error| VideoError::Platform(format!("attach Android DRM thread: {error}")))?;
        unsafe { AndroidDrmContext::from_jni(&mut env) }
    }

    pub(crate) fn acquire_protected(&self) -> Result<AndroidProtectedSurface, VideoError> {
        let surface = self.acquire(true)?;
        Ok(unsafe { AndroidProtectedSurface::from_video_surface(surface) })
    }

    pub(crate) fn acquire_clear(&self) -> Result<AndroidVideoSurface, VideoError> {
        self.acquire(false)
    }

    fn acquire(&self, secure: bool) -> Result<AndroidVideoSurface, VideoError> {
        self.discard_prior_surface_destruction()?;
        let mut env = self.vm.attach_current_thread().map_err(|error| {
            VideoError::Platform(format!("attach Android video surface thread: {error}"))
        })?;
        let surface = env
            .call_method(
                &self.host,
                "acquireVideoSurface",
                "(Z)Landroid/view/Surface;",
                &[jni::objects::JValue::Bool(u8::from(secure))],
            )
            .and_then(jni::objects::JValueGen::l)
            .map_err(|error| {
                VideoError::Platform(format!("acquire Android video Surface: {error}"))
            })?;
        if surface.is_null() {
            return Err(VideoError::Platform(String::from(
                "Android video surface host returned a null Surface",
            )));
        }
        unsafe { AndroidVideoSurface::from_jni(&mut env, &surface) }
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
        let mut env = self.vm.attach_current_thread().map_err(|error| {
            VideoError::Platform(format!("attach Android video surface thread: {error}"))
        })?;
        env.call_method(&self.host, "deactivateVideoSurface", "()V", &[])
            .map_err(|error| {
                VideoError::Platform(format!("deactivate Android video Surface: {error}"))
            })?;
        Ok(())
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
