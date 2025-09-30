//! # `FormBuilder` Derive Macro
//!
//! This crate provides the `#[derive(FormBuilder)]` procedural macro for automatically
//! implementing the `FormBuilder` trait on struct types. This enables ergonomic form
//! creation by mapping Rust data structures directly to interactive UI components.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Meta, parse_macro_input};

/// Derives the `FormBuilder` trait for structs, enabling automatic form generation.
///
/// This macro generates a complete `FormBuilder` implementation that creates a vertical
/// stack of form fields. Each struct field is automatically mapped to an appropriate
/// interactive form component based on its type.
///
/// # Type-to-Component Mapping
///
/// The macro uses these mapping rules:
///
/// | Rust Type | Form Component | Description |
/// |-----------|----------------|-------------|
/// | `String`, `&str`, `alloc::string::String` | `TextField` | Single-line text input |
/// | `bool` | `Toggle` | Switch/checkbox for boolean values |
/// | `i8`, `i16`, `i32`, `i64`, `i128`, `isize` | `Stepper` | Numeric input with +/- buttons |
/// | `u8`, `u16`, `u32`, `u64`, `u128`, `usize` | `Stepper` | Unsigned numeric input |
/// | `f32`, `f64` | `Slider` | Slider with 0.0-1.0 range |
/// | `Color` | `ColorPicker` | Color selection widget |
///
/// # Panics
///
/// This function will panic if the struct contains fields without identifiers,
/// which should not happen with named fields in normal Rust structs.
#[proc_macro_derive(FormBuilder)]
pub fn derive_form_builder(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => &fields.named,
            _ => {
                return syn::Error::new_spanned(
                    input,
                    "FormBuilder can only be derived for structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(input, "FormBuilder can only be derived for structs")
                .to_compile_error()
                .into();
        }
    };

    // Collect field information
    let field_views = fields.iter().map(|field| {
        let field_name = field.ident.as_ref().unwrap();
        let field_type = &field.ty;

        // Convert field name from snake_case to "Title Case" for label
        let field_name_str = field_name.to_string();
        let label_text = snake_to_title_case(&field_name_str);

        // Extract doc comments as placeholder text
        let placeholder = field
            .attrs
            .iter()
            .filter_map(|attr| {
                if attr.path().is_ident("doc")
                    && let Meta::NameValue(meta) = &attr.meta
                    && let syn::Expr::Lit(expr_lit) = &meta.value
                    && let syn::Lit::Str(lit_str) = &expr_lit.lit
                {
                    let doc = lit_str.value();
                    // Clean up the doc comment (remove leading/trailing whitespace)
                    let cleaned = doc.trim();
                    if !cleaned.is_empty() {
                        return Some(cleaned.to_string());
                    }
                }
                None
            })
            .collect::<Vec<_>>()
            .join(" ");

        // Use FormBuilder trait for all types
        // The FormBuilder::view method will handle whether to use the placeholder or not
        quote! {
            <#field_type as crate::FormBuilder>::view(
                &projected.#field_name,
                ::waterui::AnyView::new(#label_text),
                ::waterui::Str::from(#placeholder)
            )
        }
    });

    // Check if we need to require Project trait
    let requires_project = !fields.is_empty();

    let view_body = if requires_project {
        quote! {
            // Use the Project trait to get individual field bindings
            let projected = <Self as ::waterui::reactive::project::Project>::project(binding);

            // Create a vstack with all form fields
            ::waterui::component::layout::stack::vstack((
                #(#field_views,)*
            ))
        }
    } else {
        // Empty struct case
        quote! {
            ::waterui::component::layout::stack::vstack(())
        }
    };

    // Generate the implementation
    let expanded = quote! {
        impl crate::FormBuilder for #name {
            type View = ::waterui::component::layout::stack::VStack;

            fn view(binding: &::waterui::Binding<Self>, _label: ::waterui::AnyView, _placeholder: ::waterui::Str) -> Self::View {
                #view_body
            }
        }
    };

    TokenStream::from(expanded)
}

/// Converts `snake_case` to "Title Case"
fn snake_to_title_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first
                    .to_uppercase()
                    .chain(chars.as_str().to_lowercase().chars())
                    .collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `#[form]` attribute macro that automatically derives multiple traits commonly used for forms.
///
/// This macro derives the following traits:
/// - `Default`
/// - `Clone`
/// - `Debug`
/// - `FormBuilder`
/// - `Project` (from nami for reactive state management)
/// - `Serialize` and `Deserialize` (from serde, if available)
///
/// # Example
///
/// ```rust
/// use waterui_form::{form, form};
///
/// #[form]
/// pub struct UserForm {
///     /// User's full name
///     pub name: String,
///     /// User's age
///     pub age: i32,
///     /// Email notifications enabled
///     pub notifications: bool,
/// }
///
/// fn create_form() -> impl View {
///     let form_binding = UserForm::binding();
///     form(&form_binding)
/// }
/// ```
///
/// This is equivalent to manually writing:
///
/// ```rust
/// #[derive(Default, Clone, Debug, FormBuilder)]
/// #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
/// pub struct UserForm {
///     pub name: String,
///     pub age: i32,
///     pub notifications: bool,
/// }
///
/// impl Project for UserForm {
///     // ... implementation provided by nami derive
/// }
/// ```
#[proc_macro_attribute]
pub fn form(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (_impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Check if it's a struct with named fields
    let fields = match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => fields,
            _ => {
                return syn::Error::new_spanned(
                    input,
                    "The #[form] macro can only be applied to structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(
                input,
                "The #[form] macro can only be applied to structs",
            )
            .to_compile_error()
            .into();
        }
    };

    // Create the projected struct name for nami Project trait
    let projected_struct_name = syn::Ident::new(&format!("{name}Projected"), name.span());

    // Generate fields for the projected struct
    let projected_fields = fields.named.iter().map(|field| {
        let field_name = &field.ident;
        let field_type = &field.ty;
        quote! {
            pub #field_name: ::waterui::reactive::Binding<#field_type>
        }
    });

    // Generate the projection logic
    let field_projections = fields.named.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: {
                let source = source.clone();
                ::waterui::reactive::Binding::mapping(
                    &source,
                    |value| value.#field_name.clone(),
                    move |binding, value| {
                        binding.get_mut().#field_name = value;
                    },
                )
            }
        }
    });

    // Add lifetime bounds to generic parameters for Project trait
    let mut generics_with_static = input.generics.clone();
    for param in &mut generics_with_static.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param.bounds.push(syn::parse_quote!('static));
        }
    }
    let (impl_generics_with_static, _, _) = generics_with_static.split_for_impl();

    let expanded = quote! {
        // Original struct with conditional serde derives
        #[derive(Default, Clone, Debug, FormBuilder)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[allow(missing_docs)] // Allow missing docs for generated structs
        #input

        // Projected struct for nami Project trait
        #[derive(Debug)]
        #[allow(missing_docs)] // Allow missing docs for generated structs
        pub struct #projected_struct_name #ty_generics #where_clause {
            #(#projected_fields,)*
        }

        // Project trait implementation
        impl #impl_generics_with_static ::waterui::reactive::project::Project for #name #ty_generics #where_clause {
            type Projected = #projected_struct_name #ty_generics;

            fn project(source: &::waterui::reactive::Binding<Self>) -> Self::Projected {
                #projected_struct_name {
                    #(#field_projections,)*
                }
            }
        }
    };

    TokenStream::from(expanded)
}
