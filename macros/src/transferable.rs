use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, GenericParam, parse_quote};

pub fn expand(input: &DeriveInput) -> syn::Result<TokenStream> {
    let root = crate::drag_drop_root_path()?;
    let name = &input.ident;
    let mut generics = input.generics.clone();

    for param in &input.generics.params {
        match param {
            GenericParam::Type(type_param) => {
                let ident = &type_param.ident;
                generics
                    .make_where_clause()
                    .predicates
                    .push(parse_quote!(#ident: ::core::clone::Clone + 'static));
            }
            GenericParam::Lifetime(lifetime_param) => {
                let lifetime = &lifetime_param.lifetime;
                generics
                    .make_where_clause()
                    .predicates
                    .push(parse_quote!(#lifetime: 'static));
            }
            GenericParam::Const(_) => {}
        }
    }

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics #root::drag_drop::Transferable for #name #type_generics #where_clause {}
    })
}

#[cfg(test)]
mod tests {
    use super::expand;

    #[test]
    fn derives_transferable_for_struct_and_enum() {
        let struct_input = syn::parse_quote! {
            struct TabId(u64);
        };
        let struct_expansion = expand(&struct_input)
            .expect("derive should expand for tuple struct")
            .to_string();
        assert!(struct_expansion.contains("drag_drop :: Transferable for TabId"));

        let enum_input = syn::parse_quote! {
            enum Tool {
                Pen,
                Eraser,
            }
        };
        let enum_expansion = expand(&enum_input)
            .expect("derive should expand for enum")
            .to_string();
        assert!(enum_expansion.contains("drag_drop :: Transferable for Tool"));
    }

    #[test]
    fn bounds_generic_parameters_as_clone_static() {
        let input = syn::parse_quote! {
            struct Payload<T>(T);
        };
        let expansion = expand(&input)
            .expect("derive should expand for generic struct")
            .to_string();
        assert!(expansion.contains("T : :: core :: clone :: Clone + 'static"));
        assert!(expansion.contains("drag_drop :: Transferable for Payload < T >"));
    }
}
