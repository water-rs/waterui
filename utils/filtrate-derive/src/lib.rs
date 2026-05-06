//! Procedural macros for `filtrate`.
//!
//! This crate exposes `#[derive(Filter)]`, which generates a complete
//! [`filtrate_core::Filter`] implementation from a struct attributed with
//! `#[filter(...)]`. The macro covers the regular filter patterns used by
//! the built-in filter library; bespoke filters (separable blurs, custom
//! signal traversal, ...) can still be hand-written.
//!
//! # Attributes
//!
//! `#[filter(...)]` accepts one of two top-level shapes:
//!
//! - `color_only, fragment = "<path>"` — emits a single color-only fragment
//!   pass. The path is resolved relative to the consumer crate's
//!   `src/filters/` (resolved via `concat!("../shaders/", ...)`).
//! - `spatial, shader = "<path>"` — emits a single spatial compute pass.
//!
//! Field shapes supported: tuple structs with each field typed `T` or
//! `[T; N]`, where `T` is the generic parameter that must satisfy
//! `FilterParam`. Multiple tuple fields are flattened in declared order.
//!
//! # Example
//!
//! ```ignore
//! use filtrate_core::FilterParam;
//! use filtrate_derive::Filter;
//!
//! #[derive(Filter)]
//! #[filter(color_only, fragment = "fragments/brightness.wgsl")]
//! pub struct Brightness<T>(pub T);
//! ```

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Expr, ExprLit, Fields, FieldsUnnamed, Ident, Lit,
    Token, Type, TypeArray, TypePath, parse_macro_input, parse_quote, punctuated::Punctuated,
};

/// Generate a `filtrate_core::Filter` implementation for the annotated
/// struct. See module-level docs for accepted attribute shapes.
#[proc_macro_derive(Filter, attributes(filter))]
pub fn derive_filter(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[derive(Debug)]
enum FilterKind {
    ColorOnly,
    Spatial,
}

#[derive(Debug)]
struct FilterAttrs {
    kind: FilterKind,
    shader_path: String,
}

fn expand(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let attrs = parse_filter_attr(&input.attrs)?;
    let fields = match &input.data {
        Data::Struct(DataStruct {
            fields: Fields::Unnamed(FieldsUnnamed { unnamed, .. }),
            ..
        }) => unnamed,
        Data::Struct(DataStruct {
            fields: Fields::Unit,
            ..
        }) => &Punctuated::new(),
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Filter derive requires a tuple struct (named-field structs and enums are not supported)",
            ));
        }
    };

    let layout = analyze_fields(fields)?;
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Collect generic params used as field elements that need a FilterParam bound.
    let extra_bounds = layout.element_idents.iter().collect::<Vec<_>>();
    let extra_where: Vec<TokenStream2> = extra_bounds
        .iter()
        .map(|ident| quote! { #ident: ::filtrate_core::FilterParam })
        .collect();
    let where_clause = if extra_where.is_empty() {
        where_clause.cloned()
    } else {
        let mut wc = where_clause.cloned().unwrap_or_else(|| parse_quote!(where));
        for predicate in extra_where {
            let predicate: syn::WherePredicate = syn::parse2(predicate)?;
            wc.predicates.push(predicate);
        }
        Some(wc)
    };

    let total_params = layout.total_params;
    let params_array = layout.build_params_array_tokens();
    let visit_calls = layout.build_visit_signals_tokens();

    let color_only = matches!(attrs.kind, FilterKind::ColorOnly);
    let stage_call = if color_only {
        quote! { collector.color_fragment(self.fragments(), #total_params); }
    } else {
        quote! { collector.spatial_shader(self.fragments(), #total_params); }
    };

    let shader_path = attrs.shader_path;
    let shader_include = quote! { ::core::include_str!(::core::concat!("../shaders/", #shader_path)) };

    Ok(quote! {
        impl #impl_generics ::filtrate_core::Filter for #ident #ty_generics #where_clause {
            const COLOR_ONLY: bool = #color_only;
            type Params = [f32; #total_params];
            type Fragments = &'static str;

            #[inline]
            fn params(&self) -> [f32; #total_params] {
                #params_array
            }

            #[inline]
            fn fragments(&self) -> &'static str {
                #shader_include
            }

            fn collect_stages<__C: ::filtrate_core::StageCollector>(&self, collector: &mut __C) {
                #stage_call
            }

            fn visit_signals<__V: ::filtrate_core::SignalVisitor>(&self, visitor: &mut __V) {
                #visit_calls
            }
        }
    })
}

fn parse_filter_attr(attrs: &[Attribute]) -> syn::Result<FilterAttrs> {
    let attr = attrs
        .iter()
        .find(|a| a.path().is_ident("filter"))
        .ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "Filter derive requires a `#[filter(...)]` attribute",
            )
        })?;

    let mut kind: Option<FilterKind> = None;
    let mut shader_path: Option<String> = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("color_only") {
            kind = Some(FilterKind::ColorOnly);
            Ok(())
        } else if meta.path.is_ident("spatial") {
            kind = Some(FilterKind::Spatial);
            Ok(())
        } else if meta.path.is_ident("fragment") || meta.path.is_ident("shader") {
            let value: Expr = meta.value()?.parse()?;
            let lit = match value {
                Expr::Lit(ExprLit {
                    lit: Lit::Str(s), ..
                }) => s.value(),
                _ => {
                    return Err(meta.error("expected a string literal path"));
                }
            };
            shader_path = Some(lit);
            Ok(())
        } else {
            Err(meta.error(
                "unknown #[filter(...)] argument; expected `color_only`, `spatial`, `fragment`, or `shader`",
            ))
        }
    })?;

    let kind = kind.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            "missing #[filter(color_only)] or #[filter(spatial)] marker",
        )
    })?;
    let shader_path = shader_path.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            "missing #[filter(fragment = \"...\")] or #[filter(shader = \"...\")] path",
        )
    })?;

    Ok(FilterAttrs { kind, shader_path })
}

struct FieldLayout {
    /// `Scalar(idx)` => `self.<idx>` is a single value of generic type `T`.
    /// `Array(idx, n)` => `self.<idx>` is a `[T; n]` array.
    fields: Vec<FieldEntry>,
    total_params: usize,
    element_idents: Vec<Ident>,
}

enum FieldEntry {
    Scalar { tuple_idx: usize },
    Array { tuple_idx: usize, len: usize },
}

fn analyze_fields(fields: &Punctuated<syn::Field, Token![,]>) -> syn::Result<FieldLayout> {
    let mut layout = FieldLayout {
        fields: Vec::new(),
        total_params: 0,
        element_idents: Vec::new(),
    };

    for (tuple_idx, field) in fields.iter().enumerate() {
        match &field.ty {
            Type::Path(TypePath { qself: None, path }) => {
                let ident = path
                    .get_ident()
                    .cloned()
                    .ok_or_else(|| {
                        syn::Error::new_spanned(
                            &field.ty,
                            "Filter derive expects each scalar field to be a single generic ident (e.g. `T`)",
                        )
                    })?;
                if !layout.element_idents.contains(&ident) {
                    layout.element_idents.push(ident);
                }
                layout.fields.push(FieldEntry::Scalar { tuple_idx });
                layout.total_params += 1;
            }
            Type::Array(TypeArray { elem, len, .. }) => {
                let ident = match &**elem {
                    Type::Path(TypePath { qself: None, path }) => path
                        .get_ident()
                        .cloned()
                        .ok_or_else(|| {
                            syn::Error::new_spanned(
                                elem,
                                "Filter derive expects each array element type to be a single generic ident",
                            )
                        })?,
                    _ => {
                        return Err(syn::Error::new_spanned(
                            elem,
                            "Filter derive expects each array element type to be a single generic ident",
                        ));
                    }
                };
                let len_value = match len {
                    Expr::Lit(ExprLit {
                        lit: Lit::Int(int), ..
                    }) => int.base10_parse::<usize>()?,
                    _ => {
                        return Err(syn::Error::new_spanned(
                            len,
                            "Filter derive expects array lengths to be integer literals",
                        ));
                    }
                };
                if !layout.element_idents.contains(&ident) {
                    layout.element_idents.push(ident);
                }
                layout.fields.push(FieldEntry::Array {
                    tuple_idx,
                    len: len_value,
                });
                layout.total_params += len_value;
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Filter derive supports only fields of type `T` or `[T; N]` where `T` is a single generic ident",
                ));
            }
        }
    }

    Ok(layout)
}

impl FieldLayout {
    fn build_params_array_tokens(&self) -> TokenStream2 {
        let snapshots = self.fields.iter().flat_map(|entry| match entry {
            FieldEntry::Scalar { tuple_idx } => {
                let idx = syn::Index::from(*tuple_idx);
                vec![quote! { ::filtrate_core::FilterParam::snapshot(&self.#idx) }]
            }
            FieldEntry::Array { tuple_idx, len } => {
                let idx = syn::Index::from(*tuple_idx);
                (0..*len)
                    .map(|i| {
                        let element = syn::Index::from(i);
                        quote! { ::filtrate_core::FilterParam::snapshot(&self.#idx[#element]) }
                    })
                    .collect()
            }
        });
        quote! { [ #( #snapshots ),* ] }
    }

    fn build_visit_signals_tokens(&self) -> TokenStream2 {
        let mut current = 0usize;
        let calls = self.fields.iter().map(|entry| match entry {
            FieldEntry::Scalar { tuple_idx } => {
                let idx = syn::Index::from(*tuple_idx);
                let param_idx = current;
                current += 1;
                quote! { visitor.visit(#param_idx, &self.#idx); }
            }
            FieldEntry::Array { tuple_idx, len } => {
                let idx = syn::Index::from(*tuple_idx);
                let base = current;
                current += *len;
                quote! {
                    for __i in 0..#len {
                        visitor.visit(#base + __i, &self.#idx[__i]);
                    }
                }
            }
        });
        quote! { #( #calls )* }
    }
}
