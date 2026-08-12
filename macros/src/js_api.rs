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
use quote::ToTokens as _;
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

/// How a Rust type is spelled in TypeScript.
///
/// Only the types this boundary actually deals in are named; anything else
/// becomes `unknown`, which is honest — a wrong declaration is worse than an
/// unspecific one, because TypeScript will believe it.
///
/// `u64` and `i64` are `number | bigint` rather than `number`: a value past
/// 2^53 arrives as a `BigInt`, and a declaration saying otherwise would send
/// page authors looking for a bug in their own arithmetic.
fn typescript_type(ty: &syn::Type) -> String {
    let syn::Type::Path(path) = ty else {
        return match ty {
            syn::Type::Tuple(tuple) if tuple.elems.is_empty() => "void".to_owned(),
            syn::Type::Reference(reference) => typescript_type(&reference.elem),
            _ => "unknown".to_owned(),
        };
    };
    let Some(segment) = path.path.segments.last() else {
        return "unknown".to_owned();
    };

    let parameter = |index: usize| {
        let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return None;
        };
        arguments
            .args
            .iter()
            .nth(index)
            .and_then(|argument| match argument {
                syn::GenericArgument::Type(inner) => Some(inner),
                _ => None,
            })
    };

    match segment.ident.to_string().as_str() {
        "String" | "Str" | "str" | "char" | "PathBuf" | "Path" => "string".to_owned(),
        "bool" => "boolean".to_owned(),
        "u8" | "u16" | "u32" | "i8" | "i16" | "i32" | "f32" | "f64" | "usize" | "isize" => {
            "number".to_owned()
        }
        "u64" | "i64" | "u128" | "i128" => "number | bigint".to_owned(),
        // Bytes cross as a `Uint8Array`, whichever wrapper carries them.
        "Bytes" => "Uint8Array".to_owned(),
        "Vec" | "VecDeque" | "HashSet" | "BTreeSet" => match parameter(0) {
            Some(inner) if typescript_type(inner) == "number" && is_u8(inner) => {
                "Uint8Array".to_owned()
            }
            Some(inner) => format!("{}[]", typescript_type(inner)),
            None => "unknown[]".to_owned(),
        },
        "Option" => parameter(0).map_or_else(
            || "unknown | null".to_owned(),
            |inner| format!("{} | null", typescript_type(inner)),
        ),
        "HashMap" | "BTreeMap" => match (parameter(0), parameter(1)) {
            (Some(key), Some(value)) => format!(
                "Record<{}, {}>",
                typescript_type(key),
                typescript_type(value)
            ),
            _ => "Record<string, unknown>".to_owned(),
        },
        // WaterUI's own wrappers are transparent on the wire.
        "Json" | "Result" | "Binding" | "Computed" => {
            parameter(0).map_or_else(|| "unknown".to_owned(), typescript_type)
        }
        _ => "unknown".to_owned(),
    }
}

fn is_u8(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(path)
        if path.path.segments.last().is_some_and(|last| last.ident == "u8"))
}

/// The TypeScript for one exposed method, and which member it belongs under.
///
/// The two groups are not interchangeable: a handler is reached through
/// `waterui.invoke(name, …)` while mirrored state is a property of
/// `waterui.state`, so declaring them side by side would describe an object the
/// page does not have.
fn declaration(method: &ImplItemFn, name: &str, exposure: &Exposure) -> Option<(bool, String)> {
    let returns = match &method.sig.output {
        ReturnType::Default => "void".to_owned(),
        ReturnType::Type(_, ty) => typescript_type(ty),
    };

    match exposure {
        Exposure::Skipped => None,
        Exposure::State => {
            // Only a `Binding` is assignable; everything else is derived in Rust
            // and throws on assignment, which `readonly` is exactly how to say.
            let writable = method
                .sig
                .output
                .to_token_stream()
                .to_string()
                .contains("Binding");
            let prefix = if writable { "" } else { "readonly " };
            Some((true, format!("    {prefix}{name:?}: {returns};")))
        }
        Exposure::Method => {
            let arguments: Vec<String> = method
                .sig
                .inputs
                .iter()
                .skip(1)
                .filter_map(|argument| match argument {
                    FnArg::Typed(typed) => Some(format!(
                        "{}: {}",
                        typed.pat.to_token_stream(),
                        typescript_type(&typed.ty)
                    )),
                    FnArg::Receiver(_) => None,
                })
                .collect();
            let parameters = if arguments.is_empty() {
                String::new()
            } else {
                format!("args: {{ {} }}", arguments.join("; "))
            };
            Some((
                false,
                format!(
                    "  invoke(name: {name:?}{}): Promise<{returns}>;",
                    if parameters.is_empty() {
                        String::new()
                    } else {
                        format!(", {parameters}")
                    }
                ),
            ))
        }
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
    let mut state_members = Vec::new();
    let mut call_members = Vec::new();

    for item in &block.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };
        let exposure = classify(method)?;
        let name = exposed_name(method, args.namespace.as_ref());
        let ident = &method.sig.ident;
        if let Some((is_state, line)) = declaration(method, &name, &exposure) {
            if is_state {
                state_members.push(line);
            } else {
                call_members.push(line);
            }
        }

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

    // `state` is always declared, even when empty: the page can still reach it,
    // and an interface that omits it would make `waterui.state` a type error.
    let typescript = format!(
        "  state: {{\n{}\n  }};\n{}",
        state_members.join("\n"),
        call_members.join("\n")
    );

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

            fn typescript() -> &'static str {
                #typescript
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
