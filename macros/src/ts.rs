//! `#[derive(TsType)]` and `#[derive(TsProps)]`.
//!
//! Both expand to a `const SCHEMA` built from the field types' own constants,
//! so the compiler resolves aliases, generic arguments and `cfg`s before
//! anything is encoded. `TsProps` additionally encodes that constant during
//! const evaluation and parks the bytes in a `waterui_meta_tsprops_*`
//! `#[used] static` — the canonical channel from a proc macro to the `water`
//! CLI. The expansion is a pure read: no file is opened and no tool is run.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, Ident, Variant, parse_macro_input};

/// Path to the `waterui-ts-schema` items the expansion names.
///
/// An application reaches the crate through the `waterui` facade; a crate that
/// depends on the schema crate directly names it directly, under whatever name
/// `Cargo.toml` gives it. The schema crate itself declares
/// `extern crate self as waterui_ts_schema`, so the `FoundCrate::Itself` arm
/// works in its own tests and doctests as well as in its library.
fn ts_schema_path() -> syn::Result<TokenStream2> {
    match crate_name("waterui") {
        Ok(FoundCrate::Itself) => return Ok(quote!(crate::ts::schema)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            return Ok(quote!(::#ident::ts::schema));
        }
        Err(_) => {}
    }
    match crate_name("waterui-ts-schema") {
        Ok(FoundCrate::Itself) => Ok(quote!(::waterui_ts_schema)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(error) => Err(syn::Error::new(
            Span::call_site(),
            format!(
                "the TypeScript schema derives require either the `waterui` crate or \
                 `waterui-ts-schema` as a dependency: {error}"
            ),
        )),
    }
}

/// The schema of one field or positional element.
///
/// Spanned at the type so a missing `TsType` impl underlines the field's own
/// declaration; the trait's `#[diagnostic::on_unimplemented]` supplies the
/// wording.
fn type_schema(path: &TokenStream2, ty: &syn::Type) -> TokenStream2 {
    quote_spanned!(ty.span()=> <#ty as #path::TsType>::SCHEMA)
}

/// `&[FieldSchema { .. }, ..]` for named fields.
fn named_fields(path: &TokenStream2, fields: &syn::FieldsNamed) -> TokenStream2 {
    let entries = fields.named.iter().map(|field| {
        let name = field
            .ident
            .as_ref()
            .expect("a named field has an identifier")
            .to_string();
        let ty = type_schema(path, &field.ty);
        quote!(#path::FieldSchema { name: #name, ty: #ty })
    });
    quote!(&[#(#entries),*])
}

/// `&[TypeSchema, ..]` for positional fields.
fn unnamed_fields(path: &TokenStream2, fields: &syn::FieldsUnnamed) -> TokenStream2 {
    let entries = fields
        .unnamed
        .iter()
        .map(|field| type_schema(path, &field.ty));
    quote!(&[#(#entries),*])
}

/// The `TypeSchema` expression for a struct with named fields.
fn struct_schema(
    path: &TokenStream2,
    name: &Ident,
    data: &syn::DataStruct,
) -> syn::Result<TokenStream2> {
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(
            data.fields.span(),
            "a TypeScript props type is an object type, so it needs named fields; \
             a tuple or unit struct has no property names to project",
        ));
    };
    let literal = name.to_string();
    let fields = named_fields(path, fields);
    Ok(quote! {
        #path::TypeSchema::Struct(#path::StructSchema {
            name: #literal,
            fields: #fields,
        })
    })
}

/// The `VariantPayload` expression for one variant.
fn variant_payload(path: &TokenStream2, variant: &Variant) -> TokenStream2 {
    match &variant.fields {
        Fields::Unit => quote!(#path::VariantPayload::Unit),
        Fields::Unnamed(fields) => {
            let entries = unnamed_fields(path, fields);
            quote!(#path::VariantPayload::Tuple(#entries))
        }
        Fields::Named(fields) => {
            let entries = named_fields(path, fields);
            quote!(#path::VariantPayload::Struct(#entries))
        }
    }
}

/// The `TypeSchema` expression for an enum.
fn enum_schema(
    path: &TokenStream2,
    name: &Ident,
    data: &syn::DataEnum,
) -> syn::Result<TokenStream2> {
    if data.variants.is_empty() {
        return Err(syn::Error::new(
            name.span(),
            "an enum with no variants has no value to project into TypeScript",
        ));
    }
    let representation = if data
        .variants
        .iter()
        .all(|variant| matches!(variant.fields, Fields::Unit))
    {
        quote!(#path::EnumRepresentation::StringUnion)
    } else {
        quote!(#path::EnumRepresentation::DEFAULT_TAGGED)
    };
    let variants = data.variants.iter().map(|variant| {
        let literal = variant.ident.to_string();
        let payload = variant_payload(path, variant);
        quote!(#path::VariantSchema { name: #literal, payload: #payload })
    });
    let literal = name.to_string();
    Ok(quote! {
        #path::TypeSchema::Enum(#path::EnumSchema {
            name: #literal,
            representation: #representation,
            variants: &[#(#variants),*],
        })
    })
}

/// The `impl TsType` both derives emit.
fn ts_type_impl(path: &TokenStream2, input: &DeriveInput) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            input.generics.span(),
            "a TypeScript schema is one constant carrying one type name, so the type \
             cannot be generic: every instantiation would claim the same name",
        ));
    }
    let name = &input.ident;
    let schema = match &input.data {
        Data::Struct(data) => struct_schema(path, name, data)?,
        Data::Enum(data) => enum_schema(path, name, data)?,
        Data::Union(_) => {
            return Err(syn::Error::new(
                name.span(),
                "a union has no TypeScript projection: which field is live is not \
                 part of its type",
            ));
        }
    };
    Ok(quote! {
        impl #path::TsType for #name {
            const SCHEMA: #path::TypeSchema = #schema;
        }
    })
}

/// Expand `#[derive(TsType)]`.
pub fn derive_ts_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let path = match ts_schema_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    match ts_type_impl(&path, &input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Expand `#[derive(TsProps)]`.
pub fn derive_ts_props(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let path = match ts_schema_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    if matches!(input.data, Data::Enum(_) | Data::Union(_)) {
        return syn::Error::new(
            input.ident.span(),
            "props are the object a TypeScript module receives, so the root type is a \
             struct with named fields",
        )
        .into_compile_error()
        .into();
    }
    let schema = match ts_type_impl(&path, &input) {
        Ok(tokens) => tokens,
        Err(error) => return error.into_compile_error().into(),
    };

    let name = &input.ident;
    let length = format_ident!("__WATERUI_TS_PROPS_LEN_{}", name);
    let encoded = format_ident!("__WATERUI_TS_PROPS_ENCODED_{}", name);
    let meta = format_ident!("waterui_meta_tsprops_{}", name);
    let meta_doc = format!("The encoded props contract of `{name}`, read back by the `water` CLI.");

    quote! {
        #schema

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        const #length: usize =
            #path::encoded_len(&<#name as #path::TsType>::SCHEMA) + 1;

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        const #encoded: [u8; #length] =
            #path::encode(&<#name as #path::TsType>::SCHEMA);

        #[doc = #meta_doc]
        #[cfg(debug_assertions)]
        #[used]
        #[allow(non_upper_case_globals)]
        #[doc(hidden)]
        pub static #meta: [u8; #length] = #encoded;

        impl #path::TsProps for #name {
            const ENCODED: &'static [u8] = #path::payload(&#encoded);
            const CONTRACT_HASH: u64 =
                #path::contract_hash(<Self as #path::TsProps>::ENCODED);
        }
    }
    .into()
}
