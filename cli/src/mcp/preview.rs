//! The `preview` tool: renders a `#[preview]` function or `WaterUI`
//! expression and returns the PNG straight to the model.
//!
//! This tool is served by the `water mcp` front itself — it is never
//! forwarded to the app child, so it answers even while the app's first build
//! is still compiling (the preview build and the app build serialize on
//! Cargo's lock).

use std::borrow::Cow;
use std::path::PathBuf;

use aither_core::llm::tool::{Tool, ToolContext, ToolResult};
use eyre::{Context as _, Result, bail};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::info;

use crate::preview::request::{
    self, CliHydrolysisPreviewTheme, CliPreviewBackend, CliPreviewPlatform, DEFAULT_FRAME,
    PreviewRequest, ResolvedPreviewBackend,
};
use crate::preview::{
    ApplePreviewRequest, HydrolysisPreviewRequest, render_preview_with_apple,
    render_preview_with_hydrolysis,
};
use crate::project::{Manifest, read_project_crate_name};

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
    /// (default `false`). Expression targets require the `hydrolysis`
    /// backend or, on macOS and iOS, the default `apple` backend.
    #[serde(default)]
    pub expr: bool,

    /// Frame size `WIDTHxHEIGHT` (default `375x667`).
    #[serde(default)]
    pub frame: Option<String>,

    /// Rendering backend: `apple` or `hydrolysis`. Uses the project's
    /// platform declaration, then the platform default. A conflicting
    /// override or a backend without preview support is an error.
    #[serde(default)]
    pub backend: Option<CliPreviewBackend>,

    /// Theme package for the `hydrolysis` backend (`material3`, the default);
    /// rejected for other backends.
    #[serde(default)]
    pub theme: Option<CliHydrolysisPreviewTheme>,

    /// Target platform: `ios`, `macos`, `android`, `linux`, or `windows`.
    /// Defaults to this host's native preview platform.
    #[serde(default)]
    pub platform: Option<CliPreviewPlatform>,

    /// Android device serial or AVD name — only with `platform: "android"`.
    /// Without it the remembered last-used device wins, then an unambiguous
    /// single candidate; several candidates are an error naming every one.
    #[serde(default)]
    pub device: Option<String>,

    /// Seconds the capture waits for content its views still owe — a
    /// decoding image, a `Suspense` future — before the preview fails
    /// naming them (default `10`).
    #[serde(default)]
    pub content_timeout: Option<f64>,
}

impl PreviewArgs {
    /// Resolves the shared [`PreviewRequest`] — the same construction
    /// `water preview` applies to its clap arguments.
    ///
    /// # Errors
    /// Returns an error for a malformed frame or content timeout, or an
    /// unsupported platform/backend/theme combination.
    pub fn resolve(&self, project: &Manifest, crate_name: &str) -> Result<PreviewRequest> {
        let frame = self.frame.as_deref().unwrap_or(DEFAULT_FRAME);
        let (width, height) = request::parse_frame(frame)?;
        let content_deadline = request::parse_content_timeout(self.content_timeout)?;
        let platform = request::resolve_preview_platform(self.platform)?;
        let backend = request::resolve_preview_backend(project, platform, self.backend)?;
        let hydrolysis_theme = request::resolve_hydrolysis_preview_theme(backend, self.theme)?;
        let target = request::resolve_preview_target(crate_name, &self.target, self.expr);
        Ok(PreviewRequest::new(
            platform,
            backend,
            hydrolysis_theme,
            target,
            width,
            height,
            content_deadline,
        ))
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
    host: crate::toolchain::Host,
    project_path: PathBuf,
    sccache_path: Option<PathBuf>,
}

impl PreviewTool {
    /// Binds the tool to a project directory.
    #[must_use]
    pub const fn new(
        host: crate::toolchain::Host,
        project_path: PathBuf,
        sccache_path: Option<PathBuf>,
    ) -> Self {
        Self {
            host,
            project_path,
            sccache_path,
        }
    }

    /// Renders the requested preview, writes the PNG under the project's
    /// managed build cache, and returns it as image content.
    async fn render(&self, args: PreviewArgs) -> ToolResult {
        match Box::pin(self.run(&args)).await {
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
        let dir = crate::water_dir::build_cache_container_for(&self.host, &self.project_path)
            .await?
            .join("mcp")
            .join("preview");
        smol::fs::create_dir_all(&dir)
            .await
            .wrap_err_with(|| format!("failed to create {}", dir.display()))?;
        Ok(dir.join(format!(
            "{}-{}x{}.png",
            sanitize_target_name(request.target().display_name()),
            request.width(),
            request.height()
        )))
    }

    async fn run(&self, args: &PreviewArgs) -> Result<(PathBuf, Vec<u8>)> {
        let manifest = Manifest::open(self.project_path.join("Water.toml")).await?;
        let crate_name = read_project_crate_name(&self.project_path).await?;
        let request = args.resolve(&manifest, &crate_name)?;
        let kotlin_toolchain =
            request::check_toolchain_for_backend(&self.host, request.backend()).await?;
        let output_path = self.output_path(&request).await?;

        let android = if request.backend()
            == ResolvedPreviewBackend::Hydrolysis(crate::platform::TargetPlatform::Android)
        {
            Some(
                crate::android::device::AndroidTarget::resolve(
                    &self.host,
                    args.device.as_deref(),
                    crate::water_dir::device_memory_key(
                        crate::platform::TargetBackend::Hydrolysis,
                        crate::platform::TargetPlatform::Android,
                    ),
                )
                .await?,
            )
        } else {
            if args.device.is_some() {
                bail!("`device` applies only with `platform: \"android\"`.");
            }
            None
        };

        match request.backend() {
            ResolvedPreviewBackend::Hydrolysis(platform) => {
                Box::pin(render_preview_with_hydrolysis(
                    HydrolysisPreviewRequest {
                        host: &self.host,
                        project_path: &self.project_path,
                        source: request.target().source(),
                        theme: request
                            .hydrolysis_theme()
                            .expect("resolve guarantees a theme for hydrolysis"),
                        platform,
                        width: request.width(),
                        height: request.height(),
                        content_deadline: request.content_deadline(),
                        sccache_path: self.sccache_path.clone(),
                        // MCP serves JSON-RPC over stdio — there is no
                        // terminal sink to render compile progress into.
                        progress: None,
                    },
                    &output_path,
                    None,
                    kotlin_toolchain.as_ref(),
                    android,
                ))
                .await?;
            }
            ResolvedPreviewBackend::Apple(platform) => {
                render_preview_with_apple(
                    ApplePreviewRequest {
                        host: &self.host,
                        project_path: &self.project_path,
                        platform,
                        source: request.target().source(),
                        width: request.width(),
                        height: request.height(),
                        content_deadline: request.content_deadline(),
                        sccache_path: self.sccache_path.clone(),
                        // MCP serves JSON-RPC over stdio — there is no
                        // terminal sink to render compile progress into.
                        progress: None,
                    },
                    &output_path,
                )
                .await?;
            }
        }

        let bytes = smol::fs::read(&output_path).await?;
        Ok((output_path, bytes))
    }
}

impl Tool for PreviewTool {
    type Arguments = PreviewArgs;
    type Res = ToolResult;

    fn name(&self) -> Cow<'static, str> {
        "preview".into()
    }

    async fn call(
        &self,
        args: Self::Arguments,
        _cx: ToolContext,
    ) -> aither_core::Result<Self::Res> {
        Ok(Box::pin(self.render(args)).await)
    }
}

#[cfg(test)]
mod tests {
    use crate::preview::request::PreviewTarget;

    fn project_manifest() -> Manifest {
        Manifest::parse("[package]\nname = 'Demo'\nbundle_identifier = 'dev.example.demo'\n[platforms.linux]\nbackend = 'hydrolysis'").unwrap()
    }

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
        assert_eq!(args.device, None);
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
                "platform": "android",
                "device": "emulator-5554"
            }"#,
        )
        .expect("full args parse");
        assert!(args.expr);
        assert_eq!(args.frame.as_deref(), Some("800x600"));
        assert_eq!(args.backend, Some(CliPreviewBackend::Hydrolysis));
        assert_eq!(args.theme, Some(CliHydrolysisPreviewTheme::Material3));
        assert_eq!(args.platform, Some(CliPreviewPlatform::Android));
        assert_eq!(args.device.as_deref(), Some("emulator-5554"));
    }

    #[test]
    fn sanitize_collapses_unsafe_runs() {
        assert_eq!(sanitize_target_name("views::home"), "views_home");
        assert_eq!(sanitize_target_name("text(\"hello\")"), "text_hello_");
        assert_eq!(sanitize_target_name("a.b-c_d"), "a.b-c_d");
        assert_eq!(sanitize_target_name("**"), "_");
    }

    /// The MCP tool follows `water preview`'s resolution: `platform: "ios"`
    /// renders through the in-process Mac Catalyst preview binary.
    #[test]
    fn ios_resolves_to_the_catalyst_preview_binary() {
        let args: PreviewArgs =
            serde_json::from_value(serde_json::json!({"target": "home", "platform": "ios"}))
                .expect("args parse");
        let request = args
            .resolve(&project_manifest(), "demo_app")
            .expect("resolve");
        assert_eq!(
            request.backend(),
            ResolvedPreviewBackend::Apple(crate::platform::TargetPlatform::MacCatalyst)
        );
        assert_eq!(request.hydrolysis_theme(), None);
    }

    #[test]
    fn resolves_to_the_same_request_as_water_preview() {
        // `water preview --expr --frame 800x600 --backend hydrolysis --theme
        // material3 --platform <host> 'text("hi")'` — the platform must name
        // the host OS, so derive it.
        let (platform_name, platform, target) = match std::env::consts::OS {
            "macos" => (
                "macos",
                CliPreviewPlatform::Macos,
                crate::platform::TargetPlatform::MacOS,
            ),
            "linux" => (
                "linux",
                CliPreviewPlatform::Linux,
                crate::platform::TargetPlatform::Linux,
            ),
            "windows" => (
                "windows",
                CliPreviewPlatform::Windows,
                crate::platform::TargetPlatform::Windows,
            ),
            other => panic!("test host {other} has no native preview platform"),
        };
        let args: PreviewArgs = serde_json::from_value(serde_json::json!({
            "target": "text(\"hi\")",
            "expr": true,
            "frame": "800x600",
            "backend": "hydrolysis",
            "theme": "material3",
            "platform": platform_name,
            "content_timeout": 2.5,
        }))
        .expect("args parse");
        let request = args
            .resolve(&project_manifest(), "demo_app")
            .expect("resolve");
        assert_eq!(
            request,
            PreviewRequest::new(
                platform,
                ResolvedPreviewBackend::Hydrolysis(target),
                Some(crate::preview::HydrolysisPreviewTheme::Material3),
                PreviewTarget::Expression {
                    expression: "text(\"hi\")".to_string(),
                },
                800.0,
                600.0,
                std::time::Duration::from_millis(2500),
            )
        );
    }
}
