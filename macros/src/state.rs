use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::Nothing;
use syn::{Data, DeriveInput, Item};

use crate::view_builder::dependency_path;

pub fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    if let Err(error) = syn::parse::<Nothing>(args) {
        return error.to_compile_error().into();
    }

    let tokens = TokenStream2::from(input);
    let input = match syn::parse2::<DeriveInput>(tokens.clone()) {
        Ok(input) => input,
        Err(error) => {
            // A well-formed item that is not a struct or enum gets a named
            // diagnostic; otherwise surface the real parse error.
            return syn::parse2::<Item>(tokens).map_or_else(
                |_| error.to_compile_error().into(),
                |item| {
                    syn::Error::new_spanned(&item, "#[state] applies to a struct or enum")
                        .to_compile_error()
                        .into()
                },
            );
        }
    };

    if let Data::Union(union) = &input.data {
        return syn::Error::new_spanned(union.union_token, "#[state] does not support unions")
            .to_compile_error()
            .into();
    }

    let waterui = match crate_path() {
        Ok(path) => path,
        Err(error) => return error.to_compile_error().into(),
    };

    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    quote! {
        #input

        impl #impl_generics #waterui::extract::Extractor for #name #ty_generics #where_clause {
            fn extract(
                env: &#waterui::Environment,
            ) -> ::core::result::Result<Self, #waterui::Error> {
                // The assertion names `Self` rather than leaning on the
                // `State<Self>: Extractor` bound below so that a missing
                // `Clone` reports here, against the attribute, with a
                // `#[derive(Clone)]` suggestion.
                fn assert_state_bounds<T: Clone + 'static>() {}
                assert_state_bounds::<Self>();

                <#waterui::extract::State<Self> as #waterui::extract::Extractor>::extract(env)
                    .map(|state| state.0)
            }

            fn extract_from_action(
                env: &#waterui::Environment,
                state: &mut #waterui::extract::ExtractionState,
            ) -> ::core::result::Result<Self, #waterui::Error> {
                <#waterui::extract::State<Self> as #waterui::extract::Extractor>::extract_from_action(
                    env, state,
                )
                .map(|state| state.0)
            }
        }
    }
    .into()
}

/// `#[state]` expands against `Extractor`/`State`/`Environment`, which the
/// `waterui` facade and `waterui-core` both expose at the same paths. In-tree
/// component crates do not depend on the facade, so it falls back to
/// `waterui-core`.
fn crate_path() -> Result<TokenStream2, syn::Error> {
    dependency_path("waterui")
        .or_else(|| dependency_path("waterui-core"))
        .ok_or_else(|| {
            syn::Error::new(
                Span::call_site(),
                "#[state] requires the `waterui` or `waterui-core` crate as a dependency",
            )
        })
}
