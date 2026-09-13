//! The `preview` tool: renders a `#[preview]` function or `WaterUI`
//! expression and returns the PNG straight to the model.
//!
//! This tool is served by the `water mcp` front itself — it is never
//! forwarded to the app child, so it answers even while the app's first build
//! is still compiling (the preview build and the app build serialize on
//! Cargo's lock).

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use aither_core::llm::tool::{Tool, ToolResult};
use color_eyre::eyre::{Context as _, Result, bail};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::info;

use crate::preview::request::{
    self, CliHydrolysisPreviewTheme, CliPreviewBackend, CliPreviewPlatform, DEFAULT_FRAME,
    PreviewRequest, PreviewTarget,
};
use crate::preview::{
    HydrolysisPreviewRequest, launch_preview_session, render_preview_with_hydrolysis,
};
use crate::project::read_project_crate_name;

/// Render a `#[preview]` function or `WaterUI` expression to a PNG image.
///
/// Returns the rendered image directly; the PNG is also written under the
/// project's managed `.water` build-cache directory.
///
/// The arguments mirror `water preview`: a function path such as
/// `views::home`, or — with `expr` — an inline expression such as
/// `text("hello")`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PreviewArgs {
    /// Preview target: a `#[preview]` function path (e.g. `views::home`) or,
    /// with `expr`, a `WaterUI` expression returning `impl View`.
    pub target: String,

    /// Treat `target` as a `WaterUI` expression returning `impl View`
    /// (default `false`). Expression targets require the `hydrolysis` backend.
    #[serde(default)]
    pub expr: bool,

    /// Frame size `WIDTHxHEIGHT` (default `375x667`).
    #[serde(default)]
    pub frame: Option<String>,

    /// Rendering backend: `apple`, `android`, or `hydrolysis`. Defaults to the
    /// platform's native backend (`apple` on macOS/iOS, `android` on Android).
    #[serde(default)]
    pub backend: Option<CliPreviewBackend>,

    /// Theme package for the `hydrolysis` backend (`material3`). Required when
    /// `backend` is `hydrolysis`; rejected otherwise.
    #[serde(default)]
    pub theme: Option<CliHydrolysisPreviewTheme>,

    /// Target platform: `ios`, `macos`, or `android`. Defaults to this host's
    /// native preview platform.
    #[serde(default)]
    pub platform: Option<CliPreviewPlatform>,
}

impl PreviewArgs {
    /// Resolves the shared [`PreviewRequest`] — the same construction
    /// `water preview` applies to its clap arguments.
    ///
    /// # Errors
    /// Returns an error for a malformed frame or an unsupported
    /// platform/backend/theme combination.
    pub fn resolve(&self, crate_name: &str) -> Result<PreviewRequest> {
        let frame = self.frame.as_deref().unwrap_or(DEFAULT_FRAME);
        let (width, height) = request::parse_frame(frame)?;
        let platform = request::resolve_preview_platform(self.platform)?;
        let backend = request::resolve_preview_backend(platform, self.backend)?;
        let hydrolysis_theme = request::resolve_hydrolysis_preview_theme(backend, self.theme)?;
        let target = request::resolve_preview_target(crate_name, &self.target, self.expr);
        Ok(PreviewRequest {
            platform,
            backend,
            hydrolysis_theme,
            target,
            width,
            height,
        })
    }
}

/// Collapse `target` to a file-name-safe form: `[A-Za-z0-9_.-]` characters are
/// kept, every other run collapses to a single `_`.
fn sanitize_target_name(target: &str) -> String {
    let mut name = String::with_capacity(target.len());
    for ch in target.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-') {
            name.push(ch);
        } else if !name.ends_with('_') {
            name.push('_');
        }
    }
    name
}

/// The CLI-served `preview` tool.
#[derive(Debug)]
pub struct PreviewTool {
    project_path: PathBuf,
    sccache_path: Option<PathBuf>,
}

impl PreviewTool {
    /// Binds the tool to a project directory.
    #[must_use]
    pub const fn new(project_path: PathBuf, sccache_path: Option<PathBuf>) -> Self {
        Self {
            project_path,
            sccache_path,
        }
    }

    /// Renders the requested preview, writes the PNG under the project's
    /// managed build cache, and returns it as image content.
    async fn render(&self, args: PreviewArgs) -> ToolResult {
        match self.run(&args).await {
            Ok((output_path, bytes)) => {
                info!(path = %output_path.display(), "preview rendered");
                ToolResult::image(bytes, "image/png")
            }
            Err(error) => ToolResult::error(format!("{error:#}")),
        }
    }

    /// The deterministic output path for a request:
    /// `<build-cache container>/mcp/preview/<sanitized target>-<W>x<H>.png`.
    async fn output_path(&self, request: &PreviewRequest) -> Result<PathBuf> {
        let dir = crate::water_dir::build_cache_container_for(&self.project_path)
            .await?
            .join("mcp")
            .join("preview");
        smol::fs::create_dir_all(&dir)
            .await
            .wrap_err_with(|| format!("failed to create {}", dir.display()))?;
        Ok(dir.join(format!(
            "{}-{}x{}.png",
            sanitize_target_name(request.target.display_name()),
            request.width,
            request.height
        )))
    }

    async fn run(&self, args: &PreviewArgs) -> Result<(PathBuf, Vec<u8>)> {
        let crate_name = read_project_crate_name(&self.project_path).await?;
        let request = args.resolve(&crate_name)?;
        request::check_toolchain_for_backend(request.platform, request.backend).await?;
        let output_path = self.output_path(&request).await?;

        match request.backend {
            CliPreviewBackend::Hydrolysis => {
                render_preview_with_hydrolysis(
                    HydrolysisPreviewRequest {
                        project_path: &self.project_path,
                        source: request.target.hydrolysis_source(),
                        theme: request
                            .hydrolysis_theme
                            .expect("resolve guarantees a theme for hydrolysis"),
                        width: request.width,
                        height: request.height,
                        sccache_path: self.sccache_path.clone(),
                    },
                    &output_path,
                    None,
                )
                .await?;
            }
            CliPreviewBackend::Apple | CliPreviewBackend::Android => {
                let PreviewTarget::Function {
                    function_path,
                    symbol,
                } = &request.target
                else {
                    bail!(
                        "Expression preview is currently supported only with the `hydrolysis` backend."
                    );
                };
                self.render_support_app(&request, function_path, symbol, &output_path)
                    .await?;
            }
        }

        let bytes = smol::fs::read(&output_path).await?;
        Ok((output_path, bytes))
    }

    /// The support-app render path shared with `water preview`: launch or
    /// reuse the preview app, build the project dylib, render the symbol, and
    /// write the PNG. The app is detached on success so the next call reuses
    /// it, and shut down on failure so a broken app is never reused.
    async fn render_support_app(
        &self,
        request: &PreviewRequest,
        function_path: &str,
        symbol: &str,
        output_path: &Path,
    ) -> Result<()> {
        let mut session = launch_preview_session(
            &self.project_path,
            request.platform.into(),
            self.sccache_path.clone(),
        )
        .await?;

        let result = async {
            let dylib = session.build_dylib(&self.project_path).await?;
            let png_data = request::render_with_symbol(
                &mut session,
                function_path,
                symbol,
                dylib.id,
                &dylib.path,
                request.width,
                request.height,
            )
            .await?;
            if png_data.is_empty() {
                bail!("Preview returned empty PNG data");
            }
            smol::fs::write(output_path, &png_data).await?;
            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                session.detach();
                Ok(())
            }
            Err(err) => match session.shutdown().await {
                Ok(()) => Err(err),
                Err(shutdown_error) => Err(err.wrap_err(format!(
                    "preview support app shutdown also failed: {shutdown_error}"
                ))),
            },
        }
    }
}

impl Tool for PreviewTool {
    type Arguments = PreviewArgs;
    type Res = ToolResult;

    fn name(&self) -> Cow<'static, str> {
        "preview".into()
    }

    async fn call(&self, args: Self::Arguments) -> aither_core::Result<Self::Res> {
        Ok(self.render(args).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_default_to_water_preview_defaults() {
        let args: PreviewArgs =
            serde_json::from_str(r#"{"target": "views::home"}"#).expect("minimal args parse");
        assert_eq!(args.target, "views::home");
        assert!(!args.expr);
        assert_eq!(args.frame, None);
        assert_eq!(args.backend, None);
        assert_eq!(args.theme, None);
        assert_eq!(args.platform, None);
    }

    #[test]
    fn args_parse_all_fields() {
        let args: PreviewArgs = serde_json::from_str(
            r#"{
                "target": "text(\"hi\")",
                "expr": true,
                "frame": "800x600",
                "backend": "hydrolysis",
                "theme": "material3",
                "platform": "macos"
            }"#,
        )
        .expect("full args parse");
        assert!(args.expr);
        assert_eq!(args.frame.as_deref(), Some("800x600"));
        assert_eq!(args.backend, Some(CliPreviewBackend::Hydrolysis));
        assert_eq!(args.theme, Some(CliHydrolysisPreviewTheme::Material3));
        assert_eq!(args.platform, Some(CliPreviewPlatform::Macos));
    }

    #[test]
    fn sanitize_collapses_unsafe_runs() {
        assert_eq!(sanitize_target_name("views::home"), "views_home");
        assert_eq!(sanitize_target_name("text(\"hello\")"), "text_hello_");
        assert_eq!(sanitize_target_name("a.b-c_d"), "a.b-c_d");
        assert_eq!(sanitize_target_name("**"), "_");
    }

    #[test]
    fn resolves_to_the_same_request_as_water_preview() {
        // `water preview --expr --frame 800x600 --backend hydrolysis --theme
        // material3 --platform macos 'text("hi")'`
        let args: PreviewArgs = serde_json::from_str(
            r#"{
                "target": "text(\"hi\")",
                "expr": true,
                "frame": "800x600",
                "backend": "hydrolysis",
                "theme": "material3",
                "platform": "macos"
            }"#,
        )
        .expect("args parse");
        let request = args.resolve("demo_app").expect("resolve");
        assert_eq!(
            request,
            PreviewRequest {
                platform: CliPreviewPlatform::Macos,
                backend: CliPreviewBackend::Hydrolysis,
                hydrolysis_theme: Some(crate::preview::HydrolysisPreviewTheme::Material3),
                target: PreviewTarget::Expression {
                    expression: "text(\"hi\")".to_string(),
                },
                width: 800.0,
                height: 600.0,
            }
        );
    }
}
