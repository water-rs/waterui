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

use std::env;

use heck::{
    ToKebabCase as _, ToLowerCamelCase as _, ToShoutyKebabCase as _, ToShoutySnakeCase as _,
    ToSnakeCase as _, ToUpperCamelCase as _,
};
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt as _;
use syn::spanned::Spanned as _;
use syn::{
    Data, DataEnum, DeriveInput, Field, Fields, FieldsNamed, FieldsUnnamed, Ident, LitStr, Variant,
    parse_macro_input,
};

/// How the `waterui-ts` runtime crate is reached from the invoking crate.
enum TsRuntimePath {
    /// Direct dependency on `waterui-ts` (runtime conversions always exist).
    Direct(TokenStream2),
    /// Reached through the `waterui` facade (`ts` may be disabled when only
    /// `webview` is enabled, so conversions are gated by `__with_ts_runtime!`).
    Facade {
        facade: TokenStream2,
        ts: TokenStream2,
    },
}

impl TsRuntimePath {
    const fn ts_path(&self) -> &TokenStream2 {
        match self {
            Self::Direct(ts) | Self::Facade { ts, .. } => ts,
        }
    }
}

fn ts_runtime_path() -> Option<TsRuntimePath> {
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
    if env::var("CARGO_PKG_NAME").as_deref() == Ok("waterui-ts") {
        return Some(TsRuntimePath::Direct(quote!(::waterui_ts)));
    }
    if env::var("CARGO_PKG_NAME").as_deref() == Ok("waterui-internal")
        && env::var_os("CARGO_TARGET_TMPDIR").is_none()
    {
        return Some(TsRuntimePath::Facade {
            facade: quote!(crate),
            ts: quote!(crate::ts),
        });
    }
    if env::var("CARGO_PKG_NAME").as_deref() == Ok("waterui-webview")
        && env::var_os("CARGO_TARGET_TMPDIR").is_none()
    {
        return None;
    }

    match crate_name("waterui") {
        Ok(FoundCrate::Itself) => {
            return Some(TsRuntimePath::Facade {
                facade: quote!(crate),
                ts: quote!(crate::ts),
            });
        }
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            return Some(TsRuntimePath::Facade {
                facade: quote!(::#ident),
                ts: quote!(::#ident::ts),
            });
        }
        Err(_) => {}
    }
    match crate_name("waterui-ts") {
        Ok(FoundCrate::Itself) => Some(TsRuntimePath::Direct(quote!(::waterui_ts))),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Some(TsRuntimePath::Direct(quote!(::#ident)))
        }
        Err(_) => None,
    }
}

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
    ts_runtime_path().map(|runtime| match runtime {
        TsRuntimePath::Direct(ts) | TsRuntimePath::Facade { ts, .. } => ts,
    })
}

/// Path to the `waterui-ts-schema` items the expansion names.
///
/// Both `waterui/ts` and `waterui/webview` re-export the schema crate through
/// `waterui::__ts_schema`, so a consumer that depends on `waterui` with either
/// feature names it there. Crates depending directly on `waterui-ts`,
/// `waterui-webview` or `waterui-ts-schema` resolve through those crates.
pub fn ts_schema_path() -> syn::Result<TokenStream2> {
    if env::var("CARGO_PKG_NAME").as_deref() == Ok("waterui-webview")
        && env::var_os("CARGO_TARGET_TMPDIR").is_none()
    {
        return Ok(quote!(crate::ts_schema));
    }
    match ts_runtime_path() {
        Some(TsRuntimePath::Direct(ts)) => return Ok(quote!(#ts::schema)),
        Some(TsRuntimePath::Facade { facade, .. }) => return Ok(quote!(#facade::__ts_schema)),
        None => {}
    }
    match crate_name("waterui-webview") {
        Ok(FoundCrate::Itself) => return Ok(quote!(crate::ts_schema)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            return Ok(quote!(::#ident::ts_schema));
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
                "the TypeScript schema derives require `waterui`, `waterui-webview` or \
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

/// Case-conversion convention selected by `#[ts(rename_all = "…")]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenameRule {
    CamelCase,
    PascalCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
    LowerCase,
    UpperCase,
    Verbatim,
}

impl RenameRule {
    fn parse(value: &str, span: Span) -> syn::Result<Self> {
        match value {
            "camelCase" => Ok(Self::CamelCase),
            "PascalCase" => Ok(Self::PascalCase),
            "snake_case" => Ok(Self::SnakeCase),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnakeCase),
            "kebab-case" => Ok(Self::KebabCase),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebabCase),
            "lowercase" => Ok(Self::LowerCase),
            "UPPERCASE" => Ok(Self::UpperCase),
            "verbatim" => Ok(Self::Verbatim),
            _ => Err(syn::Error::new(
                span,
                format!(
                    "unknown `#[ts(rename_all = \"{value}\")]` convention; expected one of \
                     \"camelCase\", \"PascalCase\", \"snake_case\", \"SCREAMING_SNAKE_CASE\", \
                     \"kebab-case\", \"SCREAMING-KEBAB-CASE\", \"lowercase\", \"UPPERCASE\", \
                     or \"verbatim\""
                ),
            )),
        }
    }

    fn apply(self, raw: &str) -> String {
        match self {
            Self::CamelCase => raw.to_lower_camel_case(),
            Self::PascalCase => raw.to_upper_camel_case(),
            Self::SnakeCase => raw.to_snake_case(),
            Self::ScreamingSnakeCase => raw.to_shouty_snake_case(),
            Self::KebabCase => raw.to_kebab_case(),
            Self::ScreamingKebabCase => raw.to_shouty_kebab_case(),
            Self::LowerCase => raw.to_lowercase(),
            Self::UpperCase => raw.to_uppercase(),
            Self::Verbatim => raw.to_owned(),
        }
    }
}

/// Container-level `#[ts(…)]` attributes on a struct or enum.
#[derive(Clone, Copy, Debug, Default)]
struct ContainerAttrs {
    one_way: bool,
    rename_all: Option<RenameRule>,
}

impl ContainerAttrs {
    fn parse(input: &DeriveInput) -> syn::Result<Self> {
        let mut attrs = Self::default();
        for attribute in &input.attrs {
            if !attribute.path().is_ident("ts") {
                continue;
            }
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("one_way") {
                    attrs.one_way = true;
                    return Ok(());
                }
                if meta.path.is_ident("rename_all") {
                    let value: LitStr = meta.value()?.parse()?;
                    let rule = RenameRule::parse(&value.value(), value.span())?;
                    if attrs.rename_all.replace(rule).is_some() {
                        return Err(meta.error("`rename_all` is specified more than once"));
                    }
                    return Ok(());
                }
                Err(meta.error(
                    "the TypeScript derives accept `#[ts(one_way)]` and `#[ts(rename_all = \"…\")]`",
                ))
            })?;
        }
        Ok(attrs)
    }
}

/// The TypeScript property name a field projects to.
///
/// Fields project as `camelCase` by default, or according to the container's
/// `#[ts(rename_all = "…")]` convention. `#[ts(rename = "…")]` overrides that
/// for an individual field, and the projected name travels through the schema
/// and both conversions together so one declaration describes one shape.
fn projected_name(field: &Field, rule: RenameRule) -> syn::Result<String> {
    let ident = field
        .ident
        .as_ref()
        .expect("a named field has an identifier");
    let mut renamed: Option<(String, Span)> = None;
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
            let name: LitStr = meta.value()?.parse()?;
            let value = name.value();
            if renamed.replace((value, name.span())).is_some() {
                return Err(meta.error("this field is renamed twice"));
            }
            Ok(())
        })?;
    }
    let (value, span) =
        renamed.unwrap_or_else(|| (rule.apply(&ident.unraw().to_string()), ident.span()));
    if !is_js_identifier(&value) {
        return Err(syn::Error::new(
            span,
            format!(
                "`{value}` is not a property name TypeScript can write as `props.{value}`. \
                 A property name is an identifier: it starts with a letter, `_` or `$` and \
                 continues with those or digits"
            ),
        ));
    }
    Ok(value)
}

/// Whether `name` is an ECMAScript `IdentifierName`.
///
/// Reserved words are deliberately accepted: `props.class` and `props.default`
/// are legal property accesses, and a Rust `r#type` projecting to `type` is
/// exactly the case raw identifiers exist for. What is rejected is a name no
/// property access can reach — a space, a dash, a leading digit — which would
/// compile here and produce a `.d.ts` TypeScript refuses.
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
fn projected_names(fields: &FieldsNamed, rule: RenameRule) -> syn::Result<Vec<(&Ident, String)>> {
    let mut projected: Vec<(&Ident, String)> = Vec::with_capacity(fields.named.len());
    for field in &fields.named {
        let ident = field
            .ident
            .as_ref()
            .expect("a named field has an identifier");
        let name = projected_name(field, rule)?;
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

/// The TypeScript variant string literal a variant projects to.
///
/// Unit-only enums project their variants as `camelCase` by default;
/// data-carrying enums keep their Rust spelling unless `#[ts(rename_all = "…")]`
/// is declared. `#[ts(rename = "…")]` overrides the projected literal for an
/// individual variant.
fn projected_variant_name(variant: &Variant, rule: RenameRule) -> syn::Result<String> {
    let mut renamed: Option<(String, Span)> = None;
    for attribute in &variant.attrs {
        if !attribute.path().is_ident("ts") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if !meta.path.is_ident("rename") {
                return Err(
                    meta.error("a variant takes one TypeScript attribute: `#[ts(rename = \"…\")]`")
                );
            }
            let name: LitStr = meta.value()?.parse()?;
            let value = name.value();
            if value.is_empty() {
                return Err(syn::Error::new(
                    name.span(),
                    "a variant cannot be renamed to an empty string",
                ));
            }
            if renamed.replace((value, name.span())).is_some() {
                return Err(meta.error("this variant is renamed twice"));
            }
            Ok(())
        })?;
    }
    Ok(renamed.map_or_else(
        || rule.apply(&variant.ident.unraw().to_string()),
        |(value, _)| value,
    ))
}

/// `(variant, projected string literal)` for every variant of an enum, checked
/// for collisions.
fn projected_variant_names(
    data: &DataEnum,
    rule: RenameRule,
) -> syn::Result<Vec<(&Variant, String)>> {
    let mut projected: Vec<(&Variant, String)> = Vec::with_capacity(data.variants.len());
    for variant in &data.variants {
        let name = projected_variant_name(variant, rule)?;
        if let Some((earlier, _)) = projected.iter().find(|(_, taken)| *taken == name) {
            return Err(syn::Error::new(
                variant.span(),
                format!(
                    "`{}` projects to the variant `{name}`, which `{}` already projects to. \
                     One variant name is one arm",
                    variant.ident, earlier.ident
                ),
            ));
        }
        projected.push((variant, name));
    }
    Ok(projected)
}

/// `&[FieldSchema::new(..), ..]` for named fields.
fn named_fields(
    path: &TokenStream2,
    fields: &FieldsNamed,
    rule: RenameRule,
) -> syn::Result<TokenStream2> {
    let entries = projected_names(fields, rule)?
        .into_iter()
        .zip(&fields.named)
        .map(|((_, name), field)| {
            let ty = type_schema(path, &field.ty);
            quote!(#path::FieldSchema::new(#name, #ty))
        })
        .collect::<Vec<_>>();
    Ok(quote!(&[#(#entries),*]))
}

/// `&[TypeSchema, ..]` for positional fields.
fn unnamed_fields(path: &TokenStream2, fields: &FieldsUnnamed) -> TokenStream2 {
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
    attrs: ContainerAttrs,
) -> syn::Result<TokenStream2> {
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new(
            data.fields.span(),
            "a TypeScript props type is an object type, so it needs named fields; \
             a tuple or unit struct has no property names to project",
        ));
    };
    let literal = name.unraw().to_string();
    let rule = attrs.rename_all.unwrap_or(RenameRule::CamelCase);
    let fields = named_fields(path, fields, rule)?;
    Ok(quote! {
        #path::TypeSchema::Struct(#path::StructSchema::new(#literal, #fields))
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
            let entries = named_fields(path, fields, RenameRule::CamelCase)?;
            quote!(#path::VariantPayload::Struct(#entries))
        }
    })
}

/// The `TypeSchema` expression for an enum.
fn enum_schema(
    path: &TokenStream2,
    name: &Ident,
    data: &DataEnum,
    attrs: ContainerAttrs,
) -> syn::Result<TokenStream2> {
    if data.variants.is_empty() {
        return Err(syn::Error::new(
            name.span(),
            "an enum with no variants has no value to project into TypeScript",
        ));
    }
    let string_union = data
        .variants
        .iter()
        .all(|variant| matches!(variant.fields, Fields::Unit));
    let representation = if string_union {
        quote!(#path::EnumRepresentation::StringUnion)
    } else {
        quote!(#path::EnumRepresentation::DEFAULT_TAGGED)
    };
    let variant_rule = attrs.rename_all.unwrap_or(if string_union {
        RenameRule::CamelCase
    } else {
        RenameRule::Verbatim
    });
    let variants = projected_variant_names(data, variant_rule)?
        .into_iter()
        .map(|(variant, literal)| {
            let payload = variant_payload(path, variant)?;
            Ok(quote!(#path::VariantSchema::new(#literal, #payload)))
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let literal = name.unraw().to_string();
    Ok(quote! {
        #path::TypeSchema::Enum(#path::EnumSchema::new(
            #literal,
            #representation,
            &[#(#variants),*],
        ))
    })
}

/// The `impl TsType` both derives emit.
fn ts_type_impl(
    path: &TokenStream2,
    input: &DeriveInput,
    attrs: ContainerAttrs,
) -> syn::Result<TokenStream2> {
    if !input.generics.params.is_empty() || input.generics.where_clause.is_some() {
        return Err(syn::Error::new(
            input.generics.span(),
            "a TypeScript schema is one constant carrying one type name, so the type \
             cannot be generic: every instantiation would claim the same name",
        ));
    }
    let name = &input.ident;
    let schema = match &input.data {
        Data::Struct(data) => struct_schema(path, name, data, attrs)?,
        Data::Enum(data) => enum_schema(path, name, data, attrs)?,
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

/// `IntoJs` for a struct with named fields, or for a struct-shaped variant.
fn object_expression(
    ts: &TokenStream2,
    fields: &FieldsNamed,
    rule: RenameRule,
    access: impl Fn(&Ident) -> TokenStream2,
) -> syn::Result<TokenStream2> {
    let entries = projected_names(fields, rule)?
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
fn tuple_bindings(fields: &FieldsUnnamed) -> Vec<Ident> {
    (0..fields.unnamed.len())
        .map(|index| format_ident!("field{index}"))
        .collect()
}

/// The `impl IntoJs` a derive emits.
fn into_js_impl(
    ts: &TokenStream2,
    input: &DeriveInput,
    attrs: ContainerAttrs,
) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let body = match &input.data {
        Data::Struct(data) => {
            let Fields::Named(fields) = &data.fields else {
                return Err(syn::Error::new(
                    data.fields.span(),
                    "a TypeScript object type needs named fields",
                ));
            };
            let rule = attrs.rename_all.unwrap_or(RenameRule::CamelCase);
            object_expression(ts, fields, rule, |ident| quote!(self.#ident))?
        }
        Data::Enum(data) => {
            let string_union = data
                .variants
                .iter()
                .all(|variant| matches!(variant.fields, Fields::Unit));
            let variant_rule = attrs.rename_all.unwrap_or(if string_union {
                RenameRule::CamelCase
            } else {
                RenameRule::Verbatim
            });
            let arms = projected_variant_names(data, variant_rule)?
                .into_iter()
                .map(|(variant, literal)| -> syn::Result<TokenStream2> {
                    let ident = &variant.ident;
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
                            let bindings: Vec<&Ident> =
                                projected_names(fields, RenameRule::CamelCase)?
                                    .into_iter()
                                    .map(|(ident, _)| ident)
                                    .collect();
                            let object = object_expression(
                                ts,
                                fields,
                                RenameRule::CamelCase,
                                |ident| quote!(#ident),
                            )?;
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

/// The `impl FromJs` body for an enum.
fn from_js_enum_body(
    ts: &TokenStream2,
    type_name: &str,
    data: &DataEnum,
    attrs: ContainerAttrs,
) -> syn::Result<TokenStream2> {
    let string_union = data
        .variants
        .iter()
        .all(|variant| matches!(variant.fields, Fields::Unit));
    let variant_rule = attrs.rename_all.unwrap_or(if string_union {
        RenameRule::CamelCase
    } else {
        RenameRule::Verbatim
    });
    let arms = projected_variant_names(data, variant_rule)?
        .into_iter()
        .map(|(variant, literal)| -> syn::Result<TokenStream2> {
            let ident = &variant.ident;
            Ok(match &variant.fields {
                Fields::Unit => {
                    quote!(#literal => ::core::result::Result::Ok(Self::#ident),)
                }
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
                    let fields = projected_names(fields, RenameRule::CamelCase)?
                        .into_iter()
                        .map(|(ident, name)| {
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
        Ok(quote! {
            let name = #ts::support::variant_name(value, #type_name)?;
            match name {
                #(#arms)*
                _ => ::core::result::Result::Err(
                    #ts::support::unknown_variant(#type_name, name),
                ),
            }
        })
    } else {
        Ok(quote! {
            let (name, content) = #ts::support::tagged(value, #type_name)?;
            match name {
                #(#arms)*
                _ => ::core::result::Result::Err(
                    #ts::support::unknown_variant(#type_name, name),
                ),
            }
        })
    }
}

/// The `impl FromJs` `#[derive(TsType)]` emits unless the type is one-way.
fn from_js_impl(
    ts: &TokenStream2,
    input: &DeriveInput,
    attrs: ContainerAttrs,
) -> syn::Result<TokenStream2> {
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
            let rule = attrs.rename_all.unwrap_or(RenameRule::CamelCase);
            let fields = projected_names(fields, rule)?
                .into_iter()
                .map(|(ident, name)| {
                    quote!(#ident: #ts::support::field(entries, #type_name, #name, bridge)?)
                });
            quote! {
                let entries = #ts::support::entries(value, #type_name)?;
                ::core::result::Result::Ok(Self { #(#fields),* })
            }
        }
        Data::Enum(data) => from_js_enum_body(ts, &type_name, data, attrs)?,
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
fn conversions(
    input: &DeriveInput,
    attrs: ContainerAttrs,
    both: bool,
) -> syn::Result<TokenStream2> {
    let Some(runtime) = ts_runtime_path() else {
        return Ok(TokenStream2::new());
    };
    let ts = runtime.ts_path();
    let into_js = into_js_impl(ts, input, attrs)?;
    let impls = if !both || attrs.one_way {
        into_js
    } else {
        let from_js = from_js_impl(ts, input, attrs)?;
        quote! {
            #into_js
            #from_js
        }
    };
    Ok(match runtime {
        TsRuntimePath::Direct(_) => impls,
        TsRuntimePath::Facade { facade, .. } => quote! {
            #facade::__with_ts_runtime! {
                #impls
            }
        },
    })
}

/// Expand `#[derive(TsType)]`.
pub fn derive_ts_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let path = match ts_schema_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    let attrs = match ContainerAttrs::parse(&input) {
        Ok(attrs) => attrs,
        Err(error) => return error.into_compile_error().into(),
    };
    let schema = match ts_type_impl(&path, &input, attrs) {
        Ok(tokens) => tokens,
        Err(error) => return error.into_compile_error().into(),
    };
    let conversions = match conversions(&input, attrs, true) {
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
    let attrs = match ContainerAttrs::parse(&input) {
        Ok(attrs) => attrs,
        Err(error) => return error.into_compile_error().into(),
    };
    let schema = match ts_type_impl(&path, &input, attrs) {
        Ok(tokens) => tokens,
        Err(error) => return error.into_compile_error().into(),
    };
    // Props travel one way: a module receives them, and nothing reads a props
    // struct back out of JavaScript. Requiring it to be readable would rule
    // out the callbacks and views props exist to carry.
    let conversions = match conversions(&input, attrs, false) {
        Ok(tokens) => tokens,
        Err(error) => return error.into_compile_error().into(),
    };

    let name = &input.ident;
    // The metadata record the CLI enumerates carries the type's name, not its
    // spelling: `r#Type` and `Type` are the same type to the contract.
    let unraw = name.unraw();
    let length = format_ident!("__WATERUI_TS_PROPS_LEN_{}", unraw);
    let encoded = format_ident!("__WATERUI_TS_PROPS_ENCODED_{}", unraw);
    let record_len = format_ident!("__WATERUI_TS_PROPS_RECORD_LEN_{}", unraw);
    let meta = format_ident!("waterui_meta_tsprops_{}", unraw);
    let meta_doc =
        format!("The encoded props contract of `{unraw}`, read back by the `water` CLI.");
    // The record names itself: `name`, NUL, `payload`, NUL, so a reader can
    // walk the section it lives in and match names — a linked image need not
    // carry a symbol table, and a linked PE carries none.
    let meta_name = meta.to_string();

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

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        const #record_len: usize = #meta_name.len() + #length + 2;

        // The static lives in the metadata directory section every crate's
        // derives fill, and its bytes name themselves: the artifact reader
        // walks the section's `name`, NUL, `payload`, NUL records, which needs
        // no symbol table — a linked MSVC image carries none.
        #[doc = #meta_doc]
        #[cfg(debug_assertions)]
        #[used]
        #[allow(non_upper_case_globals)]
        #[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = ".wmeta"))]
        #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__wmeta"))]
        #[doc(hidden)]
        pub static #meta: [u8; #record_len] =
            #path::dir_entry::<#record_len>(#meta_name.as_bytes(), &#encoded);

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
    use super::{ContainerAttrs, is_js_identifier, ts_type_impl};
    use quote::quote;
    use syn::{DeriveInput, parse_quote};

    /// Expands the schema half of the derive, which is the half that projects
    /// property and variant names.
    fn expand(input: &DeriveInput) -> syn::Result<String> {
        let attrs = ContainerAttrs::parse(input)?;
        ts_type_impl(&quote!(::schema), input, attrs).map(|tokens| tokens.to_string())
    }

    #[test]
    fn fields_project_as_camel_case_by_default() {
        let expansion = expand(&parse_quote! {
            struct Props {
                item_count: u32,
                on_tap: u32,
                a11y_label: u32,
            }
        })
        .expect("default camelCase fields expand");
        assert!(expansion.contains(r#""itemCount""#), "{expansion}");
        assert!(expansion.contains(r#""onTap""#), "{expansion}");
        assert!(expansion.contains(r#""a11yLabel""#), "{expansion}");
        assert!(!expansion.contains(r#""item_count""#), "{expansion}");
    }

    #[test]
    fn unit_enum_variants_project_as_camel_case_by_default() {
        let expansion = expand(&parse_quote! {
            enum Align {
                Leading,
                BottomTrailing,
            }
        })
        .expect("unit enum expands");
        assert!(expansion.contains(r#""leading""#), "{expansion}");
        assert!(expansion.contains(r#""bottomTrailing""#), "{expansion}");
        assert!(!expansion.contains(r#""Leading""#), "{expansion}");
    }

    #[test]
    fn tagged_enum_keeps_variant_names_and_camel_cases_struct_fields() {
        let expansion = expand(&parse_quote! {
            enum Slot {
                Empty,
                Note { line_count: u32 },
            }
        })
        .expect("tagged enum expands");
        assert!(expansion.contains(r#""Empty""#), "{expansion}");
        assert!(expansion.contains(r#""Note""#), "{expansion}");
        assert!(expansion.contains(r#""lineCount""#), "{expansion}");
        assert!(!expansion.contains(r#""line_count""#), "{expansion}");
    }

    #[test]
    fn rename_all_selects_container_convention() {
        let snake = expand(&parse_quote! {
            #[ts(rename_all = "snake_case")]
            struct Props {
                item_count: u32,
                #[ts(rename = "customName")]
                overridden_field: u32,
            }
        })
        .expect("snake_case rename_all expands");
        assert!(snake.contains(r#""item_count""#), "{snake}");
        assert!(snake.contains(r#""customName""#), "{snake}");

        let kebab = expand(&parse_quote! {
            #[ts(rename_all = "kebab-case")]
            enum Status {
                InProgress,
                #[ts(rename = "done!")]
                Completed,
            }
        })
        .expect("kebab-case enum rename_all expands");
        assert!(kebab.contains(r#""in-progress""#), "{kebab}");
        assert!(kebab.contains(r#""done!""#), "{kebab}");

        let verbatim = expand(&parse_quote! {
            #[ts(rename_all = "verbatim")]
            enum Align {
                BottomTrailing,
            }
        })
        .expect("verbatim rename_all expands");
        assert!(verbatim.contains(r#""BottomTrailing""#), "{verbatim}");
    }

    #[test]
    fn unknown_rename_all_or_invalid_struct_kebab_is_refused() {
        let error = expand(&parse_quote! {
            #[ts(rename_all = "spongebob")]
            struct Props {
                item_count: u32,
            }
        })
        .expect_err("unknown rename_all must fail");
        assert!(error.to_string().contains("spongebob"), "{error}");

        let error = expand(&parse_quote! {
            #[ts(rename_all = "kebab-case")]
            struct Props {
                item_count: u32,
            }
        })
        .expect_err("kebab-case struct field is not a JS identifier");
        assert!(error.to_string().contains("item-count"), "{error}");
    }

    #[test]
    fn a_rename_reaches_the_schema() {
        let expansion = expand(&parse_quote! {
            struct ButtonAttributes {
                #[ts(rename = "onPress")]
                on_tap: u32,
            }
        })
        .expect("a valid rename expands");
        assert!(expansion.contains(r#""onPress""#), "{expansion}");
        assert!(!expansion.contains(r#""onTap""#), "{expansion}");
    }

    #[test]
    fn a_rename_that_is_not_an_identifier_is_refused() {
        let error = expand(&parse_quote! {
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
        expand(&parse_quote! {
            struct Attributes {
                #[ts(rename = "1st")]
                first: u32,
            }
        })
        .expect_err("a property name cannot start with a digit");
    }

    #[test]
    fn two_fields_projecting_to_one_property_are_refused() {
        let error = expand(&parse_quote! {
            struct Attributes {
                on_tap: u32,
                #[ts(rename = "onTap")]
                tapped: u32,
            }
        })
        .expect_err("one property name is one field");
        let message = error.to_string();
        assert!(message.contains("onTap"), "{message}");
        assert!(message.contains("tapped"), "{message}");
    }

    #[test]
    fn two_variants_projecting_to_one_literal_are_refused() {
        let error = expand(&parse_quote! {
            enum Status {
                Ready,
                #[ts(rename = "ready")]
                Done,
            }
        })
        .expect_err("one variant literal is one arm");
        let message = error.to_string();
        assert!(message.contains("ready"), "{message}");
        assert!(message.contains("Done"), "{message}");
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
