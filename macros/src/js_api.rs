//! `#[js_api]`: turns an `impl` block into the surface a page can call.
//!
//! Handlers and exposed state can both be written by hand — `.handler(name, …)`
//! and `.expose(name, …)` — but doing it that way means repeating each name as a
//! string, which nothing checks. Types cannot fix that on their own: a
//! type-level version needs a unit struct and a trait impl per command, all of
//! it restating names the method already has. That is the shape of problem a
//! macro is for.
//!
//! What it generates is only what could have been written by hand, so the manual
//! path stays equivalent and the macro adds no capability of its own. TypeScript
//! declarations are derived from the compiler-resolved `TsType::SCHEMA` and
//! `JsReplySchema::SCHEMA` constants rather than from type spellings.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt as _;
use syn::parse::{Parse, ParseStream};
use syn::{
    Attribute, Error, Expr, FnArg, Ident, ImplItem, ImplItemFn, ItemImpl, Lit, LitStr, Meta, Pat,
    ReturnType, Type,
};

/// Parsed `#[js_api(namespace = "app")]` arguments.
pub struct Args {
    namespace: Option<LitStr>,
}

impl Parse for Args {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self { namespace: None });
        }
        let meta: Meta = input.parse()?;
        let namespace = match &meta {
            Meta::NameValue(pair) if pair.path.is_ident("namespace") => match &pair.value {
                Expr::Lit(literal) => match &literal.lit {
                    Lit::Str(value) if value.value().is_empty() => {
                        return Err(Error::new(value.span(), "`namespace` cannot be empty"));
                    }
                    Lit::Str(value) => Some(value.clone()),
                    other => return Err(Error::new_spanned(other, "expected a string")),
                },
                other => return Err(Error::new_spanned(other, "expected a string")),
            },
            other => {
                return Err(Error::new_spanned(other, "expected `namespace = \"...\"`"));
            }
        };
        Ok(Self { namespace })
    }
}

/// Parsed `#[js(...)]` attributes on one method.
#[derive(Default)]
struct MethodAttrs {
    skip: bool,
    rename: Option<LitStr>,
}

impl MethodAttrs {
    fn parse(attrs: &[Attribute]) -> syn::Result<Self> {
        let mut parsed = Self::default();
        for attr in attrs {
            if !attr.path().is_ident("js") {
                continue;
            }
            let mut saw_item = false;
            attr.parse_nested_meta(|meta| {
                saw_item = true;
                if meta.path.is_ident("skip") {
                    if parsed.skip {
                        return Err(meta.error("duplicate `#[js(skip)]` attribute"));
                    }
                    parsed.skip = true;
                    return Ok(());
                }
                if meta.path.is_ident("rename") {
                    if parsed.rename.is_some() {
                        return Err(meta.error("duplicate `#[js(rename = \"...\")]` attribute"));
                    }
                    let value: LitStr = meta.value()?.parse()?;
                    if value.value().is_empty() {
                        return Err(Error::new(
                            value.span(),
                            "`#[js(rename = \"...\")]` cannot be empty",
                        ));
                    }
                    parsed.rename = Some(value);
                    return Ok(());
                }
                Err(meta
                    .error("unknown `#[js(...)]` attribute; expected `skip` or `rename = \"...\"`"))
            })?;
            if !saw_item {
                return Err(Error::new_spanned(
                    attr,
                    "`#[js(...)]` requires `skip` or `rename = \"...\"`",
                ));
            }
        }
        if parsed.skip
            && let Some(rename) = &parsed.rename
        {
            return Err(Error::new(
                rename.span(),
                "`#[js(skip)]` and `#[js(rename = \"...\")]` cannot be combined",
            ));
        }
        Ok(parsed)
    }
}

/// How one method is exposed to the page.
#[derive(Debug)]
enum Exposure {
    /// `async fn(&self, ..)` — the page calls it and awaits a reply.
    Method,
    /// `fn(&self) -> impl JsField` — mirrored state.
    State,
    /// `#[js(skip)]`, or not part of the surface.
    Skipped,
}

/// Decides how a method is exposed, from its shape alone.
///
/// The distinction is syntactic on purpose: a macro cannot see types, and
/// matching on the spelling of a return type would break the moment someone used
/// a type alias or a qualified path. `async` and the parameter list are facts the
/// tokens actually carry.
fn classify(method: &ImplItemFn, attrs: &MethodAttrs) -> syn::Result<Exposure> {
    if attrs.skip {
        return Ok(Exposure::Skipped);
    }
    let takes_self = matches!(method.sig.inputs.first(), Some(FnArg::Receiver(_)));
    if !takes_self {
        if let Some(rename) = &attrs.rename {
            return Err(Error::new(
                rename.span(),
                "`#[js(rename = \"...\")]` cannot be applied to an associated function without `self`",
            ));
        }
        return Ok(Exposure::Skipped);
    }
    if !method.sig.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &method.sig,
            "a generic method cannot be exposed: the page can only call one concrete signature",
        ));
    }
    let extra_arguments = method.sig.inputs.len() - 1;
    if method.sig.asyncness.is_some() {
        return Ok(Exposure::Method);
    }
    if extra_arguments == 0 && matches!(method.sig.output, ReturnType::Type(..)) {
        return Ok(Exposure::State);
    }
    Err(Error::new_spanned(
        &method.sig,
        "a synchronous method with arguments cannot be exposed: make it `async fn`, \
         or add #[js(skip)] if it is internal",
    ))
}

/// The name the page sees, honouring `#[js(rename = "...")]` and any namespace.
fn exposed_name(method: &ImplItemFn, attrs: &MethodAttrs, namespace: Option<&LitStr>) -> String {
    let base = attrs
        .rename
        .as_ref()
        .map_or_else(|| method.sig.ident.unraw().to_string(), LitStr::value);
    match namespace {
        Some(namespace) => format!("{}.{base}", namespace.value()),
        None => base,
    }
}

/// One typed handler argument extracted from a method signature.
struct MethodParam {
    ident: Ident,
    name: String,
    ty: Box<Type>,
}

fn method_params(method: &ImplItemFn) -> syn::Result<Vec<MethodParam>> {
    method
        .sig
        .inputs
        .iter()
        .skip(1)
        .map(|argument| match argument {
            FnArg::Typed(typed) => {
                let Pat::Ident(pat_ident) = typed.pat.as_ref() else {
                    return Err(Error::new_spanned(
                        &typed.pat,
                        "a `#[js_api]` handler parameter must be a simple identifier",
                    ));
                };
                Ok(MethodParam {
                    name: pat_ident.ident.unraw().to_string(),
                    ident: pat_ident.ident.clone(),
                    ty: typed.ty.clone(),
                })
            }
            FnArg::Receiver(receiver) => Err(Error::new_spanned(
                receiver,
                "a second `self` argument is not valid",
            )),
        })
        .collect()
}

/// Rejects `#[js(...)]` on non-method impl items where it would otherwise be ignored.
fn reject_non_method_js_attrs(item: &ImplItem) -> syn::Result<()> {
    let attrs = match item {
        ImplItem::Const(item) => &item.attrs,
        ImplItem::Type(item) => &item.attrs,
        ImplItem::Macro(item) => &item.attrs,
        _ => return Ok(()),
    };
    for attr in attrs {
        if attr.path().is_ident("js") {
            return Err(Error::new_spanned(
                attr,
                "`#[js(...)]` is only valid on methods inside `#[js_api]`",
            ));
        }
    }
    Ok(())
}

/// Strips the `#[js(...)]` attributes so the emitted impl block still compiles.
fn strip_js_attributes(block: &mut ItemImpl) {
    for item in &mut block.items {
        if let ImplItem::Fn(method) = item {
            method
                .attrs
                .retain(|attribute| !attribute.path().is_ident("js"));
        }
    }
}

fn expand_handler(method: &ImplItemFn, name: &str) -> syn::Result<(TokenStream, TokenStream)> {
    let ident = &method.sig.ident;
    let params = method_params(method)?;
    let return_ty: TokenStream = match &method.sig.output {
        ReturnType::Default => quote!(()),
        ReturnType::Type(_, ty) => quote!(#ty),
    };
    let param_schemas: Vec<_> = params
        .iter()
        .map(|param| {
            let param_name = &param.name;
            let param_ty = &param.ty;
            quote! {
                ::waterui::webview::JsApiParam::new(
                    #param_name,
                    <#param_ty as ::waterui::webview::TsType>::SCHEMA,
                )
            }
        })
        .collect();
    let schema_entry = quote! {
        ::waterui::webview::JsApiMethod::new(
            #name,
            &[#(#param_schemas),*],
            <#return_ty as ::waterui::webview::JsReplySchema>::SCHEMA,
        )
    };

    if params.is_empty() {
        // No arguments means nothing to deserialize, so the page can
        // call `await app.reset()` without inventing an empty object
        // to send.
        let registration = quote! {
            {
                let api = ::std::rc::Rc::clone(&api);
                builder = builder.handler(#name, move || {
                    let api = ::std::rc::Rc::clone(&api);
                    async move { api.#ident().await }
                });
            }
        };
        return Ok((schema_entry, registration));
    }

    let argument_struct = format_ident!("__WateruiArgs{}", ident);
    let field_idents: Vec<_> = params.iter().map(|param| &param.ident).collect();
    let field_types: Vec<_> = params.iter().map(|param| &param.ty).collect();
    let registration = quote! {
        {
            // One struct per method so the page sends a named object
            // rather than a positional tuple, which reads the same way
            // the Rust signature does.
            #[derive(::waterui::__private::serde::Deserialize)]
            #[serde(crate = "::waterui::__private::serde")]
            #[allow(non_camel_case_types)]
            struct #argument_struct {
                #(#field_idents: #field_types,)*
            }

            let api = ::std::rc::Rc::clone(&api);
            builder = builder.handler(
                #name,
                move |::waterui::webview::Json(arguments): ::waterui::webview::Json<#argument_struct>| {
                    let api = ::std::rc::Rc::clone(&api);
                    async move { api.#ident(#(arguments.#field_idents),*).await }
                },
            );
        }
    };
    Ok((schema_entry, registration))
}

/// Expands `#[js_api]`.
///
/// # Errors
///
/// Returns an error for a method whose shape cannot be exposed or an invalid
/// `#[js(...)]` attribute.
pub fn expand(args: &Args, mut block: ItemImpl) -> syn::Result<TokenStream> {
    let self_ty = block.self_ty.clone();
    let mut registrations = Vec::new();
    let mut state_entries = Vec::new();
    let mut method_entries = Vec::new();

    for item in &block.items {
        let ImplItem::Fn(method) = item else {
            reject_non_method_js_attrs(item)?;
            continue;
        };
        let attrs = MethodAttrs::parse(&method.attrs)?;
        let exposure = classify(method, &attrs)?;
        let name = exposed_name(method, &attrs, args.namespace.as_ref());
        let ident = &method.sig.ident;

        match exposure {
            Exposure::Skipped => {}
            Exposure::State => {
                let ReturnType::Type(_, return_ty) = &method.sig.output else {
                    return Err(Error::new_spanned(
                        &method.sig,
                        "mirrored state must declare a return type",
                    ));
                };
                state_entries.push(quote! {
                    ::waterui::webview::JsApiStateField::new(
                        #name,
                        <#return_ty as ::waterui::webview::TsType>::SCHEMA,
                    )
                });
                registrations.push(quote! {
                    builder = builder.expose(#name, ::std::rc::Rc::clone(&api).#ident());
                });
            }
            Exposure::Method => {
                let (schema_entry, registration) = expand_handler(method, &name)?;
                method_entries.push(schema_entry);
                registrations.push(registration);
            }
        }
    }

    strip_js_attributes(&mut block);

    Ok(quote! {
        #block

        impl ::waterui::webview::JsApi for #self_ty {
            fn register(
                api: ::std::rc::Rc<Self>,
                mut builder: ::waterui::webview::WebViewOpen,
            ) -> ::waterui::webview::WebViewOpen {
                #(#registrations)*
                builder
            }

            fn schema() -> &'static ::waterui::webview::JsApiSchema {
                const SCHEMA: ::waterui::webview::JsApiSchema = ::waterui::webview::JsApiSchema::new(
                    &[#(#state_entries),*],
                    &[#(#method_entries),*],
                );
                &SCHEMA
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{Args, Exposure, MethodAttrs, classify, expand, exposed_name};
    use proc_macro2::Span;
    use syn::{ImplItem, ImplItemFn, ItemImpl, LitStr, parse_quote, parse_str};

    fn methods(block: ItemImpl) -> Vec<ImplItemFn> {
        block
            .items
            .into_iter()
            .filter_map(|item| match item {
                ImplItem::Fn(method) => Some(method),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shape_alone_decides_how_a_method_is_exposed() {
        let block: ItemImpl = parse_quote! {
            impl Api {
                async fn greet(&self, name: String) -> Json<Greeting> { todo!() }
                fn theme(&self) -> Binding<String> { todo!() }
                #[js(skip)]
                fn internal(&self, factor: u32) -> u32 { todo!() }
                fn helper(value: u32) -> u32 { todo!() }
            }
        };
        let classified: Vec<_> = methods(block)
            .iter()
            .map(|method| {
                let attrs = MethodAttrs::parse(&method.attrs).expect("valid attrs");
                classify(method, &attrs).expect("classifies")
            })
            .collect();
        assert!(matches!(classified[0], Exposure::Method));
        assert!(matches!(classified[1], Exposure::State));
        // `#[js(skip)]`, and an associated function with no receiver, are both
        // simply not part of the surface.
        assert!(matches!(classified[2], Exposure::Skipped));
        assert!(matches!(classified[3], Exposure::Skipped));
    }

    #[test]
    fn a_synchronous_method_with_arguments_is_an_error() {
        let block: ItemImpl = parse_quote! {
            impl Api {
                fn add(&self, by: u32) -> u32 { todo!() }
            }
        };
        let method = &methods(block)[0];
        let attrs = MethodAttrs::parse(&method.attrs).expect("valid attrs");
        let error = classify(method, &attrs).expect_err("cannot be exposed");
        assert!(
            str::contains(&error.to_string(), "async fn"),
            "the error should say what to do: {error}"
        );
    }

    #[test]
    fn a_generic_method_is_an_error() {
        let block: ItemImpl = parse_quote! {
            impl Api {
                async fn send<T: Serialize>(&self, value: T) {}
            }
        };
        let method = &methods(block)[0];
        let attrs = MethodAttrs::parse(&method.attrs).expect("valid attrs");
        classify(method, &attrs).expect_err("one concrete signature only");
    }

    #[test]
    fn the_namespace_and_rename_shape_the_name_the_page_sees() {
        let block: ItemImpl = parse_quote! {
            impl Api {
                async fn greet(&self) {}
                #[js(rename = "reset")]
                async fn reset_counter(&self) {}
            }
        };
        let namespace = LitStr::new("app", Span::call_site());
        let methods = methods(block);
        let first_attrs = MethodAttrs::parse(&methods[0].attrs).expect("valid attrs");
        let second_attrs = MethodAttrs::parse(&methods[1].attrs).expect("valid attrs");
        assert_eq!(exposed_name(&methods[0], &first_attrs, None), "greet");
        assert_eq!(
            exposed_name(&methods[0], &first_attrs, Some(&namespace)),
            "app.greet"
        );
        assert_eq!(
            exposed_name(&methods[1], &second_attrs, Some(&namespace)),
            "app.reset"
        );
    }

    #[test]
    fn unknown_and_malformed_js_attributes_are_rejected() {
        let misspelled: ItemImpl = parse_quote! {
            impl Api {
                #[js(rneame = "x")]
                async fn ping(&self) {}
            }
        };
        let error = expand(&Args { namespace: None }, misspelled)
            .expect_err("misspelled #[js(...)] must fail");
        assert!(error.to_string().contains("unknown"), "{error}");

        let empty: ItemImpl = parse_quote! {
            impl Api {
                #[js()]
                async fn ping(&self) {}
            }
        };
        expand(&Args { namespace: None }, empty).expect_err("empty #[js()] must fail");

        let combined: ItemImpl = parse_quote! {
            impl Api {
                #[js(skip, rename = "x")]
                async fn ping(&self) {}
            }
        };
        expand(&Args { namespace: None }, combined)
            .expect_err("skip + rename combination must fail");

        let duplicate: ItemImpl = parse_quote! {
            impl Api {
                #[js(rename = "a")]
                #[js(rename = "b")]
                async fn ping(&self) {}
            }
        };
        expand(&Args { namespace: None }, duplicate).expect_err("duplicate rename must fail");

        let empty_rename: ItemImpl = parse_quote! {
            impl Api {
                #[js(rename = "")]
                async fn ping(&self) {}
            }
        };
        expand(&Args { namespace: None }, empty_rename).expect_err("empty rename must fail");

        assert!(
            parse_str::<Args>(r#"namespace = """#).is_err(),
            "empty namespace must fail"
        );
    }

    #[test]
    fn expansion_emits_schema_bounds_instead_of_type_spellings() {
        let block: ItemImpl = parse_quote! {
            impl Api {
                fn theme(&self) -> ThemeSignal { todo!() }
                async fn lookup(&self, id: Id) -> Json<Greeting> { todo!() }
            }
        };
        let expanded = expand(&Args { namespace: None }, block)
            .expect("valid impl expands")
            .to_string();
        assert!(
            expanded.contains("< ThemeSignal as :: waterui :: webview :: TsType > :: SCHEMA"),
            "{expanded}"
        );
        assert!(
            expanded.contains("< Id as :: waterui :: webview :: TsType > :: SCHEMA"),
            "{expanded}"
        );
        assert!(
            expanded.contains(
                "< Json < Greeting > as :: waterui :: webview :: JsReplySchema > :: SCHEMA"
            ),
            "{expanded}"
        );
    }
}
