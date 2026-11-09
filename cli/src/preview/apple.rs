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
use std::time::Duration;

use askama::Template;
use eyre::{Context as _, Result, bail};

use crate::apple::dynamic_runtime;
use crate::apple::platform::apple_rust_build;
use crate::artifact_symbols::ArtifactSymbols;
use crate::build::{BuildOptions, BuildProgress, RustDynamicLibraries};
use crate::platform::{TargetBackend, TargetPlatform};
use crate::preview::run::{self, absolute_output_path, expect_nonempty_output, write_run_config};
use crate::preview::{PreviewSource, PreviewTargetTemplate};
use crate::project::Project;
use crate::project_model::assets;
use cargo_toml::Manifest as CargoManifest;

use waterui_preview_protocol::run::{PreviewRunConfig, PreviewRunMode};

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
    let manifest =
        assets::stage_project_assets_for_apple(project, &resources_dir, symbols, false).await?;

    let mut resolved_fonts = {
        let font_manifest = project.apple_crate_path().join("Cargo.toml");
        let font_declarations =
            assets::scan_fonts(project, &font_manifest, &[platform.font_platform()]).await?;
        assets::resolve_fonts(project.host(), font_declarations).await?
    };
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);
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

impl ApplePreviewBinary {
    /// The built executable.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Execute the binary in the preview package's directory with the run
    /// configuration it reads from `WATERUI_PREVIEW_RUN_CONFIG`, and check
    /// it wrote the PNG to `output_path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the run configuration cannot be written, the
    /// binary fails, or the image was not produced.
    pub async fn render(
        &self,
        width: f32,
        height: f32,
        content_deadline: Duration,
        output_path: &Path,
    ) -> Result<()> {
        let output_path = absolute_output_path(self.project.host(), output_path);
        let crate_dir = self.project.apple_preview_crate_path();
        let run_config = PreviewRunConfig::new(
            width,
            height,
            PreviewRunMode::Image {
                output: output_path.clone(),
            },
            content_deadline,
        );
        let run_config_path = write_run_config(&crate_dir, &run_config).await?;
        run::run_preview_binary(
            self.project.host(),
            &crate_dir,
            &self.executable,
            &run_config_path,
            "Apple preview",
        )
        .await?;
        expect_nonempty_output(&output_path, "the Apple preview binary's PNG").await
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
    let built = apple_rust_build(
        &project,
        project.apple_preview_crate_path(),
        request.platform,
        triple.clone(),
        &options,
    )
    .await?
    .build_binary(project.apple_preview_crate_name().as_str(), false)
    .await
    .wrap_err("Failed to build the Apple preview binary")?;

    // The binary links `libwaterui_dylib` and `libstd` dynamically: stage
    // both beside it — the profile directory is on its rpath — then give
    // the staged runtime copy the canonical install name and retarget the
    // binary's hashed reference to it, the same fix-up the entry-owning
    // binary gets, so only one runtime image loads in the process.
    let libraries = RustDynamicLibraries::resolve(&built, &triple, &project).await?;
    let profile_dir = built.profile_dir.clone();
    libraries.stage(&profile_dir).await?;
    let staged_runtime = libraries.stage_apple_canonical(&profile_dir).await?;
    let executable = built.executable()?.to_path_buf();
    dynamic_runtime::retarget_module(request.host, &executable, &staged_runtime).await?;
    dynamic_runtime::prepare_host_runtime(request.host, &staged_runtime).await?;

    let symbols = built.app_symbols().await?;
    stage_apple_preview_resources(&project, request.platform, &symbols).await?;
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
) -> Result<()> {
    let (width, height, content_deadline) =
        (request.width, request.height, request.content_deadline);
    build_preview_with_apple(request)
        .await?
        .render(width, height, content_deadline, output_path)
        .await
}
