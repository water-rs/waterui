extern crate proc_macro;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{quote, ToTokens};
use syn::{self, parse_macro_input, Ident, ItemFn};

#[proc_macro_attribute]
pub fn init(_attr: TokenStream, items: TokenStream) -> TokenStream {
    let f = parse_macro_input!(items as ItemFn);
    let f_name = f.sig.ident.clone();
    let expanded = if f.sig.asyncness.is_some() {
        quote! {
            #f
            #[no_mangle]
            extern "C" fn waterui_init() -> *mut ::waterui_ffi::waterui_env {
                use ::waterui_ffi::IntoFFI;
                ::waterui::future::block_on(#f_name()).into_ffi()
            }
        }
    } else {
        quote! {
            #f
            #[no_mangle]
            extern "C" fn waterui_init() -> *mut ::waterui_ffi::waterui_env {
                use ::waterui_ffi::IntoFFI;
                #f_name().into_ffi()
            }
        }
    };

    TokenStream::from(expanded)
}

#[proc_macro_attribute]
pub fn export(attr: TokenStream, items: TokenStream) -> TokenStream {
    let f = parse_macro_input!(items as ItemFn);
    let f_name = f.sig.ident.clone();
    let ident = if attr.is_empty() {
        f_name.clone()
    } else {
        parse_macro_input!(attr as Ident)
    };

    let new_f_name = Ident::new(&format!("waterui_widget_{ident}"), Span::call_site());

    let expanded = quote! {
        #f

        #[no_mangle]
        extern "C" fn #new_f_name() -> *mut ::waterui_ffi::waterui_anyview {
            ::waterui_ffi::IntoFFI::into_ffi(::waterui::AnyView::new(#f_name()))
        }
    };

    TokenStream::from(expanded)
}

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, items: TokenStream) -> TokenStream {
    export(
        Ident::new("main", Span::call_site())
            .to_token_stream()
            .into(),
        items,
    )
}
