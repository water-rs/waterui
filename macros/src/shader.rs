use std::path::{Component, Path, PathBuf};

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::LitStr;

use crate::{view_builder, waterui_crate_path};

/// Expands `shader!("path.wgsl")` after validating the shader at compile time.
pub fn shader(input: TokenStream) -> TokenStream {
    let caller = input
        .clone()
        .into_iter()
        .next()
        .map_or_else(proc_macro::Span::call_site, |token| token.span())
        .local_file();
    let path = match syn::parse::<LitStr>(input) {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    match expand(&path, caller.as_deref()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand(path: &LitStr, caller: Option<&Path>) -> syn::Result<TokenStream2> {
    let span = path.span();
    let raw_path = path.value();
    if raw_path.is_empty() {
        return Err(syn::Error::new(span, "shader! path cannot be empty"));
    }

    let resolved = resolve_wgsl_path(path, caller)?;
    let source = std::fs::read_to_string(&resolved).map_err(|error| {
        syn::Error::new(
            span,
            format!(
                "shader! cannot read `{}` (`{}`): {error}",
                raw_path,
                resolved.display()
            ),
        )
    })?;

    if let Err(error) = cherenkov_record::wgsl::validate_paint_wgsl(&source) {
        return Err(syn::Error::new(span, error.to_string()));
    }

    let graphics_path = graphics_crate_path(span)?;
    Ok(quote! {
        #graphics_path::shader_paint::ShaderPaintView::__from_validated_wgsl(include_str!(#path))
    })
}

fn graphics_crate_path(span: proc_macro2::Span) -> syn::Result<TokenStream2> {
    if let Some(path) = view_builder::dependency_path("waterui-graphics") {
        return Ok(path);
    }
    if let Ok(waterui) = waterui_crate_path() {
        return Ok(quote!(#waterui::graphics));
    }
    Err(syn::Error::new(
        span,
        "shader! requires either `waterui` or `waterui-graphics` as a dependency",
    ))
}

fn resolve_wgsl_path(path: &LitStr, caller: Option<&Path>) -> syn::Result<PathBuf> {
    let span = path.span();
    let written = PathBuf::from(path.value());
    if written.is_absolute() {
        return Ok(written);
    }
    let caller = caller.ok_or_else(|| {
        syn::Error::new(
            span,
            "the compiler did not say which file this shader! is written in, so the WGSL path \
             has nothing to be relative to; pass source text to `ShaderPaintView::new` instead",
        )
    })?;
    let directory = caller.parent().ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "the file `{}` has no directory to resolve `{}` against",
                caller.display(),
                written.display()
            ),
        )
    })?;
    let mut candidate = directory.to_path_buf();
    for component in written.components() {
        if !matches!(component, Component::CurDir) {
            candidate.push(component.as_os_str());
        }
    }
    if candidate.is_file() {
        return Ok(candidate);
    }
    if let Some(manifest_dir) = std::env::var_os("CARGO_MANIFEST_DIR") {
        let mut from_manifest = PathBuf::from(manifest_dir);
        from_manifest.push(&candidate);
        if from_manifest.is_file() {
            return Ok(from_manifest);
        }
    }
    Err(syn::Error::new(
        span,
        format!(
            "shader! file `{}` not found (resolved to `{}`)",
            written.display(),
            candidate.display()
        ),
    ))
}
