use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use askama::Template;
use cargo_toml::Manifest as CargoManifest;
use eyre::{Context as _, Result, bail};

use crate::artifact_symbols::ArtifactSymbols;
use crate::build::{BuildOptions, BuildProgress, BuiltTarget};
use crate::hydrolysis::backend::HydrolysisBackend;
use crate::hydrolysis::platform::{
    build_hydrolysis_with_envs_and_features, stage_hydrolysis_shared_runtime,
};
use crate::platform::{TargetBackend, TargetPlatform};
use crate::preview::run::{self, absolute_output_path, expect_nonempty_output, write_run_config};
use crate::preview::{
    PreviewScenario, PreviewSource, PreviewTargetTemplate, PreviewVideoOptions,
    encode_raw_frame_stream_to_mp4,
};
use crate::project::Project;
use crate::project_model::assets;
use waterui_assets_planner::{FontDeclaration, FontSource};

const HYDROLYSIS_PREVIEW_FEATURE: &str = "waterui-preview-mode";
const HYDROLYSIS_PREVIEW_TEST_FEATURE: &str = "waterui-preview-test-mode";

use waterui_preview_protocol::run::{
    DEFAULT_CONTENT_DEADLINE, DEFAULT_PREVIEW_SCALE, PreviewColorScheme, PreviewRunConfig,
    PreviewRunMode, VideoCaptureConfig,
};

/// Theme package selected for Hydrolysis preview rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HydrolysisPreviewTheme {
    /// Material Design 3 package.
    Material3,
    /// Translucent glass theme package (`hydrolysis-glass`).
    Glass,
}

impl HydrolysisPreviewTheme {
    /// The Rust expression producing the `hydrolysis::Style` value the
    /// preview runtimes are constructed with.
    const fn style(self) -> &'static str {
        match self {
            Self::Material3 => "{ let _ = color_scheme; hydrolysis_m3::Material3::defaults() }",
            Self::Glass => {
                "hydrolysis_glass::Glass::for_scheme(color_scheme.unwrap_or(waterui::theme::ColorScheme::Light))"
            }
        }
    }

    /// Cargo features passed to the generated Hydrolysis launcher crate.
    pub(crate) fn preview_features(self, mode_feature: &'static str) -> Vec<&'static str> {
        match self {
            Self::Material3 => vec![mode_feature],
            Self::Glass => vec![mode_feature, "waterui-preview-glass"],
        }
    }

    fn font_declarations(self) -> Vec<FontDeclaration> {
        match self {
            Self::Material3 => vec![FontDeclaration::new(
                "Roboto".to_string(),
                FontSource::BuiltIn,
                "hydrolysis-m3".to_string(),
                None,
            )],
            Self::Glass => Vec::new(),
        }
    }
}

/// Common inputs for driving the managed Hydrolysis preview backend.
#[derive(Debug, Clone)]
pub struct HydrolysisPreviewRequest<'a> {
    /// The host the preview spawns on.
    pub host: &'a crate::toolchain::Host,
    /// `WaterUI` project directory.
    pub project_path: &'a Path,
    /// Preview view source.
    pub source: PreviewSource<'a>,
    /// Theme package the preview runtimes are constructed with.
    pub theme: HydrolysisPreviewTheme,
    /// Platform the preview renders for — the desktop target `water run`
    /// compiles the managed backend for on this host, or
    /// `TargetPlatform::Android` for the device-side instrumentation render.
    pub platform: TargetPlatform,
    /// Viewport width in logical units.
    pub width: f32,
    /// Viewport height in logical units.
    pub height: f32,
    /// How long a capture waits for the content its views owe — a decoding
    /// image, a `Suspense` future — before the preview fails naming them.
    pub content_deadline: Duration,
    /// Optional color scheme override (`None` follows the host default).
    pub color_scheme: Option<PreviewColorScheme>,
    /// Display scale factor applied to the captured image.
    pub scale: f32,
    /// `sccache` binary used for compilation caching, when available.
    pub sccache_path: Option<PathBuf>,
    /// Sink compile progress is reported to while the preview build runs cargo.
    pub progress: Option<BuildProgress>,
}

/// Render a preview via the managed Hydrolysis backend.
///
/// A desktop `platform` builds and execs the backend binary; `Android`
/// renders inside the preview host's instrumentation on a device — the
/// request shape is identical, the dispatch lives here so every caller
/// resolves one way. `kotlin_toolchain` is the toolchain
/// [`crate::preview::request::check_toolchain_for_backend`] resolved, and
/// `android` the device the caller's selection resolved — both required
/// for `TargetPlatform::Android`, ignored elsewhere.
///
/// # Errors
/// Returns an error if the managed backend cannot be prepared, built, or executed.
pub async fn render_preview_with_hydrolysis(
    request: HydrolysisPreviewRequest<'_>,
    output_path: &Path,
    scenario: Option<&PreviewScenario>,
    kotlin_toolchain: Option<&crate::android::KotlinToolchain>,
    android: Option<crate::android::device::AndroidTarget>,
) -> Result<()> {
    if request.platform == TargetPlatform::Android {
        let kotlin = kotlin_toolchain.ok_or_else(|| {
            eyre::eyre!("Internal error: Android preview has no resolved Kotlin toolchain")
        })?;
        let target = android
            .ok_or_else(|| eyre::eyre!("Internal error: Android preview has no resolved device"))?;
        return Box::pin(
            crate::preview::hydrolysis_android::render_preview_with_hydrolysis_android(
                &request,
                output_path,
                scenario,
                kotlin,
                &target,
            ),
        )
        .await;
    }
    let theme = request.theme;
    let (project, built) = build_preview_session(&request, None, false).await?;
    stage_hydrolysis_resources(
        &project,
        request.platform,
        theme,
        &built.app_symbols().await?,
    )
    .await?;
    stage_hydrolysis_shared_runtime(&project, &built, request.platform).await?;
    run_preview_binary(
        &project,
        built.executable()?,
        &request,
        output_path,
        scenario,
    )
    .await
}

/// Run a semantic preview test session via the managed Hydrolysis backend binary.
///
/// # Errors
/// Returns an error if the managed backend cannot be prepared, built, or executed.
pub async fn test_preview_with_hydrolysis(
    request: HydrolysisPreviewRequest<'_>,
    automation_body: &str,
    video: Option<&PreviewVideoOptions>,
) -> Result<String> {
    let theme = request.theme;
    let (project, built) =
        build_preview_session(&request, Some(automation_body), video.is_some()).await?;
    stage_hydrolysis_resources(
        &project,
        request.platform,
        theme,
        &built.app_symbols().await?,
    )
    .await?;
    stage_hydrolysis_shared_runtime(&project, &built, request.platform).await?;
    run_preview_test_binary(&project, built.executable()?, &request, video).await
}

/// Ensures the generated Hydrolysis backend declares `waterui-preview-glass`
/// before previewing with [`HydrolysisPreviewTheme::Glass`], allowing projects
/// that depend on `hydrolysis-glass` locally while failing fast otherwise on
/// channels or older frameworks that do not distribute it.
pub async fn ensure_glass_theme_available(project: &Project) -> Result<()> {
    let backend_manifest = project
        .backend_path::<HydrolysisBackend>()
        .join("Cargo.toml");
    let has_glass_feature = fs::read(&backend_manifest)
        .ok()
        .and_then(|bytes| CargoManifest::from_slice(&bytes).ok())
        .is_some_and(|manifest| manifest.features.contains_key("waterui-preview-glass"));
    if has_glass_feature {
        return Ok(());
    }

    let framework = project.resolved_framework().await?;
    framework.require_distributable("hydrolysis-glass")?;
    if !framework.has_scaffold("hydrolysis-glass") {
        bail!(
            "the selected framework does not declare `hydrolysis-glass` scaffold metadata; \
             use `--channel dev`, `--channel nightly`, or a local WaterUI checkout"
        );
    }
    bail!("the generated Hydrolysis backend manifest does not declare `waterui-preview-glass`")
}

/// Build the managed hydrolysis binary for a preview request: write the
/// generated bindings, compile in preview mode — or preview-test mode when
/// `automation_body` is `Some` — and return the build result. The app
/// library artifact on it carries the `waterui_meta_*`/`waterui_preview_*`
/// symbols any post-build step (resource staging, `--all` discovery) reads.
async fn build_preview_session(
    request: &HydrolysisPreviewRequest<'_>,
    automation_body: Option<&str>,
    offscreen_semantic_test: bool,
) -> Result<(Project, BuiltTarget)> {
    let project =
        crate::hydrolysis::backend::open_ready(request.host, request.project_path).await?;
    if request.theme == HydrolysisPreviewTheme::Glass {
        ensure_glass_theme_available(&project).await?;
    }
    write_preview_bindings(
        &project,
        request.source,
        request.theme,
        automation_body,
        offscreen_semantic_test,
    )
    .await?;

    let mut build_options = BuildOptions::default_development(TargetBackend::Hydrolysis);
    if let Some(sccache_path) = request.sccache_path.clone() {
        build_options = build_options.with_sccache(sccache_path);
    }
    if let Some(progress) = request.progress.clone() {
        build_options = build_options.with_progress(progress);
    }
    let feature = if automation_body.is_some() {
        HYDROLYSIS_PREVIEW_TEST_FEATURE
    } else {
        HYDROLYSIS_PREVIEW_FEATURE
    };
    let features = request.theme.preview_features(feature);
    let built = build_hydrolysis_with_envs_and_features(
        &project,
        request.platform,
        build_options,
        &[],
        &features,
        true,
    )
    .await?;
    Ok((project, built))
}

/// Enumerate the `waterui_preview_*` exports the project crate carries.
///
/// The names are read from the library artifact of a preview-mode build of
/// the project's managed backend — `preview test --all` discovers its
/// targets this way, and the probe compiles the same crate graph the
/// per-target builds then reuse warm.
///
/// # Errors
/// Returns an error when the preview build fails or its artifact cannot be
/// parsed.
pub async fn discover_hydrolysis_preview_exports(
    host: &crate::toolchain::Host,
    project_path: &Path,
    theme: HydrolysisPreviewTheme,
    platform: TargetPlatform,
    sccache_path: Option<PathBuf>,
    progress: Option<BuildProgress>,
) -> Result<Vec<String>> {
    let request = HydrolysisPreviewRequest {
        host,
        project_path,
        source: PreviewSource::Expression("text(\"\")"),
        theme,
        platform,
        width: 0.0,
        height: 0.0,
        content_deadline: DEFAULT_CONTENT_DEADLINE,
        color_scheme: None,
        scale: DEFAULT_PREVIEW_SCALE,
        sccache_path,
        progress,
    };
    let (_project, built) = build_preview_session(&request, None, false).await?;
    Ok(built
        .app_symbols()
        .await?
        .leaves_with_prefix("waterui_preview_"))
}

/// Stages the project's assets and the selected theme's fonts into the
/// generated backend's `resources/` directory. Shared by the preview and MCP
/// runtime modes. `symbols` is the app library artifact the build just
/// produced — its `waterui_meta_bundle_*` statics declare the asset mounts.
pub async fn stage_hydrolysis_resources(
    project: &Project,
    platform: TargetPlatform,
    theme: HydrolysisPreviewTheme,
    symbols: &ArtifactSymbols,
) -> Result<PathBuf> {
    let resources_dir = project
        .backend_path::<HydrolysisBackend>()
        .join("resources");
    let manifest =
        assets::stage_project_assets_for_gtk(project, &resources_dir, symbols, false).await?;

    let backend_path = project.backend_path::<HydrolysisBackend>();
    let mut font_declarations = assets::scan_fonts(
        project,
        &backend_path.join("Cargo.toml"),
        &[platform.font_platform()],
    )
    .await?;
    font_declarations.extend(theme.font_declarations());
    let mut resolved_fonts = assets::resolve_fonts(project.host(), font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);
    if !resolved_fonts.is_empty() {
        let fonts_dest = resources_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
    }

    // The staged `waterui_assets` root — what `WATERUI_ASSETS_ROOT` must name
    // for a launcher that runs the binary somewhere `resources/` does not sit
    // beside the executable (the MCP child runs out of the shared Cargo
    // profile directory).
    Ok(resources_dir.join(assets::ASSET_ROOT_DIR))
}

pub async fn write_preview_bindings(
    project: &Project,
    source: PreviewSource<'_>,
    theme: HydrolysisPreviewTheme,
    automation_body: Option<&str>,
    offscreen_semantic_test: bool,
) -> Result<()> {
    let file_name = if automation_body.is_some() {
        "preview_test.rs"
    } else {
        "preview_symbol.rs"
    };
    let module_path = project
        .backend_path::<HydrolysisBackend>()
        .join("src")
        .join(file_name);
    let crate_name_ident = project.crate_name().rust_ident();
    let rendered = PreviewTargetTemplate::hydrolysis(
        source,
        crate_name_ident.as_str(),
        theme.style(),
        automation_body,
        offscreen_semantic_test,
    )
    .render()
    .wrap_err("Failed to render hydrolysis preview bindings template")?;
    crate::templates::write_file_if_changed(&module_path, rendered.as_bytes())
        .await
        .wrap_err_with(|| format!("Failed to write {}", module_path.display()))?;
    Ok(())
}

async fn run_preview_binary(
    project: &Project,
    binary_path: &Path,
    request: &HydrolysisPreviewRequest<'_>,
    output_path: &Path,
    scenario: Option<&PreviewScenario>,
) -> Result<()> {
    let host = project.host();
    let video_target = scenario
        .and_then(PreviewScenario::video)
        .map(|video| -> Result<_> {
            let temp_dir = tempfile::tempdir()
                .wrap_err("Failed to create temp dir for preview video frames")?;
            let raw_path = temp_dir.path().join("frames.wuirf");
            let output = absolute_output_path(host, video.output_path());
            Ok((temp_dir, raw_path, output, video.fps()))
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
    let config = PreviewRunConfig::new(
        request.width,
        request.height,
        mode,
        request.content_deadline,
    )
    .with_optional_color_scheme(request.color_scheme)
    .with_scale(request.scale);
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let config_path = write_run_config(&backend_path, &config).await?;
    run::run_preview_binary(
        host,
        &backend_path,
        binary_path,
        &config_path,
        "Hydrolysis preview",
    )
    .await?;

    match config.mode() {
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
        PreviewRunMode::Image { output } => {
            expect_nonempty_output(output, "output").await?;
        }
        PreviewRunMode::Semantic { .. } => {
            unreachable!("render runs only produce images or scenarios")
        }
    }

    if let Some((_temp_dir, raw_path, video_output, _)) = video_target {
        let video_output_for_encode = video_output.clone();
        let _ = smol::unblock(move || {
            encode_raw_frame_stream_to_mp4(&raw_path, &video_output_for_encode)
        })
        .await?;
        expect_nonempty_output(&video_output, "preview video").await?;
    }

    Ok(())
}

async fn run_preview_test_binary(
    project: &Project,
    binary_path: &Path,
    request: &HydrolysisPreviewRequest<'_>,
    video: Option<&PreviewVideoOptions>,
) -> Result<String> {
    let host = project.host();
    let video_target = video
        .map(|video| -> Result<_> {
            let temp_dir =
                tempfile::tempdir().wrap_err("Failed to create temp dir for preview test video")?;
            let raw_path = temp_dir.path().join("frames.wuirf");
            let output = absolute_output_path(host, video.output_path());
            Ok((temp_dir, raw_path, output, video.fps()))
        })
        .transpose()?;
    let config = PreviewRunConfig::new(
        request.width,
        request.height,
        PreviewRunMode::Semantic {
            video: video_target
                .as_ref()
                .map(|(_, raw_path, _, fps)| VideoCaptureConfig::new(raw_path.clone(), *fps)),
        },
        request.content_deadline,
    )
    .with_optional_color_scheme(request.color_scheme)
    .with_scale(request.scale);
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let config_path = write_run_config(&backend_path, &config).await?;
    let output = run::run_preview_binary(
        host,
        &backend_path,
        binary_path,
        &config_path,
        "Hydrolysis preview test",
    )
    .await?;

    if let Some((_temp_dir, raw_path, video_output, _)) = video_target {
        let video_output_for_encode = video_output.clone();
        let _ = smol::unblock(move || {
            encode_raw_frame_stream_to_mp4(&raw_path, &video_output_for_encode)
        })
        .await?;
        expect_nonempty_output(&video_output, "preview test video").await?;
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn scenario_frame_path(output_dir: &Path, capture_ms: u64) -> PathBuf {
    output_dir.join(format!("frame-{capture_ms:04}ms.png"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::ensure_glass_theme_available;
    use crate::hydrolysis::backend::HydrolysisBackend;
    use crate::project::{ManagedBackends, Project};
    use crate::toolchain::Host;

    #[test]
    fn glass_theme_available_when_generated_manifest_declares_feature() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path();
        fs::create_dir_all(root.join("src")).expect("create src dir");
        fs::write(root.join("src/lib.rs"), "").expect("write lib.rs");
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("write Cargo.toml");
        fs::write(
            root.join("Water.toml"),
            "[package]\nname = \"Demo\"\nbundle_identifier = \"dev.example.demo\"\n",
        )
        .expect("write Water.toml");

        let host = Host::current();
        smol::block_on(async {
            let project = Project::open(&host, root, ManagedBackends::NONE)
                .await
                .expect("open project");
            let backend_manifest = project
                .backend_path::<HydrolysisBackend>()
                .join("Cargo.toml");
            fs::create_dir_all(backend_manifest.parent().expect("backend dir"))
                .expect("create backend dir");
            fs::write(
                &backend_manifest,
                "[package]\nname = \"demo-hydrolysis\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[features]\nwaterui-preview-glass = [\"dep:hydrolysis-glass\"]\n\n[dependencies]\nhydrolysis-glass = { path = \"..\", optional = true }\n",
            )
            .expect("write backend Cargo.toml");

            ensure_glass_theme_available(&project)
                .await
                .expect("local glass feature must allow glass preview");

            fs::write(
                &backend_manifest,
                "[package]\nname = \"demo-hydrolysis\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            )
            .expect("write backend Cargo.toml without glass");
            assert!(
                ensure_glass_theme_available(&project).await.is_err(),
                "missing waterui-preview-glass without scaffolded framework must fail fast"
            );
        });
    }
}
