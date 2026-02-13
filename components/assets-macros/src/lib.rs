//! Procedural macros for WaterUI asset management.
//!
//! Provides the `asset!` macro for compile-time type inference and code generation.

use proc_macro::TokenStream;
use quote::quote;
use std::net::IpAddr;
use syn::{
    Ident, LitBool, LitStr, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};

/// Asset type classification based on file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    /// Image files (.png, .jpg, .jpeg, .gif, .webp, .avif)
    Image,
    /// Video files (.mp4, .mov, .webm, .avi)
    Video,
    /// Audio files (.mp3, .wav, .ogg, .flac, .aac)
    Audio,
    /// Large binary files (.onnx, .bin, .safetensors, .gguf)
    LargeModel,
    /// Small binary files (all other extensions)
    Data,
}

impl AssetKind {
    /// Infer asset kind from file extension.
    fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            // Image extensions
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "ico" => Self::Image,
            // Video extensions
            "mp4" | "mov" | "webm" | "avi" | "mkv" => Self::Video,
            // Audio extensions
            "mp3" | "wav" | "ogg" | "flac" | "aac" | "m4a" => Self::Audio,
            // Large model extensions (ML models, large binaries)
            "onnx" | "safetensors" | "gguf" | "ggml" | "pt" | "pth" | "bin" => Self::LargeModel,
            // Everything else is Data
            _ => Self::Data,
        }
    }

    /// Returns true if this asset kind supports embed mode.
    fn supports_embed(&self) -> bool {
        matches!(self, Self::Data)
    }
}

/// Input to the `asset!` macro.
struct AssetInput {
    /// The asset path or URL.
    path: LitStr,
    /// Whether to embed the asset at compile time.
    embed: bool,
}

impl Parse for AssetInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path: LitStr = input.parse()?;
        let mut embed = false;

        // Parse optional `embed = true`
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;

            let key: Ident = input.parse()?;
            if key != "embed" {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown option `{key}`, expected `embed`"),
                ));
            }

            input.parse::<Token![=]>()?;
            let value: LitBool = input.parse()?;
            embed = value.value();
        }

        Ok(Self { path, embed })
    }
}

/// Determine if a path is a remote URL.
fn is_remote_url(path: &str) -> bool {
    path.starts_with("https://") || path.starts_with("http://")
}

/// Determine if a URL is loopback HTTP (allowed).
fn is_loopback_http(path: &str) -> bool {
    extract_http_host(path)
        .and_then(normalize_host)
        .is_some_and(is_loopback_host)
}

fn extract_http_host(path: &str) -> Option<&str> {
    let remainder = path.strip_prefix("http://")?;
    let authority = remainder.split(['/', '?', '#']).next()?;
    authority.rsplit('@').next()
}

fn normalize_host(authority: &str) -> Option<&str> {
    if authority.is_empty() {
        return None;
    }

    if let Some(host) = authority.strip_prefix('[') {
        let closing = host.find(']')?;
        return Some(&host[..closing]);
    }

    authority.split(':').next()
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Extract file extension from path.
fn get_extension(path: &str) -> Option<&str> {
    // Handle URLs by stripping query params
    let path_without_query = path.split('?').next().unwrap_or(path);

    // Get the last path component
    let filename = path_without_query.rsplit('/').next()?;

    // Get extension
    let dot_pos = filename.rfind('.')?;
    Some(&filename[dot_pos + 1..])
}

/// Load and type-infer assets at compile-time.
///
/// # Type Inference
///
/// The return type is automatically inferred based on file extension:
///
/// | Extension | Return Type |
/// |-----------|-------------|
/// | `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.avif` | `Photo` |
/// | `.mp4`, `.mov`, `.webm`, `.avi` | `Video` |
/// | `.mp3`, `.wav`, `.ogg`, `.flac`, `.aac` | `Audio` |
/// | `.onnx`, `.bin`, `.safetensors`, `.gguf` | `LargeFile` |
/// | Others | `Data` |
///
/// # Sync vs Async
///
/// | Type | Local | Remote (https://) |
/// |------|-------|-------------------|
/// | `Photo` | Sync | Sync (URL-based) |
/// | `Video` | Sync | Sync (URL-based) |
/// | `Audio` | Sync | Sync (URL-based) |
/// | `Data` | Sync | **Requires `.await`** |
/// | `LargeFile` | **Requires `.await`** | **Requires `.await`** |
///
/// # Security
///
/// `http://` URLs are forbidden (except loopback hosts). Use `https://` for remote assets.
///
/// # Options
///
/// - `embed = true`: Embed the asset at compile time (only supported for `Data`).
///
/// # Examples
///
/// ```ignore
/// // Images (Photo)
/// let logo: Photo = asset!("assets/logo.png");
/// let remote_img: Photo = asset!("https://example.com/image.jpg");
///
/// // Videos (Video)
/// let intro: Video = asset!("assets/intro.mp4");
///
/// // Audio (Audio)
/// let music: Audio = asset!("assets/music.mp3");
///
/// // Small binary data (Data)
/// let config: Data = asset!("config.json");
/// let remote_config: Data = asset!("https://api.example.com/config.json").await;
///
/// // Embedded data (compile-time)
/// let shader: Data = asset!("shader.wgsl", embed = true);
///
/// // Large files (LargeFile) - always requires await
/// let model: LargeFile = asset!("model.onnx").await;
/// let remote_model: LargeFile = asset!("https://huggingface.co/model.onnx").await;
/// ```
#[proc_macro]
pub fn asset(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as AssetInput);
    let path_str = input.path.value();
    let path_span = input.path.span();

    // Check for http:// (not allowed except localhost)
    if path_str.starts_with("http://") && !is_loopback_http(&path_str) {
        return syn::Error::new(
            path_span,
            "HTTP not allowed (use HTTPS). Only loopback hosts permit HTTP.",
        )
        .to_compile_error()
        .into();
    }

    // Extract extension and infer type
    let extension = match get_extension(&path_str) {
        Some(ext) => ext,
        None => {
            return syn::Error::new(path_span, "Could not determine file extension")
                .to_compile_error()
                .into();
        }
    };

    let kind = AssetKind::from_extension(extension);
    let is_remote = is_remote_url(&path_str);

    // Check embed mode support
    if input.embed && !kind.supports_embed() {
        let type_name = match kind {
            AssetKind::Image => "Photo",
            AssetKind::Video => "Video",
            AssetKind::Audio => "Audio",
            AssetKind::LargeModel => "LargeFile",
            AssetKind::Data => "Data",
        };
        return syn::Error::new(
            path_span,
            format!("`embed = true` is not supported for {type_name}"),
        )
        .to_compile_error()
        .into();
    }

    // Check embed mode with remote URL
    if input.embed && is_remote {
        return syn::Error::new(path_span, "`embed = true` cannot be used with remote URLs")
            .to_compile_error()
            .into();
    }

    let path_lit = &input.path;

    // Generate code based on asset kind
    let output = match kind {
        AssetKind::Image => {
            // Photo is always sync (URL-based)
            if is_remote {
                quote! {
                    ::waterui::Photo::new(#path_lit)
                }
            } else {
                // Local file: convert to file:// URL
                // Note: The actual path resolution happens at runtime
                quote! {
                    ::waterui::Photo::from_path(#path_lit)
                }
            }
        }
        AssetKind::Video => {
            // Video is always sync (URL-based)
            if is_remote {
                quote! {
                    ::waterui::Video::new(#path_lit)
                }
            } else {
                quote! {
                    ::waterui::Video::from_path(#path_lit)
                }
            }
        }
        AssetKind::Audio => {
            // Audio is always sync (URL-based)
            if is_remote {
                quote! {
                    ::waterui::Audio::new(#path_lit)
                }
            } else {
                quote! {
                    ::waterui::Audio::from_path(#path_lit)
                }
            }
        }
        AssetKind::Data => {
            if input.embed {
                // Compile-time embedding
                quote! {
                    ::waterui::Data::from_static(include_bytes!(#path_lit))
                }
            } else if is_remote {
                // Remote Data - returns a Future (needs .await)
                quote! {
                    ::waterui::Data::from_remote(#path_lit)
                }
            } else {
                // Local Data - sync
                quote! {
                    ::waterui::Data::from_local(#path_lit)
                }
            }
        }
        AssetKind::LargeModel => {
            // LargeFile always requires async
            if is_remote {
                quote! {
                    ::waterui::LargeFile::from_remote(#path_lit)
                }
            } else {
                quote! {
                    ::waterui::LargeFile::from_local(#path_lit)
                }
            }
        }
    };

    output.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_loopback_http_hosts() {
        for url in [
            "http://localhost/file.bin",
            "http://LOCALHOST:8080/file.bin",
            "http://127.0.0.1/file.bin",
            "http://127.1.2.3:9000/file.bin",
            "http://[::1]/file.bin",
        ] {
            assert!(is_loopback_http(url), "expected to allow {url}");
        }
    }

    #[test]
    fn rejects_non_loopback_http_hosts() {
        for url in [
            "http://example.com/file.bin",
            "http://localhost.evil.com/file.bin",
            "http://127.0.0.1.evil.com/file.bin",
            "http://[::2]/file.bin",
        ] {
            assert!(!is_loopback_http(url), "expected to reject {url}");
        }
    }
}
