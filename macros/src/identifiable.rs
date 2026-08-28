use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Field, Fields, Member, Meta, Type, spanned::Spanned};

pub fn expand(input: DeriveInput) -> syn::Result<TokenStream> {
    reject_type_level_marker(&input)?;

    let identifier = match &input.data {
        Data::Struct(data) => select_identifier(&data.fields)?,
        Data::Enum(_) | Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "Identifiable can only be derived for structs",
            ));
        }
    };

    let name = &input.ident;
    let member = &identifier.member;
    let identifier_type = identifier.ty;
    let mut generics = input.generics.clone();
    generics
        .make_where_clause()
        .predicates
        .push(syn::parse_quote!(
            #identifier_type: core::hash::Hash + core::cmp::Ord + Clone
        ));
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics Identifiable for #name #type_generics #where_clause {
            type Id = #identifier_type;

            fn id(&self) -> Self::Id {
                self.#member.clone()
            }
        }
    })
}

fn reject_type_level_marker(input: &DeriveInput) -> syn::Result<()> {
    if let Some(attribute) = input
        .attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("id"))
    {
        return Err(syn::Error::new_spanned(
            attribute,
            "place #[id] on the identifier field",
        ));
    }
    Ok(())
}

struct Identifier<'a> {
    member: Member,
    ty: &'a Type,
}

fn select_identifier(fields: &Fields) -> syn::Result<Identifier<'_>> {
    let field_members = fields_with_members(fields);
    if field_members.is_empty() {
        return Err(syn::Error::new(
            fields_span(field_members.as_slice()),
            "Identifiable cannot be derived for a unit struct",
        ));
    }

    let mut marked = None;
    for (member, field) in &field_members {
        if let Some(attribute) = identifier_marker(field)? {
            if marked.is_some() {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "only one field can be marked #[id]",
                ));
            }
            marked = Some(Identifier {
                member: member.clone(),
                ty: &field.ty,
            });
        }
    }

    if let Some(identifier) = marked {
        return Ok(identifier);
    }

    Err(syn::Error::new(
        fields_span(field_members.as_slice()),
        "Identifiable requires one field marked #[id]",
    ))
}

fn fields_with_members(fields: &Fields) -> Vec<(Member, &Field)> {
    match fields {
        Fields::Named(fields) => fields
            .named
            .iter()
            .map(|field| {
                let identifier = field
                    .ident
                    .clone()
                    .expect("named field should have an identifier");
                (Member::Named(identifier), field)
            })
            .collect(),
        Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(index, field)| (Member::Unnamed(index.into()), field))
            .collect(),
        Fields::Unit => Vec::new(),
    }
}

fn identifier_marker(field: &Field) -> syn::Result<Option<&syn::Attribute>> {
    let mut marker = None;
    for attribute in &field.attrs {
        if !attribute.path().is_ident("id") {
            continue;
        }
        if !matches!(attribute.meta, Meta::Path(_)) {
            return Err(syn::Error::new_spanned(
                attribute,
                "#[id] does not accept arguments",
            ));
        }
        if marker.is_some() {
            return Err(syn::Error::new_spanned(
                attribute,
                "a field can only have one #[id] marker",
            ));
        }
        marker = Some(attribute);
    }
    Ok(marker)
}

fn fields_span(fields: &[(Member, &Field)]) -> proc_macro2::Span {
    fields
        .first()
        .map_or_else(proc_macro2::Span::call_site, |(_, field)| field.span())
}

#[cfg(test)]
mod tests {
    use super::expand;

    #[test]
    fn selects_the_marked_id_field() {
        let input = syn::parse_quote! {
            struct Contact {
                #[id]
                id: u64,
                name: String,
            }
        };

        let expansion = expand(input).expect("derive should expand").to_string();

        assert!(expansion.contains("type Id = u64"));
        assert!(expansion.contains("self . id . clone"));
    }

    #[test]
    fn selects_an_explicit_identifier_field() {
        let input = syn::parse_quote! {
            struct Article {
                #[id]
                slug: String,
                title: String,
            }
        };

        let expansion = expand(input).expect("derive should expand").to_string();

        assert!(expansion.contains("type Id = String"));
        assert!(expansion.contains("self . slug . clone"));
    }

    #[test]
    fn selects_a_marked_newtype_field() {
        let input = syn::parse_quote! {
            struct ContactId(#[id] u64);
        };

        let expansion = expand(input).expect("derive should expand").to_string();

        assert!(expansion.contains("self . 0 . clone"));
    }

    #[test]
    fn rejects_an_unmarked_id_field() {
        let input = syn::parse_quote! {
            struct Contact {
                id: u64,
                name: String,
            }
        };

        let error = expand(input).expect_err("derive should require an explicit marker");

        assert_eq!(
            error.to_string(),
            "Identifiable requires one field marked #[id]"
        );
    }

    #[test]
    fn rejects_multiple_identifier_fields() {
        let input = syn::parse_quote! {
            struct Contact {
                #[id]
                local_id: u64,
                #[id]
                remote_id: u64,
            }
        };

        let error = expand(input).expect_err("derive should reject multiple identifiers");

        assert_eq!(error.to_string(), "only one field can be marked #[id]");
    }

    #[test]
    fn rejects_enums() {
        let input = syn::parse_quote! {
            enum Contact {
                Person { id: u64 },
            }
        };

        let error = expand(input).expect_err("derive should reject enums");

        assert_eq!(
            error.to_string(),
            "Identifiable can only be derived for structs"
        );
    }
}
