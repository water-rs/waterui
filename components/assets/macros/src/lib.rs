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
/// - one `#[used]` metadata static named `waterui_meta_bundle_<mount>` whose
///   NUL-terminated [`BundleMountMeta`] payload the CLI reads back from the
///   compiled artifact's symbol table. Debug builds only: the CLI reads a
///   dev-profile host rlib, and `#[used]` is linker-retained, so the gate is
///   what keeps release binaries free of it.
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

            #[cfg(debug_assertions)]
            #[used]
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
                    "include_bundle! path '{}' cannot be resolved: {error}",
                    args.path.value()
                ),
                path_span,
            );
        }
    };
    expand_mount(&args.mount.to_string(), root, path_span).into()
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
