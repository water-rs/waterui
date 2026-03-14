//! Procedural macros for WaterUI asset management.
//!
//! Provides `asset!`, `assets!`, and `include_bundle!`.

use std::collections::BTreeMap;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    Ident, LitBool, LitStr, Token,
    parse::{Parse, ParseStream},
    parse_macro_input,
};
use waterui_assets::{AssetKind, is_loopback_http_url, is_remote_url};
use waterui_assets_plan::{BundleManifest, PlannedAsset, plan_bundle, read_assets_path};

/// Input to the `asset!` macro.
struct AssetInput {
    path: LitStr,
    embed: bool,
}

impl Parse for AssetInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path: LitStr = input.parse()?;
        let mut embed = false;
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

struct IncludeBundleInput {
    path: LitStr,
    mount: Ident,
}

impl Parse for IncludeBundleInput {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path: LitStr = input.parse()?;
        input.parse::<Token![,]>()?;
        let as_ident: Ident = input.parse()?;
        if as_ident != "as" {
            return Err(syn::Error::new(as_ident.span(), "expected `as`"));
        }
        input.parse::<Token![=]>()?;
        let mount: Ident = input.parse()?;
        Ok(Self { path, mount })
    }
}

#[derive(Default, Clone)]
struct ModuleNode {
    mount: Option<String>,
    children: BTreeMap<String, ModuleNode>,
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

fn crate_root() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map(std::path::PathBuf::from)
        .expect("CARGO_MANIFEST_DIR must be set for WaterUI asset macros")
}

fn build_manifest() -> Result<BundleManifest, String> {
    let crate_root = crate_root();
    let assets_path = read_assets_path(&crate_root).map_err(|error| error.to_string())?;
    plan_bundle(&crate_root, &assets_path).map_err(|error| error.to_string())
}

fn syn_ident(name: &str) -> syn::Ident {
    syn::parse_str(name)
        .unwrap_or_else(|error| panic!("invalid generated identifier '{name}': {error}"))
}

fn insert_asset(node: &mut ModuleNode, asset: PlannedAsset) {
    let mut current = node;
    let mut segments = asset.module_segments().into_iter().peekable();
    let mount_ident =
        (!asset.mount.is_empty()).then(|| waterui_assets_plan::rust_identifier(&asset.mount));
    while let Some(segment) = segments.next() {
        let is_mount_root = mount_ident
            .as_deref()
            .is_some_and(|mount| mount == segment.as_str());
        current = current.children.entry(segment).or_default();
        if current.mount.is_none() && is_mount_root {
            current.mount = Some(asset.mount.clone());
        }
    }
    current.assets.push(asset);
}

fn bundle_tokens(mount: Option<&str>) -> proc_macro2::TokenStream {
    match mount {
        Some(mount_name) if !mount_name.is_empty() => {
            quote! { ::waterui::Bundle::new(#mount_name) }
        }
        _ => quote! { ::waterui::Bundle::main() },
    }
}

fn asset_type_tokens(kind: AssetKind) -> proc_macro2::TokenStream {
    match kind {
        AssetKind::Font => quote! { ::waterui::FontAsset },
        AssetKind::Image => quote! { ::waterui::ImageAsset },
        AssetKind::Video => quote! { ::waterui::VideoAsset },
        AssetKind::Audio => quote! { ::waterui::AudioAsset },
        AssetKind::LargeModel => quote! { ::waterui::LargeFileAsset },
        AssetKind::Data => quote! { ::waterui::DataAsset },
    }
}

fn asset_constructor_tokens(asset: &PlannedAsset) -> proc_macro2::TokenStream {
    let bundle = bundle_tokens((!asset.mount.is_empty()).then_some(asset.mount.as_str()));
    let relative = asset.relative_path.to_string_lossy().replace('\\', "/");
    match asset.kind {
        AssetKind::Font => quote! { ::waterui::FontAsset::new(#bundle, #relative) },
        AssetKind::Image => quote! { ::waterui::ImageAsset::new(#bundle, #relative) },
        AssetKind::Video => quote! { ::waterui::VideoAsset::new(#bundle, #relative) },
        AssetKind::Audio => quote! { ::waterui::AudioAsset::new(#bundle, #relative) },
        AssetKind::LargeModel => quote! { ::waterui::LargeFileAsset::new(#bundle, #relative) },
        AssetKind::Data => quote! { ::waterui::DataAsset::new(#bundle, #relative) },
    }
}

fn emit_module(
    node: &ModuleNode,
    current_mount: Option<&str>,
    root: bool,
) -> proc_macro2::TokenStream {
    let bundle = bundle_tokens(node.mount.as_deref().or(current_mount));
    let bundle_const = if root || node.mount.is_some() {
        quote! { pub const BUNDLE: ::waterui::Bundle = #bundle; }
    } else {
        quote! {}
    };

    let mut child_tokens = Vec::new();
    for (name, child) in &node.children {
        let ident = syn_ident(name);
        let body = emit_module(child, node.mount.as_deref().or(current_mount), false);
        child_tokens.push(quote! {
            pub mod #ident {
                #body
            }
        });
    }

    let mut asset_tokens = Vec::new();
    for asset in &node.assets {
        let ident = syn_ident(&asset.item_name());
        let ty = asset_type_tokens(asset.kind);
        let ctor = asset_constructor_tokens(asset);
        asset_tokens.push(quote! {
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

#[proc_macro]
pub fn assets(input: TokenStream) -> TokenStream {
    if !proc_macro2::TokenStream::from(input).is_empty() {
        return compile_error("assets!() does not accept arguments", Span::call_site());
    }
    let manifest = match build_manifest() {
        Ok(manifest) => manifest,
        Err(error) => return compile_error(error, Span::call_site()),
    };
    let mut root = ModuleNode::default();
    for asset in manifest.assets {
        insert_asset(&mut root, asset);
    }
    let body = emit_module(&root, None, true);
    quote! {
        pub mod assets {
            #body
        }
    }
    .into()
}

#[proc_macro]
pub fn include_bundle(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as IncludeBundleInput);
    let _ = input.path;
    let _ = input.mount;
    quote! { const _: () = (); }.into()
}

#[proc_macro]
pub fn asset(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as AssetInput);
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

    let extension = match get_extension(&path_str) {
        Some(ext) => ext,
        None => return compile_error("Could not determine file extension", path_span),
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
            ::waterui::FontAsset::new(::waterui::Bundle::main(), #path_lit)
        },
        AssetKind::Image => {
            if is_remote {
                quote! { ::waterui::Photo::new(#path_lit) }
            } else {
                quote! { ::waterui::Photo::from_path(#path_lit) }
            }
        }
        AssetKind::Video => {
            if is_remote {
                quote! { ::waterui::Video::new(#path_lit) }
            } else {
                quote! { ::waterui::Video::new(::waterui::Url::from_file_path_str(#path_lit)) }
            }
        }
        AssetKind::Audio => {
            if is_remote {
                return compile_error(
                    "Remote audio assets are not supported by `asset!()` yet; use your audio pipeline directly",
                    path_span,
                );
            }
            quote! { ::waterui::AudioAsset::new(::waterui::Bundle::main(), #path_lit) }
        }
        AssetKind::Data => {
            if input.embed {
                quote! { ::waterui::Data::from_static(include_bytes!(#path_lit)) }
            } else if is_remote {
                quote! { ::waterui::Data::from_remote(#path_lit) }
            } else {
                quote! { ::waterui::Data::from_local(#path_lit) }
            }
        }
        AssetKind::LargeModel => {
            if is_remote {
                quote! { ::waterui::LargeFile::from_remote(#path_lit) }
            } else {
                quote! { ::waterui::LargeFile::from_local(#path_lit) }
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
