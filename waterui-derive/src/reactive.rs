use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;

use syn::{parse, Error, Ident, ItemStruct};

pub fn impl_reactive(mut input: ItemStruct, root: Ident) -> Result<TokenStream, Error> {
    let mut state_field = Vec::new();
    let struct_name = input.ident.clone();
    let generics = input.generics.clone();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let mut output = TokenStream::new();
    let mut reactive = false;
    for field in input.fields.iter_mut() {
        let state = field
            .attrs
            .iter()
            .filter(|attribute| {
                if let Some(name) = attribute.meta.path().get_ident() {
                    let state = Ident::new("state", Span::call_site());
                    reactive = true;
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
                    #reactive
                }

                fn subscribe(&self, subscriber: extern "C" fn() -> ::#root::binding::SubscriberObject) {
                    #(self.#state_field.add_subscriber((subscriber)()));*
                }
            }
        }
        .into(),
    );

    Ok(output)
}
