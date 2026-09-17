//! `#[derive(TsType)]` and `#[derive(TsProps)]`.
//!
//! Both expand to a `const SCHEMA` built from the field types' own constants,
//! so the compiler resolves aliases, generic arguments and `cfg`s before
//! anything is encoded. `TsProps` additionally encodes that constant during
//! const evaluation and parks the bytes in a `waterui_meta_tsprops_*`
//! `#[used] static` — the canonical channel from a proc macro to the `water`
//! CLI. The expansion is a pure read: no file is opened and no tool is run.
//!
//! # The runtime half
//!
//! When the runtime crate is reachable — the expansion resolves through
//! `waterui` or `waterui-ts` — the derives also emit the conversions that
//! carry a value of the type across the seam the schema describes:
//!
//! * `#[derive(TsType)]` emits `IntoJs` and `FromJs`, because a nested data
//!   type travels both ways: out as a props field, back in as a callback
//!   argument. A type carrying a value that only travels outwards — a
//!   callback, which is registered rather than read — declares `#[ts(one_way)]`
//!   and gets `IntoJs` alone.
//! * `#[derive(TsProps)]` emits `IntoJs` only. Props are handed to a module;
//!   nothing ever reads a props struct back out of JavaScript, and requiring
//!   it to be readable would rule out the callbacks props exist to carry.
//!
//! A crate that depends on `waterui-ts-schema` alone — the `water` CLI, which
//! decodes schemas out of an artifact and links none of the framework — gets
//! the schema and nothing else.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{Data, DeriveInput, Fields, Ident, Variant, parse_macro_input};

/// Path to the `waterui-ts` items the expansion names, when the runtime crate
/// is reachable at all.
///
/// An application reaches it through the `waterui` facade as `waterui::ts`; a
/// crate that depends on the runtime directly names it directly, under
/// whatever name `Cargo.toml` gives it. `waterui-ts` declares
/// `extern crate self as waterui_ts`, so the `FoundCrate::Itself` arm works in
/// its own tests and doctests as well as in its library. `None` means the
/// expansion can see the schema crate only — the `water` CLI is the case — and
/// the conversions are left out.
pub fn ts_path() -> Option<TokenStream2> {
    // Inside `waterui-internal` the facade is this crate: `src/lib.rs`
    // declares `extern crate self as waterui`, so `crate` is the `waterui`
    // the expansion names — the same special case `waterui_crate_path` in
    // `lib.rs` makes. `CARGO_TARGET_TMPDIR` is set only when the package's
    // integration tests and benches compile, and there `crate` would name
    // the test or bench binary instead, so those expansions take the facade
    // arm like any other consumer.
    // Inside `waterui-ts` the runtime crate is this crate, which declares
    // `extern crate self as waterui_ts`. The lookup below must not answer with
    // the facade: `waterui` is a dev-dependency here — the equivalence tests
    // author ordinary WaterUI views — and the library build links none of it.
    if std::env::var("CARGO_PKG_NAME").as_deref() == Ok("waterui-ts") {
        return Some(quote!(::waterui_ts));
    }
    if std::env::var("CARGO_PKG_NAME").as_deref() == Ok("waterui-internal")
        && std::env::var_os("CARGO_TARGET_TMPDIR").is_none()
    {
        return Some(quote!(crate::ts));
    }

    match crate_name("waterui") {
        Ok(FoundCrate::Itself) => return Some(quote!(crate::ts)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            return Some(quote!(::#ident::ts));
        }
        Err(_) => {}
    }
    match crate_name("waterui-ts") {
        Ok(FoundCrate::Itself) => Some(quote!(::waterui_ts)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Some(quote!(::#ident))
        }
        Err(_) => None,
    }
}

/// Path to the `waterui-ts-schema` items the expansion names.
///
/// The runtime crate re-exports the schema crate as `schema`, so a consumer
/// that has the runtime names it through there and one copy of the crate
/// serves both halves of the expansion. A crate that depends on the schema
/// alone names it directly.
pub fn ts_schema_path() -> syn::Result<TokenStream2> {
    if let Some(ts) = ts_path() {
        return Ok(quote!(#ts::schema));
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

/// The TypeScript property name a field projects to.
///
/// `#[ts(rename = "onTap")]` states it, which is how a Rust `snake_case` field
/// reaches a property spelled the way TypeScript spells it — the component
/// catalog's attributes are the case that needs it, because a JSX attribute is
/// `onTap` and a Rust field cannot be. Without the attribute the field's own
/// name is the property name, and the rename travels through the schema and
/// both conversions together, so one declaration still describes one shape.
fn projected_name(field: &syn::Field) -> syn::Result<String> {
    let ident = field
        .ident
        .as_ref()
        .expect("a named field has an identifier");
    let mut renamed: Option<String> = None;
    for attribute in &field.attrs {
        if !attribute.path().is_ident("ts") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if !meta.path.is_ident("rename") {
                return Err(
                    meta.error("a field takes one TypeScript attribute: `#[ts(rename = \"…\")]`")
                );
            }
            let name: syn::LitStr = meta.value()?.parse()?;
            let value = name.value();
            if !is_js_identifier(&value) {
                return Err(syn::Error::new(
                    name.span(),
                    format!(
                        "`{value}` is not a property name TypeScript can write as `props.{value}`. \
                         A rename is an identifier: it starts with a letter, `_` or `$` and \
                         continues with those or digits"
                    ),
                ));
            }
            if renamed.replace(value).is_some() {
                return Err(meta.error("this field is renamed twice"));
            }
            Ok(())
        })?;
    }
    Ok(renamed.unwrap_or_else(|| ident.unraw().to_string()))
}

/// Whether `name` is an ECMAScript `IdentifierName`.
///
/// Reserved words are deliberately accepted: `props.class` and `props.default`
/// are legal property accesses, and a Rust field named `r#type` projecting to
/// `type` is exactly the case raw identifiers exist for. What is rejected is a
/// name no property access can reach — a space, a dash, a leading digit — which
/// would compile here and produce a `.d.ts` TypeScript refuses.
fn is_js_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    let starts = |character: char| {
        character == '_' || character == '$' || unicode_ident::is_xid_start(character)
    };
    let continues = |character: char| {
        character == '_'
            || character == '$'
            || character == '\u{200c}'
            || character == '\u{200d}'
            || unicode_ident::is_xid_continue(character)
    };
    starts(first) && characters.all(continues)
}

/// `(field access, projected property name)` for every named field, with the
/// projection checked as a whole.
///
/// Two fields projecting to one property name is a silent loss: the schema
/// declares the property twice, `IntoJs` writes it twice and the last write
/// wins, and `FromJs` reads one field's value into both. It is a compile error
/// here instead, named at the field that collides.
fn projected_names(fields: &syn::FieldsNamed) -> syn::Result<Vec<(&Ident, String)>> {
    let mut projected: Vec<(&Ident, String)> = Vec::with_capacity(fields.named.len());
    for field in &fields.named {
        let ident = field
            .ident
            .as_ref()
            .expect("a named field has an identifier");
        let name = projected_name(field)?;
        if let Some((earlier, _)) = projected.iter().find(|(_, taken)| *taken == name) {
            return Err(syn::Error::new(
                field.span(),
                format!(
                    "`{ident}` projects to the property `{name}`, which `{earlier}` already \
                     projects to. One property name is one field, or the value of one of them \
                     would never reach TypeScript"
                ),
            ));
        }
        projected.push((ident, name));
    }
    Ok(projected)
}

/// `&[FieldSchema { .. }, ..]` for named fields.
fn named_fields(path: &TokenStream2, fields: &syn::FieldsNamed) -> syn::Result<TokenStream2> {
    let entries = projected_names(fields)?
        .into_iter()
        .zip(&fields.named)
        .map(|((_, name), field)| {
            let ty = type_schema(path, &field.ty);
            quote!(#path::FieldSchema { name: #name, ty: #ty })
        })
        .collect::<Vec<_>>();
    Ok(quote!(&[#(#entries),*]))
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
    let literal = name.unraw().to_string();
    let fields = named_fields(path, fields)?;
    Ok(quote! {
        #path::TypeSchema::Struct(#path::StructSchema {
            name: #literal,
            fields: #fields,
        })
    })
}

/// The `VariantPayload` expression for one variant.
fn variant_payload(path: &TokenStream2, variant: &Variant) -> syn::Result<TokenStream2> {
    Ok(match &variant.fields {
        Fields::Unit => quote!(#path::VariantPayload::Unit),
        Fields::Unnamed(fields) => {
            let entries = unnamed_fields(path, fields);
            quote!(#path::VariantPayload::Tuple(#entries))
        }
        Fields::Named(fields) => {
            let entries = named_fields(path, fields)?;
            quote!(#path::VariantPayload::Struct(#entries))
        }
    })
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
    let variants = data
        .variants
        .iter()
        .map(|variant| {
            let literal = variant.ident.unraw().to_string();
            let payload = variant_payload(path, variant)?;
            Ok(quote!(#path::VariantSchema { name: #literal, payload: #payload }))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let literal = name.unraw().to_string();
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

/// Whether the type declared `#[ts(one_way)]`.
///
/// A type carrying a value that only travels outwards — a callback, which the
/// bridge registers rather than reads — cannot be read back out of a
/// JavaScript value, so it says so and the derive emits `IntoJs` alone.
fn is_one_way(input: &DeriveInput) -> syn::Result<bool> {
    let mut one_way = false;
    for attribute in &input.attrs {
        if !attribute.path().is_ident("ts") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("one_way") {
                one_way = true;
                return Ok(());
            }
            Err(meta.error(
                "the TypeScript derives take one attribute: `#[ts(one_way)]`, for a type that \
                 crosses only into TypeScript",
            ))
        })?;
    }
    Ok(one_way)
}

/// `(field access, projected property name)` for every named field.
fn named_field_names(fields: &syn::FieldsNamed) -> syn::Result<Vec<(&Ident, String)>> {
    projected_names(fields)
}

/// `IntoJs` for a struct with named fields, or for a struct-shaped variant.
fn object_expression(
    ts: &TokenStream2,
    fields: &syn::FieldsNamed,
    access: impl Fn(&Ident) -> TokenStream2,
) -> syn::Result<TokenStream2> {
    let entries = named_field_names(fields)?
        .into_iter()
        .map(|(ident, name)| {
            let value = access(ident);
            quote! {
                (
                    ::std::string::String::from(#name),
                    #ts::IntoJs::into_js(#value, bridge)?,
                )
            }
        })
        .collect::<Vec<_>>();
    Ok(quote!(#ts::engine::JsValue::Object(::std::vec![#(#entries),*])))
}

/// The `Self::Variant(f0, f1)` bindings of a tuple variant.
fn tuple_bindings(fields: &syn::FieldsUnnamed) -> Vec<Ident> {
    (0..fields.unnamed.len())
        .map(|index| format_ident!("field{index}"))
        .collect()
}

/// The `impl IntoJs` a derive emits.
fn into_js_impl(ts: &TokenStream2, input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let body = match &input.data {
        Data::Struct(data) => {
            let Fields::Named(fields) = &data.fields else {
                return Err(syn::Error::new(
                    data.fields.span(),
                    "a TypeScript object type needs named fields",
                ));
            };
            object_expression(ts, fields, |ident| quote!(self.#ident))?
        }
        Data::Enum(data) => {
            let string_union = data
                .variants
                .iter()
                .all(|variant| matches!(variant.fields, Fields::Unit));
            let arms = data
                .variants
                .iter()
                .map(|variant| -> syn::Result<TokenStream2> {
                    let ident = &variant.ident;
                    let literal = ident.unraw().to_string();
                    if string_union {
                        return Ok(quote! {
                            Self::#ident => #ts::engine::JsValue::String(
                                ::std::string::String::from(#literal),
                            ),
                        });
                    }
                    Ok(match &variant.fields {
                        Fields::Unit => quote! {
                            Self::#ident => #ts::support::tagged_object(
                                #literal,
                                ::core::option::Option::None,
                            ),
                        },
                        Fields::Unnamed(fields) => {
                            let bindings = tuple_bindings(fields);
                            quote! {
                                Self::#ident(#(#bindings),*) => #ts::support::tagged_object(
                                    #literal,
                                    ::core::option::Option::Some(
                                        #ts::engine::JsValue::Array(::std::vec![
                                            #(#ts::IntoJs::into_js(#bindings, bridge)?),*
                                        ]),
                                    ),
                                ),
                            }
                        }
                        Fields::Named(fields) => {
                            let bindings: Vec<&Ident> = named_field_names(fields)?
                                .into_iter()
                                .map(|(ident, _)| ident)
                                .collect();
                            let object = object_expression(ts, fields, |ident| quote!(#ident))?;
                            quote! {
                                Self::#ident { #(#bindings),* } => #ts::support::tagged_object(
                                    #literal,
                                    ::core::option::Option::Some(#object),
                                ),
                            }
                        }
                    })
                })
                .collect::<syn::Result<Vec<_>>>()?;
            quote!(match self { #(#arms)* })
        }
        Data::Union(_) => {
            return Err(syn::Error::new(
                name.span(),
                "a union has no TypeScript projection",
            ));
        }
    };

    Ok(quote! {
        impl #ts::IntoJs for #name {
            fn into_js(
                self,
                bridge: &#ts::Bridge,
            ) -> ::core::result::Result<#ts::engine::JsValue, #ts::engine::JsError> {
                ::core::result::Result::Ok(#body)
            }
        }
    })
}

/// The `impl FromJs` `#[derive(TsType)]` emits unless the type is one-way.
fn from_js_impl(ts: &TokenStream2, input: &DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let type_name = name.unraw().to_string();
    let body = match &input.data {
        Data::Struct(data) => {
            let Fields::Named(fields) = &data.fields else {
                return Err(syn::Error::new(
                    data.fields.span(),
                    "a TypeScript object type needs named fields",
                ));
            };
            let fields = named_field_names(fields)?.into_iter().map(|(ident, name)| {
                quote!(#ident: #ts::support::field(entries, #type_name, #name, bridge)?)
            });
            quote! {
                let entries = #ts::support::entries(value, #type_name)?;
                ::core::result::Result::Ok(Self { #(#fields),* })
            }
        }
        Data::Enum(data) => {
            let string_union = data
                .variants
                .iter()
                .all(|variant| matches!(variant.fields, Fields::Unit));
            let arms = data.variants.iter().map(|variant| -> syn::Result<TokenStream2> {
                let ident = &variant.ident;
                let literal = ident.unraw().to_string();
                Ok(match &variant.fields {
                    Fields::Unit => quote!(#literal => ::core::result::Result::Ok(Self::#ident),),
                    Fields::Unnamed(fields) => {
                        let arity = fields.unnamed.len();
                        let elements = (0..arity).map(|index| {
                            quote!(#ts::support::element(items, #type_name, #literal, #index, bridge)?)
                        });
                        quote! {
                            #literal => {
                                let payload = #ts::support::payload(content, #type_name, #literal)?;
                                let items = #ts::support::tuple_items(
                                    payload, #type_name, #literal, #arity,
                                )?;
                                ::core::result::Result::Ok(Self::#ident(#(#elements),*))
                            }
                        }
                    }
                    Fields::Named(fields) => {
                        let fields = named_field_names(fields)?.into_iter().map(|(ident, name)| {
                            quote!(#ident: #ts::support::field(entries, #type_name, #name, bridge)?)
                        });
                        quote! {
                            #literal => {
                                let payload = #ts::support::payload(content, #type_name, #literal)?;
                                let entries = #ts::support::entries(payload, #type_name)?;
                                ::core::result::Result::Ok(Self::#ident { #(#fields),* })
                            }
                        }
                    }
                })
            })
            .collect::<syn::Result<Vec<_>>>()?;
            if string_union {
                quote! {
                    let name = #ts::support::variant_name(value, #type_name)?;
                    match name {
                        #(#arms)*
                        _ => ::core::result::Result::Err(
                            #ts::support::unknown_variant(#type_name, name),
                        ),
                    }
                }
            } else {
                quote! {
                    let (name, content) = #ts::support::tagged(value, #type_name)?;
                    match name {
                        #(#arms)*
                        _ => ::core::result::Result::Err(
                            #ts::support::unknown_variant(#type_name, name),
                        ),
                    }
                }
            }
        }
        Data::Union(_) => {
            return Err(syn::Error::new(
                name.span(),
                "a union has no TypeScript projection",
            ));
        }
    };

    Ok(quote! {
        impl #ts::FromJs for #name {
            fn from_js(
                value: &#ts::engine::JsValue,
                bridge: &#ts::Bridge,
            ) -> ::core::result::Result<Self, #ts::engine::JsError> {
                #body
            }
        }
    })
}

/// The runtime conversions, when the runtime crate is reachable.
///
/// `both` asks for `FromJs` as well as `IntoJs`: a nested data type crosses
/// both ways, while props only ever travel into TypeScript.
fn conversions(input: &DeriveInput, both: bool) -> syn::Result<TokenStream2> {
    // The attribute is parsed first, and whatever its answer is used for: a
    // misspelled `#[ts(…)]` is a compile error on `TsProps` and in a crate
    // that sees only the schema, exactly as it is on `TsType` in the full
    // graph. A derive that skipped the parse whenever it could not act on the
    // answer would let `#[ts(oneway)]` compile and mean nothing.
    let one_way = is_one_way(input)?;
    let Some(ts) = ts_path() else {
        return Ok(TokenStream2::new());
    };
    let into_js = into_js_impl(&ts, input)?;
    if !both || one_way {
        return Ok(into_js);
    }
    let from_js = from_js_impl(&ts, input)?;
    Ok(quote! {
        #into_js
        #from_js
    })
}

/// Expand `#[derive(TsType)]`.
pub fn derive_ts_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let path = match ts_schema_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    let schema = match ts_type_impl(&path, &input) {
        Ok(tokens) => tokens,
        Err(error) => return error.into_compile_error().into(),
    };
    let conversions = match conversions(&input, true) {
        Ok(tokens) => tokens,
        Err(error) => return error.into_compile_error().into(),
    };
    quote! {
        #schema
        #conversions
    }
    .into()
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
    // Props travel one way: a module receives them, and nothing reads a props
    // struct back out of JavaScript. Requiring it to be readable would rule
    // out the callbacks and views props exist to carry.
    let conversions = match conversions(&input, false) {
        Ok(tokens) => tokens,
        Err(error) => return error.into_compile_error().into(),
    };

    let name = &input.ident;
    // The metadata symbol the CLI enumerates carries the type's name, not its
    // spelling: `r#Type` and `Type` are the same type to the contract.
    let unraw = name.unraw();
    let length = format_ident!("__WATERUI_TS_PROPS_LEN_{}", unraw);
    let encoded = format_ident!("__WATERUI_TS_PROPS_ENCODED_{}", unraw);
    let meta = format_ident!("waterui_meta_tsprops_{}", unraw);
    let meta_doc =
        format!("The encoded props contract of `{unraw}`, read back by the `water` CLI.");

    quote! {
        #schema
        #conversions

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

#[cfg(test)]
mod tests {
    use super::{is_js_identifier, ts_type_impl};
    use quote::quote;

    /// Expands the schema half of the derive, which is the half that projects
    /// property names.
    fn expand(input: &syn::DeriveInput) -> syn::Result<String> {
        ts_type_impl(&quote!(::schema), input).map(|tokens| tokens.to_string())
    }

    #[test]
    fn a_rename_reaches_the_schema() {
        let expansion = expand(&syn::parse_quote! {
            struct ButtonAttributes {
                #[ts(rename = "onTap")]
                on_tap: u32,
            }
        })
        .expect("a valid rename expands");
        assert!(expansion.contains(r#""onTap""#), "{expansion}");
        assert!(!expansion.contains(r#""on_tap""#), "{expansion}");
    }

    #[test]
    fn a_rename_that_is_not_an_identifier_is_refused() {
        let error = expand(&syn::parse_quote! {
            struct Attributes {
                #[ts(rename = "foo bar")]
                foo: u32,
            }
        })
        .expect_err("a property name with a space cannot be written as a property access");
        let message = error.to_string();
        assert!(message.contains("foo bar"), "{message}");
        assert!(message.contains("props.foo bar"), "{message}");
    }

    #[test]
    fn a_rename_that_starts_with_a_digit_is_refused() {
        expand(&syn::parse_quote! {
            struct Attributes {
                #[ts(rename = "1st")]
                first: u32,
            }
        })
        .expect_err("a property name cannot start with a digit");
    }

    #[test]
    fn two_fields_projecting_to_one_property_are_refused() {
        let error = expand(&syn::parse_quote! {
            struct Attributes {
                on_tap: u32,
                #[ts(rename = "on_tap")]
                tapped: u32,
            }
        })
        .expect_err("one property name is one field");
        let message = error.to_string();
        assert!(message.contains("on_tap"), "{message}");
        assert!(message.contains("tapped"), "{message}");
    }

    #[test]
    fn a_reserved_word_is_a_legal_property_name() {
        // `props.class` and `props.default` are legal property accesses, and a
        // Rust `r#type` projecting to `type` is what raw identifiers are for.
        assert!(is_js_identifier("class"));
        assert!(is_js_identifier("type"));
        assert!(is_js_identifier("$ref"));
        assert!(is_js_identifier("_private"));
        // A Unicode hyphen is not an ID_Continue character.
        assert!(!is_js_identifier("aria\u{2010}label"));
        assert!(!is_js_identifier(""));
        assert!(!is_js_identifier("a-b"));
    }
}
