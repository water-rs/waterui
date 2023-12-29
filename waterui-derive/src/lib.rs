use proc_macro::TokenStream;

use quote::{quote, ToTokens};
use syn::{parse, parse::Nothing, punctuated::Punctuated, Error, FnArg, ItemImpl, Type};
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

fn arg_is_self(arg: &FnArg) -> bool {
    if let FnArg::Receiver(receiver) = arg {
        if *receiver.ty == parse::<Type>(quote!(Self).into()).unwrap() {
            return true;
        }
    }
    false
}

fn view_impl(mut input: ItemImpl) -> Result<TokenStream, Error> {
    for item in &mut input.items {
        match item {
            syn::ImplItem::Fn(f) => {
                let mut return_ty: Type = match f.sig.output.clone() {
                    syn::ReturnType::Default => parse(quote!(()).into())?,
                    syn::ReturnType::Type(_, ty) => *ty,
                };
                if let Type::ImplTrait(_impltrait) = return_ty {
                    return_ty = parse(quote!(_).into())?;
                }

                if f.sig.ident == "body" {
                    let block = &f.block;
                    let inputs: Vec<_> = f.sig.inputs.iter().collect();

                    if inputs.is_empty() || arg_is_self(inputs[0]) {
                        let mut new_inputs = Punctuated::new();
                        new_inputs.push(parse(quote!(self).into())?);
                        new_inputs.push(parse(quote!(_env:waterui::env::Environment).into())?);
                        f.sig.inputs = new_inputs;
                    }

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
