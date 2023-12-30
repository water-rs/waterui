use proc_macro::TokenStream;
use quote::quote;
use syn::{parse, Error, ItemFn};

#[proc_macro_attribute]
pub fn water_main(_attr: TokenStream, input: TokenStream) -> TokenStream {
    inner(input).unwrap_or_else(|error| error.into_compile_error().into())
}

fn inner(input: TokenStream) -> Result<TokenStream, Error> {
    let f: ItemFn = parse(input)?;
    let ident = f.sig.ident.clone();
    if f.sig.asyncness.is_some() {
        Ok(quote!(
            #f
            #[no_mangle]
            pub extern "C" fn waterui_main() -> ::waterui::ffi::App {
                let app: ::waterui::app::App = ::waterui::__block_on(#ident());
                ::waterui::ffi::App::from(app)
            }
        )
        .into())
    } else {
        Ok(quote!(
            #f
            #[no_mangle]
            pub extern "C" fn waterui_main() -> ::waterui::ffi::App {
                let app: ::waterui::app::App = #ident();
                ::waterui::ffi::App::from(app)
            }
        )
        .into())
    }
}
