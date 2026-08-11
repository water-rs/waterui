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
//! path stays equivalent and the macro adds no capability of its own.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ImplItemFn, ItemImpl, LitStr, Meta, ReturnType};

/// Parsed `#[js_api(namespace = "app")]` arguments.
pub struct Args {
    namespace: Option<LitStr>,
}

impl syn::parse::Parse for Args {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self { namespace: None });
        }
        let meta: Meta = input.parse()?;
        let namespace = match &meta {
            Meta::NameValue(pair) if pair.path.is_ident("namespace") => match &pair.value {
                syn::Expr::Lit(literal) => match &literal.lit {
                    syn::Lit::Str(value) => Some(value.clone()),
                    other => return Err(syn::Error::new_spanned(other, "expected a string")),
                },
                other => return Err(syn::Error::new_spanned(other, "expected a string")),
            },
            other => {
                return Err(syn::Error::new_spanned(
                    other,
                    "expected `namespace = \"...\"`",
                ));
            }
        };
        Ok(Self { namespace })
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
fn classify(method: &ImplItemFn) -> Result<Exposure, syn::Error> {
    if method.attrs.iter().any(is_skip) {
        return Ok(Exposure::Skipped);
    }
    let takes_self = matches!(method.sig.inputs.first(), Some(FnArg::Receiver(_)));
    if !takes_self {
        return Ok(Exposure::Skipped);
    }
    if !method.sig.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
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
    Err(syn::Error::new_spanned(
        &method.sig,
        "a synchronous method with arguments cannot be exposed: make it `async fn`, \
         or add #[js(skip)] if it is internal",
    ))
}

fn is_skip(attribute: &syn::Attribute) -> bool {
    attribute.path().is_ident("js")
        && attribute
            .parse_args::<syn::Ident>()
            .is_ok_and(|ident| ident == "skip")
}

/// The name the page sees, honouring `#[js(rename = "...")]`.
fn exposed_name(method: &ImplItemFn, namespace: Option<&LitStr>) -> String {
    let renamed = method.attrs.iter().find_map(|attribute| {
        if !attribute.path().is_ident("js") {
            return None;
        }
        attribute
            .parse_args::<syn::MetaNameValue>()
            .ok()
            .filter(|pair| pair.path.is_ident("rename"))
            .and_then(|pair| match pair.value {
                syn::Expr::Lit(literal) => match literal.lit {
                    syn::Lit::Str(value) => Some(value.value()),
                    _ => None,
                },
                _ => None,
            })
    });
    let base = renamed.unwrap_or_else(|| method.sig.ident.to_string());
    match namespace {
        Some(namespace) => format!("{}.{base}", namespace.value()),
        None => base,
    }
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

/// Expands `#[js_api]`.
///
/// # Errors
///
/// Returns an error for a method whose shape cannot be exposed.
pub fn expand(args: &Args, mut block: ItemImpl) -> Result<TokenStream, syn::Error> {
    let self_ty = block.self_ty.clone();
    let mut registrations = Vec::new();

    for item in &block.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        let exposure = classify(method)?;
        let name = exposed_name(method, args.namespace.as_ref());
        let ident = &method.sig.ident;

        match exposure {
            Exposure::Skipped => {}
            Exposure::State => registrations.push(quote! {
                builder = builder.expose(#name, ::std::rc::Rc::clone(&api).#ident());
            }),
            Exposure::Method => {
                let arguments: Vec<_> = method
                    .sig
                    .inputs
                    .iter()
                    .skip(1)
                    .map(|argument| match argument {
                        FnArg::Typed(typed) => Ok(typed.clone()),
                        FnArg::Receiver(receiver) => Err(syn::Error::new_spanned(
                            receiver,
                            "a second `self` argument is not valid",
                        )),
                    })
                    .collect::<Result<_, _>>()?;
                if arguments.is_empty() {
                    // No arguments means nothing to deserialize, so the page can
                    // call `await app.reset()` without inventing an empty object
                    // to send.
                    registrations.push(quote! {
                        {
                            let api = ::std::rc::Rc::clone(&api);
                            builder = builder.handler(#name, move || {
                                let api = ::std::rc::Rc::clone(&api);
                                async move { api.#ident().await }
                            });
                        }
                    });
                    continue;
                }

                let argument_struct = format_ident!("__WateruiArgs{}", ident);
                let names: Vec<_> = arguments
                    .iter()
                    .map(|argument| argument.pat.clone())
                    .collect();
                let types: Vec<_> = arguments
                    .iter()
                    .map(|argument| argument.ty.clone())
                    .collect();

                registrations.push(quote! {
                    {
                        // One struct per method so the page sends a named object
                        // rather than a positional tuple, which reads the same way
                        // the Rust signature does.
                        #[derive(::waterui::webview::serde::Deserialize)]
                        #[serde(crate = "::waterui::webview::serde")]
                        #[allow(non_camel_case_types)]
                        struct #argument_struct {
                            #(#names: #types,)*
                        }

                        let api = ::std::rc::Rc::clone(&api);
                        builder = builder.handler(
                            #name,
                            move |::waterui::webview::Json(arguments): ::waterui::webview::Json<#argument_struct>| {
                                let api = ::std::rc::Rc::clone(&api);
                                async move { api.#ident(#(arguments.#names),*).await }
                            },
                        );
                    }
                });
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
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{Exposure, classify, exposed_name};
    use syn::{ImplItem, ImplItemFn, ItemImpl, LitStr, parse_quote};

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
            .map(|method| classify(method).expect("classifies"))
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
        let error = classify(&methods(block)[0]).expect_err("cannot be exposed");
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
        classify(&methods(block)[0]).expect_err("one concrete signature only");
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
        let namespace = LitStr::new("app", proc_macro2::Span::call_site());
        let methods = methods(block);
        assert_eq!(exposed_name(&methods[0], None), "greet");
        assert_eq!(exposed_name(&methods[0], Some(&namespace)), "app.greet");
        assert_eq!(exposed_name(&methods[1], Some(&namespace)), "app.reset");
    }
}
