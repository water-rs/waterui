//! Procedural macros for `filtrate`.
//!
//! This crate exposes `#[derive(Filter)]`, which generates a complete
//! `filtrate_core::Filter` implementation from a struct attributed with
//! `#[filter(...)]`. The macro covers the regular filter patterns used by
//! the built-in filter library; bespoke filters (separable blurs, custom
//! signal traversal, ...) can still be hand-written.
//!
//! # Attributes
//!
//! `#[filter(...)]` takes exactly one kind marker and one shader path:
//!
//! - `color_only, shader = "<path>"` — emits a single color-only fragment
//!   pass. The path is resolved relative to the consumer crate's
//!   `src/shaders/` directory.
//! - `spatial, shader = "<path>"` — emits a single spatial compute pass.
//!
//! Repeating a marker, combining `color_only` with `spatial`, or repeating
//! `shader` is a compile error.
//!
//! # Field shapes
//!
//! Tuple structs and named-field structs are both supported. Each field is
//! typed `T` or `[T; N]`, where `T` is a generic parameter bound to
//! `FilterParam` (or a concrete type implementing it). Fields flatten into
//! the parameter array in declaration order.
//!
//! # Example
//!
//! The example is `ignore`d because it cannot compile here: the generated
//! `collect_stages` expands to
//! `include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/shaders/", <path>))`,
//! so the shader must live in the crate that writes the `#[derive]`, and a
//! proc-macro crate has no `src/shaders/`. The compiled version of this
//! example lives in `filtrate::filters`, next to the shaders it names.
//!
//! ```ignore
//! use filtrate_core::FilterParam;
//! use filtrate_derive::Filter;
//!
//! #[derive(Filter)]
//! #[filter(color_only, shader = "color/brightness.wgsl")]
//! pub struct Brightness<T>(pub T);
//!
//! #[derive(Filter)]
//! #[filter(spatial, shader = "distortion/twirl_distortion.wgsl")]
//! pub struct TwirlDistortion<T> {
//!     pub center_x: T,
//!     pub center_y: T,
//!     pub radius: T,
//!     pub angle: T,
//! }
//! ```

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{
    Attribute, Data, DataStruct, DeriveInput, Expr, ExprLit, GenericParam, Ident, Lit, Member,
    Type, TypeArray, TypePath, parse_macro_input, parse_quote,
};

/// Generate a `filtrate_core::Filter` implementation for the annotated
/// struct. See module-level docs for accepted attribute shapes.
/// Resolves the path of the crate providing the `Filter` machinery.
///
/// The derive is re-exported by `filtrate`, so a consumer may depend on
/// `filtrate-core` directly or only on `filtrate` (whose root re-exports every
/// name the generated code references). Emitting a hard-coded
/// `::filtrate_core` would break the latter, standard, arrangement.
fn core_path() -> syn::Result<TokenStream2> {
    for candidate in ["filtrate-core", "filtrate"] {
        match proc_macro_crate::crate_name(candidate) {
            Ok(proc_macro_crate::FoundCrate::Itself) => return Ok(quote! { crate }),
            Ok(proc_macro_crate::FoundCrate::Name(name)) => {
                let ident = Ident::new(&name, proc_macro2::Span::call_site());
                return Ok(quote! { ::#ident });
            }
            Err(_) => {}
        }
    }
    Err(syn::Error::new(
        proc_macro2::Span::call_site(),
        "#[derive(Filter)] requires a dependency on `filtrate` or `filtrate-core`",
    ))
}

/// Derives a complete `Filter` implementation from a `#[filter(...)]`
/// attribute; see the crate docs for the supported shapes.
#[proc_macro_derive(Filter, attributes(filter))]
pub fn derive_filter(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    let attrs = parse_filter_attr(input)?;
    let fields: Vec<&syn::Field> = match &input.data {
        Data::Struct(DataStruct { fields, .. }) => fields.iter().collect(),
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Filter derive requires a struct (enums and unions are not supported)",
            ));
        }
    };

    let generic_type_params: Vec<Ident> = input
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            GenericParam::Type(ty) => Some(ty.ident.clone()),
            _ => None,
        })
        .collect();

    let core = core_path()?;
    let layout = analyze_fields(&fields, &generic_type_params)?;
    let ident = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let extra_where: Vec<TokenStream2> = layout
        .bound_idents
        .iter()
        .map(|ident| quote! { #ident: #core::FilterParam })
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
    let params_array = layout.build_params_array_tokens(&core);
    let visit_calls = layout.build_visit_signals_tokens();

    let shader_path = attrs.shader_path;
    let shader_include = quote! {
        ::core::include_str!(::core::concat!(
            ::core::env!("CARGO_MANIFEST_DIR"),
            "/src/shaders/",
            #shader_path
        ))
    };

    let color_only = attrs.kind == FilterKind::ColorOnly;
    let stage_call = if color_only {
        quote! { collector.color_fragment(#shader_include, #total_params); }
    } else {
        quote! { collector.spatial_shader(#shader_include, #total_params); }
    };

    Ok(quote! {
        impl #impl_generics #core::Filter for #ident #ty_generics #where_clause {
            const COLOR_ONLY: bool = #color_only;
            type Params = [f32; #total_params];

            #[inline]
            fn params(&self) -> [f32; #total_params] {
                #params_array
            }

            fn collect_stages<__C: #core::StageCollector>(&self, collector: &mut __C) {
                #stage_call
            }

            fn visit_signals<__V: #core::SignalVisitor>(&self, visitor: &mut __V) {
                #visit_calls
            }
        }
    })
}

fn parse_filter_attr(input: &DeriveInput) -> syn::Result<FilterAttrs> {
    let mut filter_attrs = input.attrs.iter().filter(|a| a.path().is_ident("filter"));
    let attr: &Attribute = filter_attrs.next().ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "Filter derive requires a `#[filter(...)]` attribute",
        )
    })?;
    if let Some(duplicate) = filter_attrs.next() {
        return Err(syn::Error::new_spanned(
            duplicate,
            "duplicate #[filter(...)] attribute; declare kind and shader in one attribute",
        ));
    }

    let mut kind: Option<FilterKind> = None;
    let mut shader_path: Option<String> = None;

    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("color_only") || meta.path.is_ident("spatial") {
            let parsed = if meta.path.is_ident("color_only") {
                FilterKind::ColorOnly
            } else {
                FilterKind::Spatial
            };
            if let Some(existing) = kind {
                return Err(meta.error(if existing == parsed {
                    "duplicate filter kind marker"
                } else {
                    "conflicting filter kind markers; declare exactly one of `color_only` or `spatial`"
                }));
            }
            kind = Some(parsed);
            Ok(())
        } else if meta.path.is_ident("shader") {
            if shader_path.is_some() {
                return Err(meta.error("duplicate `shader` argument"));
            }
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
                "unknown #[filter(...)] argument; expected `color_only`, `spatial`, or `shader`",
            ))
        }
    })?;

    let kind = kind.ok_or_else(|| {
        syn::Error::new_spanned(
            attr,
            "missing #[filter(color_only)] or #[filter(spatial)] marker",
        )
    })?;
    let shader_path = shader_path
        .ok_or_else(|| syn::Error::new_spanned(attr, "missing #[filter(shader = \"...\")] path"))?;

    Ok(FilterAttrs { kind, shader_path })
}

struct FieldLayout {
    fields: Vec<FieldEntry>,
    total_params: usize,
    /// Generic type parameters that need a `FilterParam` where-bound.
    bound_idents: Vec<Ident>,
}

enum FieldEntry {
    Scalar { member: Member },
    Array { member: Member, len: usize },
}

fn analyze_fields(
    fields: &[&syn::Field],
    generic_type_params: &[Ident],
) -> syn::Result<FieldLayout> {
    let mut layout = FieldLayout {
        fields: Vec::new(),
        total_params: 0,
        bound_idents: Vec::new(),
    };

    let record_element = |layout: &mut FieldLayout, ident: &Ident| {
        // Only generic parameters need an explicit where-bound; concrete
        // element types (e.g. `f32`) resolve `FilterParam` directly and
        // fail with a normal trait error if they don't implement it.
        if generic_type_params.contains(ident) && !layout.bound_idents.contains(ident) {
            layout.bound_idents.push(ident.clone());
        }
    };

    for (field_idx, field) in fields.iter().enumerate() {
        let member = field.ident.clone().map_or_else(
            || Member::Unnamed(syn::Index::from(field_idx)),
            Member::Named,
        );
        match &field.ty {
            Type::Path(TypePath {
                qself: None, path, ..
            }) => {
                let ident = path.get_ident().cloned().ok_or_else(|| {
                    syn::Error::new_spanned(
                        &field.ty,
                        "Filter derive expects each scalar field to be a single type ident (e.g. `T`)",
                    )
                })?;
                record_element(&mut layout, &ident);
                layout.fields.push(FieldEntry::Scalar { member });
                layout.total_params += 1;
            }
            Type::Array(TypeArray { elem, len, .. }) => {
                let ident = match &**elem {
                    Type::Path(TypePath {
                        qself: None, path, ..
                    }) => path.get_ident().cloned().ok_or_else(|| {
                        syn::Error::new_spanned(
                            elem,
                            "Filter derive expects each array element type to be a single type ident",
                        )
                    })?,
                    _ => {
                        return Err(syn::Error::new_spanned(
                            elem,
                            "Filter derive expects each array element type to be a single type ident",
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
                record_element(&mut layout, &ident);
                layout.fields.push(FieldEntry::Array {
                    member,
                    len: len_value,
                });
                layout.total_params += len_value;
            }
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "Filter derive supports only fields of type `T` or `[T; N]`",
                ));
            }
        }
    }

    Ok(layout)
}

impl FieldLayout {
    fn build_params_array_tokens(&self, core: &TokenStream2) -> TokenStream2 {
        let snapshots = self.fields.iter().flat_map(|entry| match entry {
            FieldEntry::Scalar { member } => {
                vec![quote! { #core::FilterParam::snapshot(&self.#member) }]
            }
            FieldEntry::Array { member, len } => (0..*len)
                .map(|i| {
                    let element = syn::Index::from(i);
                    quote! { #core::FilterParam::snapshot(&self.#member[#element]) }
                })
                .collect(),
        });
        quote! { [ #( #snapshots ),* ] }
    }

    fn build_visit_signals_tokens(&self) -> TokenStream2 {
        let mut current = 0usize;
        let calls = self.fields.iter().map(|entry| match entry {
            FieldEntry::Scalar { member } => {
                let param_idx = current;
                current += 1;
                quote! { visitor.visit(#param_idx, &self.#member); }
            }
            FieldEntry::Array { member, len } => {
                let base = current;
                current += *len;
                quote! {
                    for __i in 0..#len {
                        visitor.visit(#base + __i, &self.#member[__i]);
                    }
                }
            }
        });
        quote! { #( #calls )* }
    }
}
