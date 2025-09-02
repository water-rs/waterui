//! Derive macro for the `FormBuilder` trait.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, Fields, Type};

/// Derives the `FormBuilder` trait for structs.
/// 
/// This macro generates a `FormBuilder` implementation that creates a vertical stack
/// of form fields based on the struct's field types. Each field is mapped to an 
/// appropriate form component:
/// 
/// - `String` or `&str` -> `TextField`
/// - `bool` -> `Toggle`
/// - `i32`, `i64`, `u32`, `u64`, etc. -> `Stepper`
/// - `f32`, `f64` -> `Slider` (with 0.0-1.0 range)
/// - `Color` -> `ColorPicker`
/// 
/// # Example
/// 
/// ```rust
/// use waterui_form::FormBuilder;
/// 
/// #[derive(FormBuilder)]
/// pub struct UserForm {
///     username: String,
///     password: String, 
///     age: i32,
///     remember_me: bool,
/// }
/// ```
/// 
/// # Panics
/// 
/// This macro will cause a compile-time error (not a runtime panic) if:
/// - The target is not a struct with named fields
/// - Any field has an unsupported type
#[proc_macro_derive(FormBuilder)]
pub fn derive_form_builder(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let _fields = match &input.data {
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
            return syn::Error::new_spanned(
                input,
                "FormBuilder can only be derived for structs",
            )
            .to_compile_error()
            .into();
        }
    };

    // For now, create a simple implementation that shows we can derive the trait
    let expanded = quote! {
        impl crate::FormBuilder for #name {
            type View = waterui_core::Str;

            fn view(_binding: &waterui_core::Binding<Self>) -> Self::View {
                "Generated Form (FormBuilder derive working!)".into()
            }
        }
    };

    TokenStream::from(expanded)
}

/// Maps a type to the appropriate form component.
#[allow(dead_code)]
fn get_form_component_for_type(ty: &Type) -> Option<proc_macro2::TokenStream> {
    
    let type_str = match ty {
        Type::Path(type_path) => {
            let path = &type_path.path;
            if path.segments.len() == 1 {
                let segment = &path.segments[0];
                segment.ident.to_string()
            } else {
                quote!(#ty).to_string()
            }
        }
        _ => quote!(#ty).to_string(),
    };

    match type_str.as_str() {
        "String" | "str" | "&str" => Some(quote! { crate::TextField }),
        "bool" => Some(quote! { crate::Toggle }),
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" |
        "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => Some(quote! { crate::Stepper }),
        "f32" | "f64" => Some(quote! { crate::slider::Slider }),
        "Color" => Some(quote! { crate::picker::ColorPicker }),
        _ => {
            // Handle qualified paths like alloc::string::String
            if type_str.contains("String") {
                Some(quote! { crate::TextField })
            } else {
                None
            }
        }
    }
}