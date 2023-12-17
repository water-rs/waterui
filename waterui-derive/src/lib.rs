use proc_macro::TokenStream;

use quote::{quote, ToTokens};
use syn::{parse, parse::Nothing, Error, ItemImpl, Type};
#[doc(hidden)]
#[proc_macro_attribute]
pub fn view(attribute: TokenStream, item: TokenStream) -> TokenStream {
    match view_inner(attribute, item) {
        Ok(stream) => stream,
        Err(error) => error.into_compile_error().into(),
    }
}

fn view_inner(attribute: TokenStream, input: TokenStream) -> Result<TokenStream, Error> {
    let _: Nothing = parse(attribute)?;
    view_impl(parse(input)?)
}

fn view_impl(mut input: ItemImpl) -> Result<TokenStream, Error> {
    for item in &mut input.items {
        match item {
            syn::ImplItem::Fn(f) => {
                let sig = f.sig.clone();
                let mut return_ty: Type = match sig.output {
                    syn::ReturnType::Default => parse(quote!(()).into())?,
                    syn::ReturnType::Type(_, ty) => *ty,
                };
                if let Type::ImplTrait(_impltrait) = return_ty {
                    return_ty = parse(quote!(_).into())?;
                }
                if sig.ident == "view" {
                    let block = &f.block;
                    f.sig.output = parse(quote!(-> ::waterui::view::BoxView).into())?;
                    f.block = parse(
                        quote! {
                            {let __view:#return_ty=#block;
                            let __check:&dyn ::waterui::view::View=&__view;
                            Box::new(__view)}
                        }
                        .into(),
                    )?;
                }
            }

            _ => todo!(),
        }
    }

    Ok(input.into_token_stream().into())
}
