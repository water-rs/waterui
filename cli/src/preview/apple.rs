//! In-process Apple preview for `water preview --platform macos` and
//! `--platform ios`.
//!
//! Builds the managed Apple preview package — the generated binary that
//! calls `waterui_apple::preview::run` — with one cargo invocation in the
//! shared runtime's target directory, then execs it with the run
//! configuration. The render happens inside the generated process: no
//! TCP, no support app. The package is the same for both platforms; only
//! the target differs: `aarch64-apple-darwin` renders the `AppKit`
//! realization, `aarch64-apple-ios-macabi` the `UIKit` one through Mac
//! Catalyst, exec'd on this Mac with no simulator.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod staging;

use askama::Template;
use eyre::{Context as _, Result, bail};

use crate::apple::platform::apple_rust_build;
use crate::artifact_symbols::ArtifactSymbols;
use crate::build::{BuildOptions, BuildProgress};
use crate::platform::{TargetBackend, TargetPlatform};
use crate::preview::hydrolysis::scenario_frame_path;
use crate::preview::run::{self, absolute_output_path, expect_nonempty_output, write_run_config};
use crate::preview::{
    PreviewScenario, PreviewSource, PreviewTargetTemplate, encode_raw_frame_stream_to_mp4,
};
use crate::project::Project;
use crate::project_model::assets;
use cargo_toml::Manifest as CargoManifest;

use waterui_preview_protocol::run::{
    PreviewColorScheme, PreviewRunConfig, PreviewRunMode, VideoCaptureConfig,
};

/// Request for Apple in-process preview rendering.
#[derive(Debug, Clone)]
pub struct ApplePreviewRequest<'a> {
    /// The host the preview spawns on.
    pub host: &'a crate::toolchain::Host,
    /// Path to the project to preview.
    pub project_path: &'a Path,
    /// The Apple target the preview binary builds for:
    /// `TargetPlatform::MacOS` or `TargetPlatform::MacCatalyst`.
    pub platform: TargetPlatform,
    /// Source used to produce the preview view.
    pub source: PreviewSource<'a>,
    /// Preview width in logical pixels.
    pub width: f32,
    /// Preview height in logical pixels.
    pub height: f32,
    /// How long a capture waits for the content its views owe — a decoding
    /// image, a `Suspense` future — before the preview fails naming them.
    pub content_deadline: Duration,
    /// Optional color scheme override (`None` follows the host default).
    pub color_scheme: Option<PreviewColorScheme>,
    /// Display scale factor applied to the captured image.
    pub scale: f32,
    /// Optional sccache binary path.
    pub sccache_path: Option<PathBuf>,
    /// Optional build progress sink.
    pub progress: Option<BuildProgress>,
}

/// Generate `preview_target.rs`, the module wiring the preview binary to
/// the requested target.
///
/// Written through `write_file_if_changed`: a run that repeats a target
/// leaves the binary's own unit alone in the shared target directory.
async fn write_apple_preview_target(project: &Project, source: &PreviewSource<'_>) -> Result<()> {
    let crate_name_ident = project.crate_name().rust_ident();
    let rendered = PreviewTargetTemplate::apple(*source, crate_name_ident.as_str())
        .render()
        .map_err(eyre::Report::new)?;
    crate::project_model::templates::write_file_if_changed(
        &project
            .apple_preview_crate_path()
            .join("src/preview_target.rs"),
        rendered.as_bytes(),
    )
    .await
    .wrap_err("Failed to write the preview target module")
}

/// Stage the project's assets and fonts where the preview binary's
/// `ResourceContext` looks for them, the way the Hydrolysis preview
/// locates them.
async fn stage_apple_preview_resources(
    project: &Project,
    platform: TargetPlatform,
    symbols: &ArtifactSymbols,
) -> Result<()> {
    let resources_dir = project.apple_preview_crate_path().join("resources");
    let manifest = assets::plan_library_resources(project, symbols, false).await?;
    assets::write_library_resources(&manifest, &resources_dir).await?;

    let mut resolved_fonts = {
        let font_manifest = project.apple_crate_path().join("Cargo.toml");
        let font_declarations =
            assets::scan_fonts(project, &font_manifest, &[platform.font_platform()]).await?;
        assets::resolve_fonts(project.host(), font_declarations).await?
    };
    resolved_fonts.extend(assets::cached_project_fonts(project, &manifest).await?);
    if resolved_fonts.is_empty() {
        return Ok(());
    }
    assets::copy_fonts(&resolved_fonts, &resources_dir.join("fonts")).await
}

/// The preview binary links the project's `dev` feature — which must enable
/// `waterui/dynamic_linking` — so the runtime symbols it resolves come from
/// the shared dylib it is staged beside.
///
/// # Errors
/// Returns an error when the project does not declare the feature the preview
/// build requires.
async fn ensure_project_dev_feature_for_preview(project: &Project) -> Result<()> {
    let manifest_path = project.root().join("Cargo.toml");
    let manifest = smol::unblock(move || CargoManifest::from_path(&manifest_path)).await?;
    let Some(dev_features) = manifest.features.get("dev") else {
        bail!(
            "Preview requires `{}/dev` feature. Add `[features] dev = [\"waterui/dynamic_linking\"]` to {}",
            project.crate_name().as_str(),
            project.root().join("Cargo.toml").display()
        );
    };
    if !dev_features
        .iter()
        .any(|feature| feature == "waterui/dynamic_linking")
    {
        bail!(
            "Preview requires `{}/dev` to include `waterui/dynamic_linking`. Update {}",
            project.crate_name().as_str(),
            project.root().join("Cargo.toml").display()
        );
    }
    Ok(())
}

/// The Apple preview binary [`build_preview_with_apple`] built, with its
/// shared runtime and resources staged beside it — ready to exec.
#[derive(Debug)]
pub struct ApplePreviewBinary {
    project: Project,
    executable: PathBuf,
}

/// Options for executing a built [`ApplePreviewBinary`].
#[derive(Debug, Clone)]
pub struct ApplePreviewRenderOptions<'a> {
    /// Preview width in logical pixels.
    pub width: f32,
    /// Preview height in logical pixels.
    pub height: f32,
    /// How long a capture waits for owed content.
    pub content_deadline: Duration,
    /// Optional color-scheme override.
    pub color_scheme: Option<PreviewColorScheme>,
    /// Capture scale factor.
    pub scale: f32,
    /// Destination PNG path for single-image captures.
    pub output_path: &'a Path,
    /// Optional scenario timeline (PNG frames and/or hardware-encoded MP4 video).
    pub scenario: Option<&'a PreviewScenario>,
}

impl ApplePreviewBinary {
    /// The built executable.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Execute the binary in the preview package's directory with the run
    /// configuration it reads from `WATERUI_PREVIEW_RUN_CONFIG`, and check
    /// it wrote the PNG (or scenario frames / hardware-encoded MP4 video).
    ///
    /// # Errors
    ///
    /// Returns an error when the run configuration cannot be written, the
    /// binary fails, an expected output was not produced, or video encoding fails.
    pub async fn render(&self, options: ApplePreviewRenderOptions<'_>) -> Result<()> {
        let ApplePreviewRenderOptions {
            width,
            height,
            content_deadline,
            color_scheme,
            scale,
            output_path,
            scenario,
        } = options;
        let host = self.project.host();
        let crate_dir = self.project.apple_preview_crate_path();
        let video_target = scenario
            .and_then(PreviewScenario::video)
            .map(|video| -> Result<_> {
                let raw_dir = tempfile::tempdir()
                    .wrap_err("Failed to create temp dir for preview video frames")?;
                let raw_path = raw_dir.path().join("frames.wuirf");
                let output = absolute_output_path(host, video.output_path());
                Ok((raw_dir, raw_path, output, video.fps()))
            })
            .transpose()?;
        let mode = scenario.map_or_else(
            || PreviewRunMode::Image {
                output: absolute_output_path(host, output_path),
            },
            |scenario| PreviewRunMode::Scenario {
                output_dir: scenario
                    .output_dir()
                    .map(|dir| absolute_output_path(host, dir)),
                captures_ms: scenario.captures_ms().to_vec(),
                events: scenario.events().to_vec(),
                video: video_target.as_ref().map(|(_, raw_path, _, fps)| {
                    VideoCaptureConfig::new(raw_path.clone(), *fps)
                        .with_duration_ms(scenario.duration_ms())
                }),
            },
        );
        let run_config = PreviewRunConfig::new(width, height, mode, content_deadline)
            .with_optional_color_scheme(color_scheme)
            .with_scale(scale);
        let run_config_path = write_run_config(&crate_dir, &run_config).await?;
        let process_started = Instant::now();
        let output = run::run_preview_binary(
            host,
            &crate_dir,
            &self.executable,
            &run_config_path,
            "Apple preview",
        )
        .await?;
        tracing::info!(
            phase = "process",
            elapsed_ms = process_started.elapsed().as_millis(),
            "Apple preview phase"
        );
        for line in String::from_utf8_lossy(&output.stderr).lines() {
            tracing::info!("{line}");
        }
        match run_config.mode() {
            PreviewRunMode::Image { output } => {
                expect_nonempty_output(output, "the Apple preview binary's PNG").await?;
            }
            PreviewRunMode::Scenario {
                output_dir,
                captures_ms,
                ..
            } => {
                if let Some(output_dir) = output_dir {
                    for capture_ms in captures_ms {
                        let frame_path = scenario_frame_path(output_dir, *capture_ms);
                        expect_nonempty_output(&frame_path, "scenario frame").await?;
                    }
                }
            }
            PreviewRunMode::Semantic { .. } => {
                unreachable!("Apple preview only runs image or scenario modes")
            }
        }
        if let Some((_raw_dir, raw_path, video_output, _)) = video_target {
            let video_output_for_encode = video_output.clone();
            let _ = smol::unblock(move || {
                encode_raw_frame_stream_to_mp4(&raw_path, &video_output_for_encode)
            })
            .await?;
            expect_nonempty_output(&video_output, "preview video").await?;
        }
        Ok(())
    }
}

/// Build the Apple preview binary for `request.platform` and stage what it
/// loads at run time, without running it.
///
/// # Errors
///
/// Returns an error when the project cannot be opened, the preview package
/// cannot be scaffolded or built, or its runtime and resources cannot be
/// staged.
pub async fn build_preview_with_apple(
    request: ApplePreviewRequest<'_>,
) -> Result<ApplePreviewBinary> {
    let project = Project::open_for_preview_build(request.host, request.project_path).await?;
    // The Apple companion is scaffolded too: its manifest owns the shared
    // runtime's feature forwards and the font declarations the resources
    // scan reads, and its build already lives in the shared target
    // directory.
    project.scaffold_apple_companion().await?;
    ensure_project_dev_feature_for_preview(&project).await?;
    project.scaffold_apple_preview_companion().await?;
    write_apple_preview_target(&project, &request.source).await?;

    let triple = request.platform.triple();
    // The preview binary compiles under the configuration `water run`
    // builds the companion with — the default development options for the
    // Apple backend, through the one function both builds derive their cargo
    // invocation from — so the shared runtime it links is the unit `water
    // run` already compiled, not a variant that relinks on every switch.
    let mut options = BuildOptions::default_development(TargetBackend::Apple);
    if let Some(sccache) = request.sccache_path {
        options = options.with_sccache(sccache);
    }
    if let Some(progress) = request.progress {
        options = options.with_progress(progress);
    }
    let build_started = Instant::now();
    let built = apple_rust_build(
        &project,
        project.apple_preview_crate_path(),
        request.platform,
        triple.clone(),
        &options,
    )
    .await?
    // Preview stages the shared runtime and its dependencies beside the executable.
    .with_final_rustc_arg("-Clink-arg=-Wl,-rpath,@loader_path")
    .build_binary(project.apple_preview_crate_name().as_str(), false)
    .await;
    let built = match built {
        Ok(built) => built,
        Err(error) => {
            crate::toolchain_checks::check_apple_preview(request.host, request.platform).await?;
            return Err(error).wrap_err("Failed to build the Apple preview binary");
        }
    };

    tracing::info!(
        phase = "build",
        elapsed_ms = build_started.elapsed().as_millis(),
        "Apple preview phase"
    );
    let staging_started = Instant::now();

    let executable = built.executable()?.to_path_buf();
    futures_util::try_join!(staging::prepare_runtime(&project, &built, &triple), async {
        let symbols = built.app_symbols().await?;
        stage_apple_preview_resources(&project, request.platform, &symbols).await
    },)?;
    tracing::info!(
        phase = "staging",
        elapsed_ms = staging_started.elapsed().as_millis(),
        "Apple preview phase"
    );
    Ok(ApplePreviewBinary {
        project,
        executable,
    })
}

/// Render a preview with the Apple backend in-process: build the binary,
/// then run it.
///
/// # Errors
///
/// Returns an error when the binary cannot be built or staged, the binary
/// fails, or the image was not produced.
pub async fn render_preview_with_apple(
    request: ApplePreviewRequest<'_>,
    output_path: &Path,
    scenario: Option<&PreviewScenario>,
) -> Result<()> {
    let (width, height, content_deadline, color_scheme, scale) = (
        request.width,
        request.height,
        request.content_deadline,
        request.color_scheme,
        request.scale,
    );
    build_preview_with_apple(request)
        .await?
        .render(ApplePreviewRenderOptions {
            width,
            height,
            content_deadline,
            color_scheme,
            scale,
            output_path,
            scenario,
        })
        .await
}
