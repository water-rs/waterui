//! The in-process preview entry `water preview` drives — the `AppKit`
//! realization on macOS, and the headless `UIKit` realization on
//! `aarch64-apple-ios-sim`.
//!
//! - **`AppKit` (macOS):** holds the `Prohibited` activation policy before
//!   launch, mounts the view inside a never-ordered capture window, and
//!   captures through the in-process Core Animation render server.
//! - **`UIKit` (iOS Simulator):** runs `UIApplicationMain` headlessly against
//!   an in-process `CAWindowServer` (`SimFramebuffer`) and `FBSWorkspace`
//!   scene host, compositing all scene layers (including `UIMenu` popups,
//!   alerts, sheets, and popovers) into an offscreen `'RGhA'` surface and
//!   delivering scenario pointer/drag/scroll events via `IOHIDEvent`.

#[cfg(ios_sim_preview)]
mod ios_sim;
#[cfg(target_os = "macos")]
mod macos;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use cocoa_ui::bitmap::HdrRaster;
use waterui::AnyView;
use waterui_backend_core::Environment;
use waterui_capture::content_ledger::ContentError;
use waterui_capture::view_renderer::RenderResult;
#[cfg(target_os = "macos")]
use waterui_preview_protocol::run::PreviewColorScheme;
use waterui_preview_protocol::run::{
    Alpha, PngError, PreviewRunConfig, RawFrameError, RawFrameStreamWriter, ScenarioEvent,
    VideoCaptureConfig, write_hdr_png, write_png,
};
use waterui_resources::ResourceContext;

#[cfg(not(feature = "gpu_surface"))]
use crate::capture::CaptureError;
#[cfg(feature = "gpu_surface")]
use crate::capture_image::CaptureFailure as ImageCaptureFailure;

/// The capture failure a preview run reports — `ViewCapture`'s with
/// `gpu_surface`, the bitmap readback's without it.
#[cfg(feature = "gpu_surface")]
type CaptureFailure = ImageCaptureFailure;
/// See the `gpu_surface` alias.
#[cfg(not(feature = "gpu_surface"))]
type CaptureFailure = CaptureError;

/// The iOS Simulator preview binary's `Info.plist`: a bundle identity,
/// `UIDeviceFamily` 1 (iPhone idiom), and `UIApplicationSceneManifest`.
#[cfg(ios_sim_preview)]
#[doc(hidden)]
pub const INFO_PLIST: [u8; include_bytes!("ios_sim_info.plist").len()] =
    *include_bytes!("ios_sim_info.plist");

/// Embeds the preview binary's `Info.plist` in its `__TEXT,__info_plist`
/// section on iOS Simulator (`aarch64-apple-ios-sim`), and expands to nothing
/// elsewhere.
#[macro_export]
macro_rules! preview_info_plist {
    () => {
        #[cfg(all(target_os = "ios", target_abi = "sim"))]
        #[used]
        #[unsafe(link_section = "__TEXT,__info_plist")]
        static WATERUI_PREVIEW_INFO_PLIST: [u8; $crate::preview::INFO_PLIST.len()] =
            $crate::preview::INFO_PLIST;
    };
}

/// Why a preview run failed — the generated `main` propagates it for the
/// process to report and exit non-zero.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PreviewError {
    /// The run mode is not implemented by the Apple preview.
    #[error("apple preview does not support {0} runs")]
    UnsupportedMode(&'static str),
    /// Mounting, presenting or rasterizing the view failed.
    #[error(transparent)]
    Capture(#[from] CaptureFailure),
    /// A view still owed content when the run's content deadline passed, or
    /// its content failed to load.
    #[error(transparent)]
    Content(#[from] ContentError),
    /// Encoding or writing the capture PNG failed.
    #[error(transparent)]
    Write(#[from] PngError),
    /// Recording raw video frames or planning the scenario timeline failed.
    #[error(transparent)]
    RawFrame(#[from] RawFrameError),
    /// The run loop returned without the capture handing its result back.
    #[error("the main run loop returned before the capture settled")]
    RunLoopExited,
    /// The preview was started off the main thread, where no platform view
    /// can exist.
    #[error("the preview must run on the main thread")]
    NotMainThread,
    /// `AppKit` refused the `Prohibited` activation policy the preview must
    /// hold before it finishes launching, so a capture could show a window,
    /// take focus or put an icon in the Dock.
    #[cfg(target_os = "macos")]
    #[error(
        "the preview process must hold the Prohibited activation policy before AppKit finishes launching"
    )]
    ActivationPolicyRefused,
    /// Headless iOS Simulator runtime bootstrap failed before entering `UIApplicationMain`.
    #[cfg(ios_sim_preview)]
    #[error("{0}")]
    SimulatorBootstrap(String),
    /// Reading a presented frame from `SimFramebuffer` failed or timed out.
    #[cfg(ios_sim_preview)]
    #[error("{0}")]
    SimulatorFramebuffer(String),
}

/// What a preview run executes once bring-up succeeds.
pub(super) enum RunTask {
    Image {
        output: PathBuf,
    },
    Scenario {
        output_dir: Option<PathBuf>,
        captures_ms: Vec<u64>,
        events: Vec<ScenarioEvent>,
        video: Option<VideoCaptureConfig>,
    },
}

/// Runs one preview capture and answers its result.
///
/// # Errors
///
/// Returns a [`PreviewError`] if the run mode is unsupported, platform bring-up
/// fails, content fails or times out, rasterization fails, or encoding/writing
/// fails.
pub fn run(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    config: PreviewRunConfig,
) -> Result<(), PreviewError> {
    #[cfg(target_os = "macos")]
    let result = macos::drive(compose, view, resources, config);
    #[cfg(ios_sim_preview)]
    let result = ios_sim::drive(compose, view, resources, config);
    if let Err(error) = &result {
        tracing::error!("preview run failed: {error}");
    }
    result
}

/// Runs one headless HDR preview capture on macOS and returns its unclamped
/// extended linear Display P3 `f32` RGBA raster (`HdrRaster`).
///
/// # Errors
///
/// Same failure modes as [`run`].
#[cfg(target_os = "macos")]
pub fn capture_hdr(
    compose: impl FnOnce(Environment) -> Environment + 'static,
    view: impl FnOnce() -> AnyView + 'static,
    resources: ResourceContext,
    width: u32,
    height: u32,
    color_scheme: Option<PreviewColorScheme>,
    content_deadline: Duration,
) -> Result<HdrRaster, PreviewError> {
    let result = macos::drive_hdr(
        compose,
        view,
        resources,
        width,
        height,
        color_scheme,
        content_deadline,
    );
    if let Err(error) = &result {
        tracing::error!("preview HDR capture failed: {error}");
    }
    result
}

pub(super) fn write_scenario_sdr_sample(
    result: RenderResult,
    png_capture_ms: Option<u64>,
    record_video: bool,
    offset: Duration,
    output_dir: Option<&Path>,
    video_writer: Option<&mut RawFrameStreamWriter>,
) -> Result<(), PreviewError> {
    let (rgba_data, width, height) = result.into_parts();
    match (png_capture_ms, record_video) {
        (Some(capture_ms), true) => {
            if let Some(output_dir) = output_dir {
                write_png(
                    &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
                    width,
                    height,
                    rgba_data.clone(),
                    Alpha::Premultiplied,
                )?;
            }
            if let Some(writer) = video_writer {
                writer.write_frame(width, height, rgba_data, Alpha::Premultiplied, offset)?;
            }
        }
        (Some(capture_ms), false) => {
            if let Some(output_dir) = output_dir {
                write_png(
                    &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
                    width,
                    height,
                    rgba_data,
                    Alpha::Premultiplied,
                )?;
            }
        }
        (None, true) => {
            if let Some(writer) = video_writer {
                writer.write_frame(width, height, rgba_data, Alpha::Premultiplied, offset)?;
            }
        }
        (None, false) => {}
    }
    Ok(())
}

pub(super) fn write_scenario_hdr_sample(
    raster: HdrRaster,
    png_capture_ms: Option<u64>,
    record_video: bool,
    offset: Duration,
    output_dir: Option<&Path>,
    video_writer: Option<&mut RawFrameStreamWriter>,
) -> Result<(), PreviewError> {
    // Write video first from a slice borrow so the PNG writer can take
    // ownership of `raster.pixels` without cloning the frame buffer.
    if record_video && let Some(writer) = video_writer {
        writer.write_hdr_frame_f32(
            raster.width,
            raster.height,
            &raster.pixels,
            Alpha::Premultiplied,
            offset,
        )?;
    }
    if let (Some(capture_ms), Some(output_dir)) = (png_capture_ms, output_dir) {
        write_hdr_png(
            &output_dir.join(format!("frame-{capture_ms:04}ms.png")),
            raster.width,
            raster.height,
            raster.pixels,
            Alpha::Premultiplied,
        )?;
    }
    Ok(())
}
