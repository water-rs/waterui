//! Hydrolysis platform build and package utilities.

use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use askama::Template;
use color_eyre::eyre::{self, Context, bail};
use futures::FutureExt as _;
use smol::{
    channel::{Sender, bounded},
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tracing::info;

use crate::{
    assets,
    build::BuildOptions,
    device::Artifact,
    hydrolysis::backend::HydrolysisBackend,
    macos_bundle::{MacOsUsageDescription, package_binary_as_app},
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    templates::TemplateContext,
    toolchain::{ToolchainError, windows_arm64_llvm::WindowsArm64LlvmToolchain},
    utils::{command, run_command_os, which},
};

#[cfg(target_os = "macos")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform macos --backend hydrolysis";
#[cfg(target_os = "linux")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform linux --backend hydrolysis";
#[cfg(target_os = "windows")]
const HYDROLYSIS_INIT_HINT: &str = "water run --platform windows --backend hydrolysis";
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
const HYDROLYSIS_INIT_HINT: &str = "initialize hydrolysis backend on macOS, Linux, or Windows";

const HYDROLYSIS_WEB_BOOTSTRAP_TEMPLATE: &str =
    include_str!("../templates/hydrolysis/web/bootstrap.js.tpl");
const HYDROLYSIS_WEB_STYLE_TEMPLATE: &str =
    include_str!("../templates/hydrolysis/web/style.css.tpl");

#[derive(Template)]
#[template(path = "src/templates/hydrolysis/web/index.html.tpl", escape = "none")]
struct HydrolysisWebIndexTemplate<'a> {
    ctx: &'a TemplateContext,
}

/// Build hydrolysis binary for the host platform.
///
/// # Errors
/// Returns an error if the platform is unsupported, the backend is missing, or Cargo fails.
pub async fn build_hydrolysis(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<PathBuf> {
    build_hydrolysis_with_envs_and_args(project, platform, options, &[], &[]).await
}

/// Build hydrolysis binary for the host platform with extra Cargo environment variables.
///
/// # Errors
/// Returns an error if the platform is unsupported, the backend is missing, or Cargo fails.
pub async fn build_hydrolysis_with_envs(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
    extra_envs: &[(String, OsString)],
) -> eyre::Result<PathBuf> {
    build_hydrolysis_with_envs_and_args(project, platform, options, extra_envs, &[]).await
}

/// Build hydrolysis binary for the host platform with extra Cargo environment variables and args.
///
/// # Errors
/// Returns an error if the platform is unsupported, the backend is missing, or Cargo fails.
pub async fn build_hydrolysis_with_envs_and_args(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
    extra_envs: &[(String, OsString)],
    extra_args: &[&str],
) -> eyre::Result<PathBuf> {
    if !is_hydrolysis_native_platform(platform) {
        bail!("Hydrolysis backend is only supported on macOS, Linux, and Windows");
    }

    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.backend_target_dir("hydrolysis").await?;

    if !cargo_toml.exists() {
        bail!(
            "Hydrolysis backend not found at {}. Run `{HYDROLYSIS_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    let profile = if options.is_release() {
        "release"
    } else {
        "debug"
    };

    let mut cargo = smol::process::Command::new("cargo");
    let cargo = command(&mut cargo);
    cargo.arg("build").arg("--manifest-path").arg(&cargo_toml);
    cargo.arg("--target-dir").arg(&backend_target_dir);
    cargo.args(extra_args);
    if let Some(sccache_path) = options.sccache_path() {
        cargo.env("RUSTC_WRAPPER", sccache_path);
    }
    let llvm_envs = WindowsArm64LlvmToolchain
        .cargo_envs()
        .await
        .map_err(|error| match error {
            ToolchainError::Fixable(_) => eyre::eyre!(
                "Windows ARM64 LLVM toolchain is missing. Run `water doctor --fix` to install it automatically."
            ),
            ToolchainError::Unfixable(unfixable) => {
                eyre::eyre!("Windows ARM64 LLVM toolchain check failed: {unfixable}")
            }
        })?;
    for (key, value) in llvm_envs {
        cargo.env(key, value);
    }
    for (key, value) in extra_envs {
        cargo.env(key, value);
    }
    if options.is_release() {
        cargo.arg("--release");
    }

    let output = cargo.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let details = if stderr.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        bail!(
            "Failed to build hydrolysis backend with cargo (status {}):\n{}",
            output.status,
            details
        );
    }

    Ok(backend_target_dir.join(profile))
}

/// Resolve the built hydrolysis backend binary path for the given profile.
///
/// # Errors
/// Returns an error if neither the canonical nor underscored backend binary can be found.
pub async fn built_hydrolysis_binary_path(
    project: &Project,
    profile: &str,
) -> eyre::Result<PathBuf> {
    let target_dir = project
        .backend_target_dir("hydrolysis")
        .await?
        .join(profile);
    let binary_name = project.hydrolysis_backend_crate_name();
    let binary_path = if cfg!(windows) {
        target_dir.join(format!("{binary_name}.exe"))
    } else {
        target_dir.join(binary_name.as_ref())
    };
    if binary_path.exists() {
        return Ok(binary_path);
    }

    let alt_binary_name = binary_name.replace('-', "_");
    let underscored_path = if cfg!(windows) {
        target_dir.join(format!("{alt_binary_name}.exe"))
    } else {
        target_dir.join(alt_binary_name)
    };
    if underscored_path.exists() {
        return Ok(underscored_path);
    }

    bail!(
        "Built hydrolysis binary not found at {} or {}",
        binary_path.display(),
        underscored_path.display()
    );
}

/// Clean Cargo build artifacts for hydrolysis.
///
/// # Errors
/// Returns an error if `cargo clean` fails or generated web output cannot be removed.
pub async fn clean_hydrolysis(project: &Project) -> eyre::Result<()> {
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let backend_target_dir = project.backend_target_dir("hydrolysis").await?;

    if !cargo_toml.exists() {
        return Ok(());
    }

    let args: Vec<OsString> = vec![
        "clean".into(),
        "--manifest-path".into(),
        cargo_toml.as_os_str().to_owned(),
        "--target-dir".into(),
        backend_target_dir.as_os_str().to_owned(),
    ];
    run_command_os("cargo", args).await?;
    let dist_web = backend_path.join("dist/web");
    if dist_web.exists() {
        fs::remove_dir_all(&dist_web).await?;
    }
    let dist_web_dev = backend_path.join("dist/web-dev");
    if dist_web_dev.exists() {
        fs::remove_dir_all(&dist_web_dev).await?;
    }
    Ok(())
}

/// Package a hydrolysis app.
///
/// Linux/Windows return a binary artifact path.
/// macOS returns a `.app` bundle path.
///
/// # Errors
/// Returns an error if packaging prerequisites are missing, assets cannot be staged, or output artifacts cannot be produced.
pub async fn package_hydrolysis(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> eyre::Result<Artifact> {
    if platform == TargetPlatform::Web {
        let site_root = package_hydrolysis_web_site(project, options.is_debug(), false).await?;
        return Ok(Artifact::new(project.bundle_identifier(), site_root));
    }

    if !is_hydrolysis_native_platform(platform) {
        bail!(
            "Hydrolysis backend is only supported on macOS, Linux, and Windows for binary packaging"
        );
    }

    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let backend_path = project.backend_path::<HydrolysisBackend>();
    copy_assets_and_fonts(project, &backend_path).await?;

    let final_binary_path = built_hydrolysis_binary_path(project, profile).await?;

    #[cfg(target_os = "macos")]
    {
        if platform == TargetPlatform::MacOS {
            let app_name = project
                .manifest()
                .package
                .name
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ')
                .collect::<String>();
            let app_name = if app_name.is_empty() {
                "WaterUIHydrolysis".to_string()
            } else {
                app_name
            };
            let dist_dir = backend_path.join("dist");
            fs::create_dir_all(&dist_dir).await?;
            let usage_descriptions = project
                .manifest()
                .permissions
                .iter()
                .filter(|(_, entry)| entry.is_enabled())
                .filter_map(|(key, entry)| {
                    key.apple_usage_description_key()
                        .map(|plist_key| MacOsUsageDescription {
                            plist_key,
                            description: entry.description().to_string(),
                        })
                })
                .collect::<Vec<_>>();
            let app_path = package_binary_as_app(
                &final_binary_path,
                project.bundle_identifier(),
                &app_name,
                &usage_descriptions,
                Some(&backend_path.join("resources")),
                &dist_dir,
            )
            .await?;
            return Ok(Artifact::new(project.bundle_identifier(), app_path));
        }
    }

    Ok(Artifact::new(
        project.bundle_identifier(),
        final_binary_path,
    ))
}

/// Check if a platform is supported by the hydrolysis backend.
#[must_use]
pub const fn is_hydrolysis_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::Linux
            | TargetPlatform::MacOS
            | TargetPlatform::Windows
            | TargetPlatform::Web
    )
}

const fn is_hydrolysis_native_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::Linux | TargetPlatform::MacOS | TargetPlatform::Windows
    )
}

async fn copy_assets_and_fonts(project: &Project, backend_path: &Path) -> eyre::Result<()> {
    let resources_dir = backend_path.join("resources");
    fs::create_dir_all(&resources_dir).await?;
    assets::stage_project_assets_for_gtk(project, &resources_dir).await?;

    let mut font_declarations = assets::scan_fonts(project).await?;
    font_declarations.extend(assets::hydrolysis_default_font_declarations());
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(project)?);
    if !resolved_fonts.is_empty() {
        let fonts_dest = resources_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;
        info!(
            "Copied {} fonts to hydrolysis resources",
            resolved_fonts.len()
        );
    }
    Ok(())
}

/// Build the Hydrolysis web site in debug mode for `water run --platform web`.
///
/// # Errors
/// Returns an error if the web site cannot be packaged.
pub async fn prepare_hydrolysis_web_dev_site(project: &Project) -> eyre::Result<PathBuf> {
    package_hydrolysis_web_site(project, true, true).await
}

/// A lightweight static-file server for packaged Hydrolysis web output.
#[derive(Debug)]
pub struct HydrolysisWebDevServer {
    address: SocketAddr,
    shutdown_tx: Option<Sender<()>>,
    _task: smol::Task<()>,
}

impl HydrolysisWebDevServer {
    /// Start serving the provided Hydrolysis web site root on a random localhost port.
    ///
    /// # Errors
    /// Returns an error if the local TCP listener cannot be bound.
    pub async fn start(site_root: PathBuf) -> eyre::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .wrap_err("Failed to bind Hydrolysis web dev server")?;
        let address = listener
            .local_addr()
            .wrap_err("Failed to resolve Hydrolysis web dev server address")?;
        let (shutdown_tx, shutdown_rx) = bounded::<()>(1);

        let task = smol::spawn(async move {
            let shutdown = shutdown_rx.recv().fuse();
            futures::pin_mut!(shutdown);

            loop {
                let accept = listener.accept().fuse();
                futures::pin_mut!(accept);

                match futures::future::select(accept, shutdown.as_mut()).await {
                    futures::future::Either::Left((Ok((mut stream, _peer)), _)) => {
                        let site_root = site_root.clone();
                        smol::spawn(async move {
                            if let Err(error) = serve_http_request(&mut stream, &site_root).await {
                                tracing::warn!(
                                    target: "waterui::hydrolysis::web",
                                    error = %error,
                                    "Hydrolysis web dev server request failed"
                                );
                            }
                        })
                        .detach();
                    }
                    futures::future::Either::Left((Err(error), _)) => {
                        tracing::warn!(
                            target: "waterui::hydrolysis::web",
                            error = %error,
                            "Hydrolysis web dev server accept failed"
                        );
                        break;
                    }
                    futures::future::Either::Right((_shutdown, _)) => break,
                }
            }
        });

        Ok(Self {
            address,
            shutdown_tx: Some(shutdown_tx),
            _task: task,
        })
    }

    /// Get the bound localhost address for this server instance.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for HydrolysisWebDevServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.try_send(());
        }
    }
}

async fn package_hydrolysis_web_site(
    project: &Project,
    debug: bool,
    dev_site: bool,
) -> eyre::Result<PathBuf> {
    let backend_path = project.backend_path::<HydrolysisBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    let lib_rs = backend_path.join("src/lib.rs");
    if !cargo_toml.exists() {
        bail!(
            "Hydrolysis backend not found at {}. Run `water backend add hydrolysis` first.",
            backend_path.display(),
        );
    }
    if !lib_rs.exists() {
        bail!(
            "Hydrolysis backend at {} is missing src/lib.rs for web packaging. Re-scaffold the backend and try again.",
            backend_path.display(),
        );
    }

    let site_root = backend_path.join(if dev_site { "dist/web-dev" } else { "dist/web" });
    if site_root.exists() {
        fs::remove_dir_all(&site_root).await?;
    }
    fs::create_dir_all(&site_root).await?;

    write_hydrolysis_web_shell(project, &site_root).await?;
    build_hydrolysis_web_bundle(&backend_path, &site_root, debug).await?;
    copy_web_assets_and_fonts(project, &site_root).await?;

    Ok(site_root)
}

async fn build_hydrolysis_web_bundle(
    backend_path: &Path,
    site_root: &Path,
    debug: bool,
) -> eyre::Result<()> {
    let wasm_pack = which("wasm-pack")
        .await
        .wrap_err("wasm-pack is required to build Hydrolysis web bundles")?;
    let pkg_dir = site_root.join("pkg");
    fs::create_dir_all(&pkg_dir).await?;

    let mut wasm_pack_cmd = smol::process::Command::new(wasm_pack);
    let wasm_pack_cmd = command(&mut wasm_pack_cmd);
    wasm_pack_cmd
        .current_dir(backend_path)
        .arg("build")
        .arg("--target")
        .arg("web")
        .arg("--out-dir")
        .arg(&pkg_dir)
        .arg("--out-name")
        .arg("app");
    if debug {
        wasm_pack_cmd.arg("--dev");
    } else {
        wasm_pack_cmd.arg("--release");
    }

    let output = wasm_pack_cmd.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let details = if stderr.trim().is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        bail!(
            "Failed to build Hydrolysis web bundle with wasm-pack (status {}):\n{}",
            output.status,
            details
        );
    }

    Ok(())
}

async fn write_hydrolysis_web_shell(project: &Project, site_root: &Path) -> eyre::Result<()> {
    let app_name = project
        .manifest()
        .package
        .name
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect::<String>();
    let ctx = TemplateContext::for_project_manifest(
        project.manifest(),
        project.crate_name().clone(),
        app_name,
    );

    fs::write(
        site_root.join("index.html"),
        HydrolysisWebIndexTemplate { ctx: &ctx }
            .render()
            .map_err(|error| eyre::eyre!("Failed to render hydrolysis index template: {error}"))?,
    )
    .await?;
    fs::write(
        site_root.join("bootstrap.js"),
        HYDROLYSIS_WEB_BOOTSTRAP_TEMPLATE,
    )
    .await?;
    fs::write(site_root.join("style.css"), HYDROLYSIS_WEB_STYLE_TEMPLATE).await?;
    Ok(())
}

async fn copy_web_assets_and_fonts(project: &Project, site_root: &Path) -> eyre::Result<()> {
    assets::stage_project_assets_for_web(project, site_root).await?;
    assets::stage_hydrolysis_web_fonts(project, site_root).await?;
    Ok(())
}

async fn serve_http_request(
    stream: &mut smol::net::TcpStream,
    site_root: &Path,
) -> eyre::Result<()> {
    let mut buffer = vec![0u8; 8192];
    let bytes_read = stream
        .read(&mut buffer)
        .await
        .wrap_err("Failed to read HTTP request")?;
    if bytes_read == 0 {
        return Ok(());
    }

    let request = std::str::from_utf8(&buffer[..bytes_read])
        .wrap_err("Hydrolysis web dev server received non-UTF-8 request head")?;
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| eyre::eyre!("Hydrolysis web dev server received an empty request"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or("/");
    if method != "GET" && method != "HEAD" {
        write_response(
            stream,
            405,
            "text/plain; charset=utf-8",
            b"Method Not Allowed",
        )
        .await?;
        return Ok(());
    }

    let Ok(file_path) = resolve_site_path(site_root, path) else {
        write_response(stream, 404, "text/plain; charset=utf-8", b"Not Found").await?;
        return Ok(());
    };
    let body = match fs::read(&file_path).await {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_response(stream, 404, "text/plain; charset=utf-8", b"Not Found").await?;
            return Ok(());
        }
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("Failed to read {}", file_path.display()));
        }
    };
    let mime = mime_type_for_path(&file_path);
    if method == "HEAD" {
        write_headers(stream, 200, mime, body.len()).await?;
        return Ok(());
    }
    write_response(stream, 200, mime, &body).await
}

fn resolve_site_path(site_root: &Path, request_path: &str) -> eyre::Result<PathBuf> {
    let request_path = request_path.split('?').next().unwrap_or("/");
    let trimmed = request_path.trim_start_matches('/');
    let relative = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    let mut resolved = site_root.to_path_buf();
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || segment.contains('\\') {
            bail!("Hydrolysis web dev server rejected invalid path {request_path}");
        }
        resolved.push(segment);
    }

    if resolved.is_dir() {
        resolved.push("index.html");
    }
    if !resolved.exists() {
        bail!("Hydrolysis web dev server could not resolve {request_path}");
    }
    Ok(resolved)
}

async fn write_headers(
    stream: &mut smol::net::TcpStream,
    status_code: u16,
    content_type: &str,
    content_length: usize,
) -> eyre::Result<()> {
    let status_text = match status_code {
        200 => "OK",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let headers = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {content_length}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(headers.as_bytes()).await?;
    Ok(())
}

async fn write_response(
    stream: &mut smol::net::TcpStream,
    status_code: u16,
    content_type: &str,
    body: &[u8],
) -> eyre::Result<()> {
    write_headers(stream, status_code, content_type, body.len()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    Ok(())
}

fn mime_type_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(std::ffi::OsStr::to_str) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
