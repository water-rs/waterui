//! Procedural macros for `WaterUI` asset management.
//!
//! Provides `asset!`, `assets!`, and `include_bundle!`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::{
    Ident, LitBool, LitStr, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};
use waterui_assets_core::{AssetKind, is_loopback_http_url, is_remote_url};
use waterui_assets_planner::{BundleMountMeta, PlannedAsset, plan_mount, read_assets_path};

fn waterui_crate_path() -> syn::Result<TokenStream2> {
    match crate_name("waterui") {
        Ok(FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(error) => Err(syn::Error::new(
            Span::call_site(),
            format!(
                "WaterUI asset macros require the `waterui` crate as a dependency; \
                 Cargo.toml may rename it: {error}"
            ),
        )),
    }
}

/// Input to the `asset!` macro.
struct AssetInput {
    path: LitStr,
    embed: bool,
}

impl Parse for AssetInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path: LitStr = input.parse()?;
        let embed = if input.peek(Token![,]) {
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
            value.value()
        } else {
            false
        };
        Ok(Self { path, embed })
    }
}

/// Parsed arguments of an `include_bundle!("path", as = mount)` invocation.
struct IncludeBundleArgs {
    /// Bundle directory path relative to the crate root.
    path: LitStr,
    /// Module name the bundle is mounted under.
    mount: Ident,
}

impl Parse for IncludeBundleArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        input.parse::<Token![as]>()?;
        input.parse::<Token![=]>()?;
        let mount: Ident = input.parse()?;
        Ok(Self { path, mount })
    }
}

#[derive(Default)]
struct ModuleNode {
    children: BTreeMap<String, Self>,
    assets: Vec<PlannedAsset>,
}

fn get_extension(path: &str) -> Option<&str> {
    let path_without_query = path.split('?').next().unwrap_or(path);
    let filename = path_without_query.rsplit('/').next()?;
    let dot_pos = filename.rfind('.')?;
    Some(&filename[dot_pos + 1..])
}

fn compile_error(message: impl AsRef<str>, span: Span) -> TokenStream {
    syn::Error::new(span, message.as_ref())
        .to_compile_error()
        .into()
}

fn crate_root() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .expect("CARGO_MANIFEST_DIR must be set for WaterUI asset macros")
}

fn syn_ident(name: &str) -> Ident {
    syn::parse_str(name)
        .unwrap_or_else(|error| panic!("invalid generated identifier '{name}': {error}"))
}

fn insert_asset(node: &mut ModuleNode, asset: PlannedAsset) {
    let mut current = node;
    for segment in asset.module_segments() {
        current = current.children.entry(segment).or_default();
    }
    current.assets.push(asset);
}

fn asset_type_tokens(waterui: &TokenStream2, kind: AssetKind) -> TokenStream2 {
    match kind {
        AssetKind::Font => quote! { #waterui::FontAsset },
        AssetKind::Image => quote! { #waterui::ImageAsset },
        AssetKind::Video => quote! { #waterui::VideoAsset },
        AssetKind::Audio => quote! { #waterui::AudioAsset },
        AssetKind::LargeModel => quote! { #waterui::LargeFileAsset },
        AssetKind::Data => quote! { #waterui::DataAsset },
    }
}

fn asset_constructor_tokens(
    waterui: &TokenStream2,
    bundle: &TokenStream2,
    asset: &PlannedAsset,
) -> TokenStream2 {
    let relative = asset.relative_path.to_string_lossy().replace('\\', "/");
    match asset.kind {
        AssetKind::Font => quote! { #waterui::FontAsset::new(#bundle, #relative) },
        AssetKind::Image => quote! { #waterui::ImageAsset::new(#bundle, #relative) },
        AssetKind::Video => quote! { #waterui::VideoAsset::new(#bundle, #relative) },
        AssetKind::Audio => quote! { #waterui::AudioAsset::new(#bundle, #relative) },
        AssetKind::LargeModel => quote! { #waterui::LargeFileAsset::new(#bundle, #relative) },
        AssetKind::Data => quote! { #waterui::DataAsset::new(#bundle, #relative) },
    }
}

fn emit_module(
    waterui: &TokenStream2,
    bundle: &TokenStream2,
    node: &ModuleNode,
    root: bool,
) -> TokenStream2 {
    let bundle_const = root.then(|| {
        quote! {
            /// The asset bundle this module's accessors resolve against.
            pub const BUNDLE: #waterui::Bundle = #bundle;
        }
    });

    let mut child_tokens = Vec::new();
    for (name, child) in &node.children {
        let ident = syn_ident(name);
        let body = emit_module(waterui, bundle, child, false);
        child_tokens.push(quote! {
            #[doc = "Bundle assets under this directory."]
            pub mod #ident {
                #body
            }
        });
    }

    let mut asset_tokens = Vec::new();
    for asset in &node.assets {
        let ident = syn_ident(&asset.item_name());
        let ty = asset_type_tokens(waterui, asset.kind);
        let ctor = asset_constructor_tokens(waterui, bundle, asset);
        let doc = format!("Returns the `{}` asset.", asset.logical_path.display());
        asset_tokens.push(quote! {
            #[doc = #doc]
            pub fn #ident() -> #ty {
                #ctor
            }
        });
    }

    quote! {
        #bundle_const
        #(#child_tokens)*
        #(#asset_tokens)*
    }
}

/// Expand one mounted directory into `pub mod <mount> { ... }`.
///
/// `mount` is `""` for the main application asset root, which is emitted as
/// `pub mod assets` over `Bundle::main`. Every mount also emits:
///
/// - one `const _: &[u8] = include_bytes!(<abs file>);` per planned asset, so
///   adding or editing a file retriggers expansion (a proc macro's own reads
///   are invisible to Cargo's dependency graph otherwise);
/// - one metadata static named `waterui_meta_bundle_<mount>` whose
///   NUL-terminated [`BundleMountMeta`] payload the CLI reads back from the
///   compiled artifact's symbol table, in every profile — a `water build
///   --release` mounts the same bundle a debug build does. It carries no
///   `#[used]`: the CLI reads the crate's own rlib
///   (`waterui-cli::build::app_library_artifact` selects the app crate's rlib
///   over any linked artifact), and archive members keep their symbols
///   whether or not downstream code references them, so nothing marks the
///   static for retention into a shipped binary — the linker dead-strips it.
fn expand_mount(mount: &str, root: PathBuf, span: Span) -> TokenStream2 {
    if !root.is_dir() {
        return syn::Error::new(
            span,
            format!("bundle directory '{}' does not exist", root.display()),
        )
        .to_compile_error();
    }
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error(),
    };
    let planned = match plan_mount(&root, mount) {
        Ok(planned) => planned,
        Err(error) => return syn::Error::new(span, error.to_string()).to_compile_error(),
    };

    let main = mount.is_empty() || mount == "assets";
    let module_ident = syn_ident(if main { "assets" } else { mount });
    let bundle = if main {
        quote! { #waterui::Bundle::main() }
    } else {
        quote! { #waterui::Bundle::new(#mount) }
    };

    let mut node = ModuleNode::default();
    let mut tracking = Vec::with_capacity(planned.len());
    for asset in planned {
        let file = LitStr::new(asset.source_path.to_string_lossy().as_ref(), span);
        tracking.push(quote! {
            const _: &[u8] = include_bytes!(#file);
        });
        insert_asset(&mut node, asset);
    }
    let body = emit_module(&waterui, &bundle, &node, true);

    let meta = BundleMountMeta {
        mount: if main {
            "assets".to_string()
        } else {
            mount.to_string()
        },
        path: root,
        project: None,
    };
    let meta_ident = syn_ident(&meta.symbol_leaf());
    let payload = meta.to_payload();
    let payload_len = payload.len();
    let payload_lit = syn::LitByteStr::new(&payload, span);
    let module_doc = format!("Bundle assets mounted at `{mount}`.");

    quote! {
        #[doc = #module_doc]
        pub mod #module_ident {
            #body

            #(#tracking)*

            #[allow(non_upper_case_globals)]
            #[doc(hidden)]
            pub static #meta_ident: [u8; #payload_len] = *#payload_lit;
        }
    }
}

#[proc_macro]
/// Generates the `assets` module for the current crate.
///
/// `assets!()` is sugar for `include_bundle!(<assets_path>, as = assets)` where
/// the path comes from `Water.toml`'s `[package].assets_path` (default
/// `assets/`). The generated `assets` module is the main bundle: it keeps an
/// empty path prefix and is the only mount that can claim the `AppIcon` role.
pub fn assets(input: TokenStream) -> TokenStream {
    if !proc_macro2::TokenStream::from(input).is_empty() {
        return compile_error("assets!() does not accept arguments", Span::call_site());
    }
    let crate_root = crate_root();
    let assets_path = match read_assets_path(&crate_root) {
        Ok(path) => path,
        Err(error) => return compile_error(error.to_string(), Span::call_site()),
    };
    expand_mount("", crate_root.join(assets_path), Span::call_site()).into()
}

#[proc_macro]
/// Mounts a directory as a named asset bundle.
///
/// Expands to `pub mod <mount> { ... }` with a `BUNDLE` constant and one
/// accessor per file, and emits a `waterui_meta_bundle_<mount>` metadata
/// static so the CLI stages the directory at package time without scanning
/// sources. The path is resolved against `CARGO_MANIFEST_DIR` at expansion
/// time.
pub fn include_bundle(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input as IncludeBundleArgs);
    let path_span = args.path.span();
    let root = match crate_root().join(args.path.value()).canonicalize() {
        Ok(root) => root,
        Err(error) => {
            return compile_error(
                format!(
                    "include_bundle! path '{}' cannot be resolved: {}",
                    args.path.value(),
                    error.kind()
                ),
                path_span,
            );
        }
    };
    expand_mount(&args.mount.to_string(), root, path_span).into()
}

/// Parsed arguments of an `include_web!("web", out_dir = "…", …)` invocation.
struct IncludeWebArgs {
    /// Web project root relative to `CARGO_MANIFEST_DIR`.
    root: LitStr,
    /// Build output directory inside the root (`dist` by default).
    out_dir: Option<LitStr>,
    /// Entry document (`index.html` by default).
    entry: Option<LitStr>,
    /// SPA fallback flag.
    spa: Option<LitBool>,
    /// Content-Security-Policy override.
    csp: Option<LitStr>,
}

impl Parse for IncludeWebArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let root: LitStr = input.parse()?;
        let mut args = Self {
            root,
            out_dir: None,
            entry: None,
            spa: None,
            csp: None,
        };
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            if key == "out_dir" {
                args.out_dir = Some(input.parse()?);
            } else if key == "entry" {
                args.entry = Some(input.parse()?);
            } else if key == "spa" {
                args.spa = Some(input.parse()?);
            } else if key == "csp" {
                args.csp = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    format!(
                        "unknown include_web! option `{key}`, expected one of \
                         `out_dir`, `entry`, `spa`, `csp`"
                    ),
                ));
            }
        }
        Ok(args)
    }
}

/// One application has one web frontend: the mount is always `web`, which is
/// also what makes a second `include_web!` an error — two statics with the
/// same leaf and different payloads fail artifact enumeration.
const WEB_MOUNT: &str = "web";

#[proc_macro]
/// Embeds a web frontend into a view: `include_web!("web")` expands to the
/// `WebViewOpen` that serves the project's staged build output over the
/// engine's asset origin.
///
/// The first argument is the web project root, required, resolved against
/// `CARGO_MANIFEST_DIR`; it must contain a `package.json` (the macro points at
/// the project, not its build output). Named arguments are the complete
/// configuration surface:
///
/// - `out_dir = "build"` — the build output inside the root (`dist` default);
///   it need not exist at expansion time: the macro embeds nothing.
/// - `entry = "app.html"` — the entry document (`index.html` default).
/// - `spa = true` — unresolved extensionless paths fall back to `index.html`.
/// - `csp = "…"` — widen the strict default `Content-Security-Policy`.
///
/// Building the frontend and staging `<root>/<out_dir>` into the platform
/// bundle are the CLI's job (`water package` / `water run`); the macro records
/// the resolved paths in a `waterui_meta_bundle_web` artifact-channel symbol
/// the CLI reads back from the compiled artifact's symbol table, in every
/// profile. The macro never runs a bundler, never reads `Water.toml`, and
/// embeds no frontend bytes in the binary. On a development-linkage build
/// (the crate's `dev` feature, which the generated backend enables for `water
/// run`/`preview`/`build`) the expansion first consults the dev-server
/// handoff
/// (`WATERUI_DEV_URL` or a `--waterui-dev-url=` argument) and serves the
/// bundler's URL instead when one was handed over; a packaged build always
/// serves the staged bundle.
///
/// One `include_web!` per application: a second invocation emits a metadata
/// symbol with the same leaf and a different payload, which the CLI's artifact
/// enumeration reports as an error.
///
/// The expansion is an ordinary [`WebViewOpen`](waterui_webview::WebViewOpen),
/// so everything chains as usual:
/// `include_web!("web").serve(MyApi).inject(..).on_event(..)`.
///
/// Requires the `webview` and `assets` features of the `waterui` crate.
pub fn include_web(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input as IncludeWebArgs);
    let root_span = args.root.span();

    let root = match crate_root().join(args.root.value()).canonicalize() {
        Ok(root) => root,
        Err(error) => {
            return compile_error(
                format!(
                    "include_web! root '{}' cannot be resolved: {}",
                    args.root.value(),
                    error.kind()
                ),
                root_span,
            );
        }
    };
    if !root.join("package.json").is_file() {
        return compile_error(
            format!(
                "'{}' has no package.json — include_web! points at the web \
                 project root, not its build output",
                args.root.value()
            ),
            root_span,
        );
    }

    let out = root.join(
        args.out_dir
            .as_ref()
            .map_or_else(|| "dist".to_string(), LitStr::value),
    );
    let entry = args
        .entry
        .as_ref()
        .map_or_else(|| "index.html".to_string(), LitStr::value);
    let spa = args.spa.as_ref().is_some_and(LitBool::value);
    let csp = args.csp.as_ref().map(|csp| {
        let value = csp.value();
        quote! { .csp(#value) }
    });

    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };

    let meta = BundleMountMeta {
        mount: WEB_MOUNT.to_string(),
        path: out,
        project: Some(root.clone()),
    };
    let meta_ident = syn_ident(&meta.symbol_leaf());
    let payload = meta.to_payload();
    let payload_len = payload.len();
    let payload_lit = syn::LitByteStr::new(&payload, root_span);
    let package_json = LitStr::new(
        root.join("package.json").to_string_lossy().as_ref(),
        root_span,
    );
    let entry_lit = LitStr::new(&entry, root_span);
    let web_root = LitStr::new(WEB_MOUNT, root_span);

    quote! {
        {
            #[allow(non_upper_case_globals)]
            #[doc(hidden)]
            static #meta_ident: [u8; #payload_len] = *#payload_lit;
            // A block-scoped static is private: it is only emitted into the
            // object file when the surrounding code refers to it, so this
            // throwaway reference is what puts the symbol into the rlib the
            // CLI reads. `#[used]` would do that too, but it also survives
            // the linker's dead stripping and ships in the binary.
            let _ = &#meta_ident;

            // Cargo does not see a proc macro's filesystem reads, so the
            // package.json the expansion checked is tracked explicitly: the
            // macro re-expands when it appears or changes.
            const _: &[u8] = ::core::include_bytes!(#package_json);

            #[allow(unexpected_cfgs)]
            let dev: ::core::option::Option<#waterui::Url> = {
                #[cfg(feature = "dev")]
                {
                    #waterui::webview::dev_url()
                }
                #[cfg(not(feature = "dev"))]
                {
                    ::core::option::Option::None
                }
            };
            match dev {
                ::core::option::Option::Some(url) => #waterui::webview::WebView::open(url),
                ::core::option::Option::None => #waterui::webview::WebView::open_assets(
                    #waterui::webview::DirectoryServer::new(
                        #waterui::Bundle::new(#web_root).path("")
                    )
                    .spa(#spa)
                    #csp
                    .into_server_fn(),
                    #entry_lit,
                ),
            }
        }
    }
    .into()
}

#[proc_macro]
/// Expands a single asset path into its inferred `WaterUI` asset handle.
pub fn asset(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as AssetInput);
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    let path_str = input.path.value();
    let path_span = input.path.span();

    if path_str
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
        && !is_loopback_http_url(&path_str)
    {
        return compile_error(
            "HTTP not allowed (use HTTPS). Only loopback hosts permit HTTP.",
            path_span,
        );
    }

    let Some(extension) = get_extension(&path_str) else {
        return compile_error("Could not determine file extension", path_span);
    };

    let kind = AssetKind::from_extension(extension);
    let is_remote = is_remote_url(&path_str);
    if input.embed && !kind.supports_embed() {
        let type_name = match kind {
            AssetKind::Font => "FontAsset",
            AssetKind::Image => "Photo",
            AssetKind::Video => "Video",
            AssetKind::Audio => "AudioAsset",
            AssetKind::LargeModel => "LargeFile",
            AssetKind::Data => "Data",
        };
        return compile_error(
            format!("`embed = true` is not supported for {type_name}"),
            path_span,
        );
    }
    if input.embed && is_remote {
        return compile_error("`embed = true` cannot be used with remote URLs", path_span);
    }

    let path_lit = &input.path;
    let output = match kind {
        AssetKind::Font => quote! {
            #waterui::FontAsset::new(#waterui::Bundle::main(), #path_lit)
        },
        AssetKind::Image => {
            if is_remote {
                quote! { #waterui::media::Photo::new(#path_lit) }
            } else {
                quote! { #waterui::media::Photo::from_path(#path_lit) }
            }
        }
        AssetKind::Video => {
            if is_remote {
                quote! { #waterui::video::video(#path_lit) }
            } else {
                quote! { #waterui::video::video(#waterui::Url::from_file_path_str(#path_lit)) }
            }
        }
        AssetKind::Audio => {
            if is_remote {
                return compile_error(
                    "Remote audio assets are not supported by `asset!()` yet; use your audio pipeline directly",
                    path_span,
                );
            }
            quote! { #waterui::AudioAsset::new(#waterui::Bundle::main(), #path_lit) }
        }
        AssetKind::Data => {
            if input.embed {
                quote! { #waterui::Data::from_static(include_bytes!(#path_lit)) }
            } else if is_remote {
                quote! { #waterui::Data::from_remote(#path_lit) }
            } else {
                quote! { #waterui::Data::from_local(#path_lit) }
            }
        }
        AssetKind::LargeModel => {
            if is_remote {
                quote! { #waterui::LargeFile::from_remote(#path_lit) }
            } else {
                quote! { #waterui::LargeFile::from_local(#path_lit) }
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
            assert!(is_loopback_http_url(url), "expected to allow {url}");
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
            assert!(!is_loopback_http_url(url), "expected to reject {url}");
        }
    }
}
