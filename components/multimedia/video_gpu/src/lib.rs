//! Self-drawn GPU realization for `WaterUI` video components.

#[cfg(target_os = "android")]
mod android_video_surface;
mod decoder_worker;
mod latest_channel;
mod runtime_player;

#[cfg(target_os = "android")]
#[doc(hidden)]
pub use android_video_surface::{AndroidVideoSurfaceBridge, AndroidVideoSurfaceHost};

use waterkit_audio::{AudioDevice, AudioOutput, PlayerError};
use waterkit_video::{AnyLicenseServer, LicenseServer, ZenwaveLicenseServer};
use waterui_core::{Binding, Environment};

/// Backend configuration for self-drawn video playback.
#[derive(Debug, Clone)]
pub struct VideoGpuOptions {
    pub(crate) audio_output: AudioOutput,
    pub(crate) skip_silence: Binding<bool>,
    pub(crate) license_server: AnyLicenseServer,
}

impl VideoGpuOptions {
    /// Creates options that follow the platform's default audio route.
    #[must_use]
    pub fn new() -> Self {
        Self {
            audio_output: AudioOutput::system_default(),
            skip_silence: Binding::bool(false),
            license_server: AnyLicenseServer::new(ZenwaveLicenseServer),
        }
    }

    /// Routes video audio through the selected `WaterKit` output.
    #[must_use]
    pub fn audio_output(mut self, output: AudioOutput) -> Self {
        self.audio_output = output;
        self
    }

    /// Binds sustained-silence shortening for decoded PCM playback.
    ///
    /// The audio presentation clock advances across removed samples, so both
    /// progressive and segmented video remain synchronized while silence is skipped.
    #[must_use]
    pub fn skip_silence(mut self, enabled: &Binding<bool>) -> Self {
        self.skip_silence = enabled.clone();
        self
    }

    /// Replaces the platform-CDM challenge transport.
    ///
    /// The default implementation sends bounded requests exclusively through
    /// Zenwave. A custom server can add application authentication or route
    /// challenges through an application-owned service.
    #[must_use]
    pub fn license_server(mut self, server: impl LicenseServer) -> Self {
        self.license_server = AnyLicenseServer::new(server);
        self
    }
}

impl Default for VideoGpuOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Enumerates audio outputs supported by the self-drawn player.
///
/// # Errors
///
/// Returns an error when enumeration is unavailable or fails. On iOS, apps
/// select external routes through the system route picker instead.
#[cfg_attr(
    target_os = "ios",
    expect(
        clippy::missing_const_for_fn,
        reason = "the cross-platform API must keep identical constness where other targets enumerate devices at runtime"
    )
)]
pub fn audio_output_devices() -> Result<Vec<AudioDevice>, PlayerError> {
    AudioDevice::list()
}

/// Installs the self-drawn [`waterui_video::Video`] and
/// [`waterui_video::VideoPlayer`] realization into an environment.
pub fn install(env: &mut Environment) {
    install_with_options(env, VideoGpuOptions::new());
}

/// Installs the self-drawn video realization with backend-specific options.
pub fn install_with_options(env: &mut Environment, options: VideoGpuOptions) {
    runtime_player::install(env, options);
}
