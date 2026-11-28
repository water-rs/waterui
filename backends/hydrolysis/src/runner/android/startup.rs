//! GPU initialization owned by the JNI lifecycle handle before mounting.

use std::sync::Arc;

use executor_core::LocalExecutor;
use executor_core::async_task::AsyncTask;
use ndk::native_window::NativeWindow;

use super::gpu::{AndroidGpuContext, DisplayRefresh, GpuError};
use super::host::{
    AndroidSession, HostAppearance, HostBridge, MetricsSnapshot, SessionInit, UiThreadServices,
};
use super::jni::JniError;
use crate::platform::InputEvent;

pub(super) struct Attachment {
    pub window: NativeWindow,
    pub refresh: DisplayRefresh,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

pub(super) struct Startup {
    pub task: AsyncTask<Result<AndroidGpuContext, GpuError>>,
    pub bridge: Arc<HostBridge>,
    pub services: &'static UiThreadServices,
    pub metrics: MetricsSnapshot,
    pub appearance: HostAppearance,
    pub visible: bool,
    pub resumed: bool,
    pub attached: bool,
    pub surface: Option<Attachment>,
    pub demand: (bool, bool),
    events: Vec<(InputEvent, f64)>,
}

impl Startup {
    pub fn new(
        bridge: Arc<HostBridge>,
        services: &'static UiThreadServices,
        metrics: MetricsSnapshot,
        appearance: HostAppearance,
    ) -> Self {
        let wake = Arc::clone(&bridge);
        let task = services.executor.spawn_local(async move {
            let result = AndroidGpuContext::request_async().await;
            wake.request_frame();
            result
        });
        Self {
            task,
            bridge,
            services,
            metrics,
            appearance,
            visible: false,
            resumed: false,
            attached: false,
            surface: None,
            demand: (false, false),
            events: Vec::new(),
        }
    }

    /// Input coordinates belong to the density at arrival, even if a later
    /// configuration update changes the metrics before mounting completes.
    pub fn push_event(&mut self, event: InputEvent) {
        self.events.push((event, self.metrics.density));
    }

    pub fn mount(
        self,
        env: &mut jni::Env,
        gpu: AndroidGpuContext,
    ) -> Result<AndroidSession, JniError> {
        let initial_phase = super::host::android_phase(self.visible, self.resumed);
        let mut session = AndroidSession::create(
            env,
            self.bridge,
            gpu,
            &jni::objects::JObject::null(),
            SessionInit {
                metrics: self.metrics,
                appearance: self.appearance,
                initial_phase,
            },
            self.services,
        )?
        .ok_or_else(|| JniError("hydrolysis android: application mounting failed".to_owned()))?;
        session.set_host_attached(self.attached);
        session.apply_initial_lifecycle(env, self.visible, self.resumed);
        if let Some(surface) = self.surface {
            session
                .surface_attached_with_generation(
                    surface.window,
                    surface.refresh,
                    surface.width,
                    surface.height,
                    surface.generation,
                )
                .map_err(JniError)?;
        }
        session
            .set_frame_demand(self.demand.0, self.demand.1)
            .map_err(JniError)?;
        for (event, density) in self.events {
            session
                .runtime
                .platform
                .push_event_with_density(event, density);
        }
        Ok(*session)
    }
}
