/// Impl `View` trait automatically for your custom widget.`
/// ```
/// #[derive(Reactive)]
/// pub struct Home{
///     #[state]
///     list:Vec<String>
/// }
use proc_macro::TokenStream;

use proc_macro2::Span;
use quote::{quote, ToTokens};
use syn::{parse, parse::Nothing, Error, Ident, ItemImpl, ItemStruct, Meta, Type};
#[doc(hidden)]
#[proc_macro_attribute]
pub fn view(attribute: TokenStream, item: TokenStream) -> TokenStream {
    match widget_inner(attribute, item) {
        Ok(stream) => stream,
        Err(error) => error.into_compile_error().into(),
    }
}

fn widget_inner(attribute: TokenStream, input: TokenStream) -> Result<TokenStream, Error> {
    let mut root = Ident::new("waterui", Span::call_site());
    if let Ok(meta) = parse(attribute.clone()) {
        if let Meta::Path(path) = meta {
            if path.get_ident().expect("Must be ident") == "use_core" {
                root = Ident::new("waterui_core", Span::call_site());
            }
        } else {
            return Err(Error::new(Span::call_site(), "Unexpected input"));
        }
    } else {
        let _: Nothing = parse(attribute)?;
    }
    if let Ok(input) = parse(input.clone()) {
        return widget_struct(input, root);
    }

    widget_impl(parse(input)?, root)
}

fn widget_impl(mut input: ItemImpl, root: Ident) -> Result<TokenStream, Error> {
    let mut stream = TokenStream::new();
    let generics = input.generics.clone();
    let (_impl_generics, _ty_generics, _where_clause) = generics.split_for_impl();

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
                    f.sig.output = parse(quote!(-> ::#root::view::BoxView).into())?;
                    f.block = parse(
                        quote! {
                            {let __view:#return_ty=#block;
                            let __check:&dyn ::#root::view::View=&__view;
                            Box::new(__view)}
                        }
                        .into(),
                    )?;
                }
            }

            _ => todo!(),
        }
    }

    stream.extend::<TokenStream>(input.into_token_stream().into());

    Ok(stream)
}

fn widget_struct(mut input: ItemStruct, root: Ident) -> Result<TokenStream, Error> {
    let mut state_field = Vec::new();
    let struct_name = input.ident.clone();
    let generics = input.generics.clone();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let mut output = TokenStream::new();

    for field in input.fields.iter_mut() {
        let state = field
            .attrs
            .iter()
            .filter(|attribute| {
                if let Some(name) = attribute.meta.path().get_ident() {
                    let state = Ident::new("state", Span::call_site());
                    *name == state
                } else {
                    false
                }
            })
            .enumerate()
            .next();
        if let Some((index, _)) = state {
            field.attrs.remove(index);
            let ty = field.ty.clone();
            field.ty = parse(quote!(::#root::binding::Binding<#ty>).into())?;
            state_field.push(field.ident.clone().unwrap())
        }
    }

    output.extend::<TokenStream>(
        quote! {
            #input
            impl #impl_generics ::#root::view::Reactive for #struct_name #ty_generics #where_clause{
                fn is_reactive(&self) -> bool {
                    true
                }

                fn subscribe(&self, subscriber: fn() -> ::#root::binding::BoxSubscriber) {
                    #(self.#state_field.add_boxed_subscriber((subscriber)()));*
                }
            }
        }
        .into(),
    );

    Ok(output)
}
