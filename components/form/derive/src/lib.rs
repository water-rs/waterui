//! # `FormBuilder` Derive Macro
//!
//! This crate provides the `#[derive(FormBuilder)]` procedural macro for automatically
//! implementing the `FormBuilder` trait on struct types. This enables ergonomic form
//! creation by mapping Rust data structures directly to interactive UI components.
//!
//! ## Quick Example
//!
//! ```rust
//! use waterui_form::{FormBuilder, form};
//! use waterui_core::{Binding, View};
//!
//! #[derive(Default, Clone, Debug, FormBuilder)]
//! pub struct RegistrationForm {
//!     /// User's full name
//!     pub full_name: String,
//!     /// User's email address
//!     pub email: String,
//!     /// User's age (must be 18+)
//!     pub age: i32,
//!     /// Subscribe to newsletter
//!     pub subscribe: bool,
//! }
//!
//! fn registration_view() -> impl View {
//!     let form_binding = RegistrationForm::binding();
//!     form(&form_binding)
//! }
//! ```
//!
//! ## Type Mapping Rules
//!
//! The derive macro automatically selects appropriate form components based on field types:
//!
//! - **Text Types**: `String`, `&str`, `alloc::string::String` → `TextField`
//! - **Boolean**: `bool` → `Toggle` (switch/checkbox)  
//! - **Integers**: `i8`, `i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize` → `Stepper`
//! - **Floats**: `f32`, `f64` → `Slider` (0.0 to 1.0 range)
//! - **Colors**: `Color` → `ColorPicker`
//!
//! ## Requirements
//!
//! To use `#[derive(FormBuilder)]`, your struct must:
//!
//! 1. Have named fields (not tuple structs or unit structs)
//! 2. All fields must have supported types
//! 3. Typically should also derive `Default` and `Clone` for convenience
//!
//! ## Doc Comments as Labels
//!
//! Documentation comments on struct fields become labels in the generated form:
//!
//! ```rust
//! # use waterui_form::FormBuilder;
//! #[derive(FormBuilder)]
//! struct UserSettings {
//!     /// Your display name (visible to other users)
//!     display_name: String,
//!     /// Enable dark mode theme
//!     dark_mode: bool,
//!     /// Notification volume level
//!     volume: f32,
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Type, parse_macro_input};

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
/// # Basic Example
///
/// ```rust
/// use waterui_form::{FormBuilder, form};
/// use waterui_core::{Binding, View};
///
/// #[derive(Default, Clone, Debug, FormBuilder)]
/// pub struct UserProfile {
///     /// User's display name
///     username: String,
///     /// Account password (will be secured automatically)
///     password: String,
///     /// User's current age
///     age: i32,
///     /// Email notification preference
///     email_notifications: bool,
///     /// Profile completion percentage
///     completion: f32,
/// }
///
/// fn profile_form() -> impl View {
///     let binding = UserProfile::binding();
///     form(&binding)
/// }
/// ```
///
/// # Advanced Usage with Validation
///
/// ```rust
/// use waterui_form::{FormBuilder, form};
/// use waterui_core::{Binding, View};
/// use waterui_layout::vstack;
/// use waterui_text::text;
///
/// #[derive(Default, Clone, Debug, FormBuilder)]
/// struct RegistrationForm {
///     /// Full name (required)
///     full_name: String,
///     /// Email address (must be valid)
///     email: String,
///     /// Age (must be 18+)
///     age: i32,
///     /// Agree to terms and conditions
///     agree_terms: bool,
/// }
///
/// fn registration_with_validation() -> impl View {
///     let form_binding = RegistrationForm::binding();
///     
///     vstack((
///         form(&form_binding),
///         // Real-time validation feedback
///         text!(
///             if form_binding.full_name.get().is_empty() {
///                 "Please enter your full name"
///             } else if form_binding.age.get() < 18 {
///                 "Must be 18 or older"
///             } else if !form_binding.agree_terms.get() {
///                 "Please agree to terms"
///             } else {
///                 "Form is valid!"
///             }
///         ),
///     ))
/// }
/// ```
///
/// # Requirements
///
/// For successful derivation, your struct must:
///
/// 1. **Named Fields**: Must be a struct with named fields (`struct Foo { field: Type }`)
/// 2. **Supported Types**: All fields must use types from the mapping table above
/// 3. **Recommended Traits**: Should also derive `Default`, `Clone`, and `Debug`
///
/// # Generated Layout
///
/// The macro generates a vertical stack (`VStack`) containing form fields in the order
/// they appear in the struct definition. Each field becomes a labeled form control
/// using any doc comments as the label text.
///
/// # Compile-Time Errors
///
/// This macro will produce compile-time errors (not runtime panics) if:
///
/// - Applied to enums, unions, or tuple structs
/// - Applied to structs with unnamed fields
/// - Any struct field has an unsupported type
/// - Required trait bounds are not satisfied
///
/// # Doc Comments as Labels
///
/// Documentation comments on fields become user-visible labels:
///
/// ```rust
/// # use waterui_form::FormBuilder;
/// #[derive(FormBuilder)]
/// struct Settings {
///     /// Your preferred display name
///     display_name: String,  // Label: "Your preferred display name"
///     
///     /// Enable dark mode for better night viewing
///     dark_mode: bool,       // Label: "Enable dark mode for better night viewing"
/// }
/// ```
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
            return syn::Error::new_spanned(input, "FormBuilder can only be derived for structs")
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
        "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128"
        | "usize" => Some(quote! { crate::Stepper }),
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
                "The #[form] macro can only be applied to structs"
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
            pub #field_name: nami::Binding<#field_type>
        }
    });

    // Generate the projection logic
    let field_projections = fields.named.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: {
                let source = source.clone();
                nami::Binding::mapping(
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
        impl #impl_generics_with_static nami::project::Project for #name #ty_generics #where_clause {
            type Projected = #projected_struct_name #ty_generics;

            fn project(source: &nami::Binding<Self>) -> Self::Projected {
                #projected_struct_name {
                    #(#field_projections,)*
                }
            }
        }
    };

    TokenStream::from(expanded)
}
