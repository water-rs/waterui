//! Procedural macros for `WaterUI` framework.
//!
//! This crate provides derive macros and procedural macros for the `WaterUI` framework,
//! including form generation, reactive signal formatting, and view builder patterns.

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, ItemFn, Meta, parse_macro_input};

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
/// | `f64` | `Slider` | Slider with 0.0-1.0 range |
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
        let field_name = field
            .ident
            .as_ref()
            .expect("field should have an identifier");
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
            ::waterui::component::stack::vstack((
                #(#field_views,)*
            ))
        }
    } else {
        // Empty struct case
        quote! {
            ::waterui::component::stack::vstack(())
        }
    };

    let field_types = fields.iter().map(|field| &field.ty);

    // Generate the implementation
    let expanded = quote! {
        impl crate::FormBuilder for #name {
            type View = ::waterui::component::stack::VStack<((#(<#field_types as crate::FormBuilder>::View),*),)>;

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
/// - `Project` (from `waterui::reactive` for reactive state management)
/// - `Serialize` and `Deserialize` (from serde, if available)
///
/// # Example
///
/// ```text
/// use waterui::{form, FormBuilder};
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
/// ```text
/// #[derive(Default, Clone, Debug, FormBuilder)]
/// #[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
/// pub struct UserForm {
///     pub name: String,
///     pub age: i32,
///     pub notifications: bool,
/// }
///
/// impl Project for UserForm {
///     // ... implementation provided by waterui::reactive derive
/// }
/// ```
#[proc_macro_attribute]
pub fn form(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let _name = &input.ident;
    let (_impl_generics, _ty_generics, _where_clause) = input.generics.split_for_impl();

    // Check if it's a struct with named fields
    let _fields = match &input.data {
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

    let expanded = quote! {
        #[derive(Default, Clone, Debug, ::waterui::FormBuilder, ::waterui::Project)]
        #input
    };

    TokenStream::from(expanded)
}

use syn::{Expr, LitStr, Token, Type, parse::Parse, punctuated::Punctuated};

/// Derive macro for implementing the `Project` trait on structs.
///
/// This macro automatically generates a `Project` implementation that allows
/// decomposing a struct binding into separate bindings for each field.
///
/// # Examples
///
/// ```rust
/// use waterui::reactive::{Binding, binding, project::Project};
/// use waterui_macros::Project;
///
/// #[derive(Project, Clone)]
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// let person_binding: Binding<Person> = binding(Person {
///     name: "Alice".to_string(),
///     age: 30,
/// });
///
/// let projected = person_binding.project();
/// projected.name.set("Bob".to_string());
/// projected.age.set(25u32);
///
/// let person = person_binding.get();
/// assert_eq!(person.name, "Bob");
/// assert_eq!(person.age, 25);
/// ```
#[proc_macro_derive(Project)]
pub fn derive_project(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => derive_project_struct(&input, fields_named),
            Fields::Unnamed(fields_unnamed) => derive_project_tuple_struct(&input, fields_unnamed),
            Fields::Unit => derive_project_unit_struct(&input),
        },
        Data::Enum(_) => {
            syn::Error::new_spanned(input, "Project derive macro does not support enums")
                .to_compile_error()
                .into()
        }
        Data::Union(_) => {
            syn::Error::new_spanned(input, "Project derive macro does not support unions")
                .to_compile_error()
                .into()
        }
    }
}

fn derive_project_struct(input: &DeriveInput, fields: &syn::FieldsNamed) -> TokenStream {
    let struct_name = &input.ident;
    let (_impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Create the projected struct type
    let projected_struct_name =
        syn::Ident::new(&format!("{struct_name}Projected"), struct_name.span());

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

    // Add lifetime bounds to generic parameters
    let mut generics_with_static = input.generics.clone();
    for param in &mut generics_with_static.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param.bounds.push(syn::parse_quote!('static));
        }
    }
    let (impl_generics_with_static, _, _) = generics_with_static.split_for_impl();

    let expanded = quote! {
        /// Projected version of #struct_name with each field wrapped in a Binding.
        #[derive(Debug)]
        pub struct #projected_struct_name #ty_generics #where_clause {
            #(#projected_fields,)*
        }

        impl #impl_generics_with_static ::waterui::reactive::project::Project for #struct_name #ty_generics #where_clause {
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

fn derive_project_tuple_struct(input: &DeriveInput, fields: &syn::FieldsUnnamed) -> TokenStream {
    let struct_name = &input.ident;
    let (_impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Generate tuple type for projection
    let field_types: Vec<&Type> = fields.unnamed.iter().map(|field| &field.ty).collect();
    let projected_tuple = if field_types.len() == 1 {
        quote! { (::waterui::reactive::Binding<#(#field_types)*>,) }
    } else {
        quote! { (#(::waterui::reactive::Binding<#field_types>),*) }
    };

    // Generate field projections using index access
    let field_projections = fields.unnamed.iter().enumerate().map(|(index, _)| {
        let idx = syn::Index::from(index);
        quote! {
            {
                let source = source.clone();
                ::waterui::reactive::Binding::mapping(
                    &source,
                    |value| value.#idx.clone(),
                    move |binding, value| {
                        binding.get_mut().#idx = value;
                    },
                )
            }
        }
    });

    // Add lifetime bounds to generic parameters
    let mut generics_with_static = input.generics.clone();
    for param in &mut generics_with_static.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param.bounds.push(syn::parse_quote!('static));
        }
    }
    let (impl_generics_with_static, _, _) = generics_with_static.split_for_impl();

    let projection_tuple = if field_projections.len() == 1 {
        quote! { (#(#field_projections)*,) }
    } else {
        quote! { (#(#field_projections),*) }
    };

    let expanded = quote! {
        impl #impl_generics_with_static ::waterui::reactive::project::Project for #struct_name #ty_generics #where_clause {
            type Projected = #projected_tuple;

            fn project(source: &::waterui::reactive::Binding<Self>) -> Self::Projected {
                #projection_tuple
            }
        }
    };

    TokenStream::from(expanded)
}

fn derive_project_unit_struct(input: &DeriveInput) -> TokenStream {
    let struct_name = &input.ident;
    let (_impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Add lifetime bounds to generic parameters
    let mut generics_with_static = input.generics.clone();
    for param in &mut generics_with_static.params {
        if let syn::GenericParam::Type(type_param) = param {
            type_param.bounds.push(syn::parse_quote!('static));
        }
    }
    let (impl_generics_with_static, _, _) = generics_with_static.split_for_impl();

    let expanded = quote! {
        impl #impl_generics_with_static ::waterui::reactive::project::Project for #struct_name #ty_generics #where_clause {
            type Projected = ();

            fn project(_source: &::waterui::reactive::Binding<Self>) -> Self::Projected {
                ()
            }
        }
    };

    TokenStream::from(expanded)
}

/// Input structure for the `s!` macro
struct SInput {
    format_str: LitStr,
    args: Punctuated<Expr, Token![,]>,
}

impl Parse for SInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let format_str: LitStr = input.parse()?;
        let args = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            Punctuated::parse_terminated(input)?
        } else {
            Punctuated::new()
        };

        Ok(Self { format_str, args })
    }
}

/// Function-like procedural macro for creating formatted string signals with automatic variable capture.
///
/// This macro automatically detects named variables in format strings and captures them from scope.
///
/// # Examples
///
/// ```rust
/// use waterui_macros::s;
/// use waterui::reactive::constant;
///
/// let name = constant("Alice");
/// let age = constant(25);
///
/// // Automatic variable capture from format string
/// let msg = s!("Hello {name}, you are {age} years old");
///
/// // Positional arguments still work
/// let msg2 = s!("Hello {}, you are {}", name, age);
/// ```
#[proc_macro]
#[allow(clippy::similar_names, clippy::too_many_lines)]
pub fn s(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as SInput);
    let format_str = input.format_str;
    let format_value = format_str.value();

    // Check for format string issues
    let (has_positional, has_named, positional_count, named_vars) =
        analyze_format_string(&format_value);

    // If there are explicit arguments, validate and use positional approach
    if !input.args.is_empty() {
        // Check for mixed usage errors
        if has_named {
            return syn::Error::new_spanned(
                &format_str,
                format!(
                    "Format string contains named arguments like {{{}}} but you provided positional arguments. \
                    Either use positional placeholders like {{}} or remove the explicit arguments to use automatic variable capture.",
                    named_vars.first().unwrap_or(&String::new())
                )
            )
            .to_compile_error()
            .into();
        }

        // Check argument count matches placeholders
        if positional_count != input.args.len() {
            return syn::Error::new_spanned(
                &format_str,
                format!(
                    "Format string has {} positional placeholders but {} arguments were provided",
                    positional_count,
                    input.args.len()
                ),
            )
            .to_compile_error()
            .into();
        }
        let args: Vec<_> = input.args.iter().collect();
        return match args.len() {
            1 => {
                let arg = &args[0];
                quote! {
                    {
                        use ::waterui::reactive::SignalExt;
                        SignalExt::map(#arg.clone(), |arg| waterui::reactive::__format!(#format_str, arg))
                    }
                }
                .into()
            }
            2 => {
                let arg1 = &args[0];
                let arg2 = &args[1];
                quote! {
                    {
                        use waterui::reactive::{SignalExt, zip::zip};
                        SignalExt::map(zip(#arg1.clone(), #arg2.clone()), |(arg1, arg2)| {
                            waterui::reactive::__format!(#format_str, arg1, arg2)
                        })
                    }
                }
                .into()
            }
            3 => {
                let arg1 = &args[0];
                let arg2 = &args[1];
                let arg3 = &args[2];
                quote! {
                    {
                        use ::waterui::reactive::{SignalExt, zip::zip};
                        SignalExt::map(
                            zip(zip(#arg1.clone(), #arg2.clone()), #arg3.clone()),
                            |((arg1, arg2), arg3)| waterui::reactive::__format!(#format_str, arg1, arg2, arg3)
                        )
                    }
                }
                .into()
            }
            4 => {
                let arg1 = &args[0];
                let arg2 = &args[1];
                let arg3 = &args[2];
                let arg4 = &args[3];
                quote! {
                    {
                        use ::waterui::reactive::{SignalExt, zip::zip};
                        SignalExt::map(
                            zip(
                                zip(#arg1.clone(), #arg2.clone()),
                                zip(#arg3.clone(), #arg4.clone())
                            ),
                            |((arg1, arg2), (arg3, arg4))| waterui::reactive::__format!(#format_str, arg1, arg2, arg3, arg4)
                        )
                    }
                }.into()
            }
            _ => syn::Error::new_spanned(format_str, "Too many arguments, maximum 4 supported")
                .to_compile_error()
                .into(),
        };
    }

    // Check for mixed placeholders when no explicit arguments
    if has_positional && has_named {
        return syn::Error::new_spanned(
            &format_str,
            "Format string mixes positional {{}} and named {{var}} placeholders. \
            Use either all positional with explicit arguments, or all named for automatic capture.",
        )
        .to_compile_error()
        .into();
    }

    // If has positional placeholders but no arguments provided
    if has_positional && input.args.is_empty() {
        return syn::Error::new_spanned(
            &format_str,
            format!(
                "Format string has {positional_count} positional placeholder(s) {{}} but no arguments provided. \
                Either provide arguments or use named placeholders like {{variable}} for automatic capture."
            )
        )
        .to_compile_error()
        .into();
    }

    // Parse format string to extract variable names for automatic capture
    let var_names = named_vars;

    // If no variables found, return constant
    if var_names.is_empty() {
        return quote! {
            {
                use ::waterui::reactive::constant;
                constant(waterui::reactive::__format!(#format_str))
            }
        }
        .into();
    }

    // Generate code for named variable capture
    let var_idents: Vec<syn::Ident> = var_names
        .iter()
        .map(|name| syn::Ident::new(name, format_str.span()))
        .collect();

    match var_names.len() {
        1 => {
            let var = &var_idents[0];
            quote! {
                {
                    use ::waterui::reactive::SignalExt;
                    SignalExt::map(#var.clone(), |#var| {
                        waterui::reactive::__format!(#format_str)
                    })
                }
            }
            .into()
        }
        2 => {
            let var1 = &var_idents[0];
            let var2 = &var_idents[1];
            quote! {
                {
                    use ::waterui::reactive::{SignalExt, zip::zip};
                    SignalExt::map(zip(#var1.clone(), #var2.clone()), |(#var1, #var2)| {
                        waterui::reactive::__format!(#format_str)
                    })
                }
            }
            .into()
        }
        3 => {
            let var1 = &var_idents[0];
            let var2 = &var_idents[1];
            let var3 = &var_idents[2];
            quote! {
                {
                    use ::waterui::reactive::{SignalExt, zip::zip};
                    SignalExt::map(
                        zip(zip(#var1.clone(), #var2.clone()), #var3.clone()),
                        |((#var1, #var2), #var3)| {
                            ::waterui::reactive::__format!(#format_str)
                        }
                    )
                }
            }
            .into()
        }
        4 => {
            let var1 = &var_idents[0];
            let var2 = &var_idents[1];
            let var3 = &var_idents[2];
            let var4 = &var_idents[3];
            quote! {
                {
                    use ::waterui::reactive::{SignalExt, zip::zip};
                    SignalExt::map(
                        zip(
                            zip(#var1.clone(), #var2.clone()),
                            zip(#var3.clone(), #var4.clone())
                        ),
                        |((#var1, #var2), (#var3, #var4))| {
                            ::waterui::reactive::__format!(#format_str)
                        }
                    )
                }
            }
            .into()
        }
        _ => syn::Error::new_spanned(format_str, "Too many named variables, maximum 4 supported")
            .to_compile_error()
            .into(),
    }
}

/// Analyze a format string to detect placeholder types and extract variable names
fn analyze_format_string(format_str: &str) -> (bool, bool, usize, Vec<String>) {
    let mut has_positional = false;
    let mut has_named = false;
    let mut positional_count = 0;
    let mut named_vars = Vec::new();
    let mut chars = format_str.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'{') {
            // Skip escaped braces
            chars.next();
        } else if c == '{' {
            let mut content = String::new();
            let mut has_content = false;

            while let Some(&next_char) = chars.peek() {
                if next_char == '}' {
                    chars.next(); // consume }
                    break;
                } else if next_char == ':' {
                    // Format specifier found, we've captured the name/position part
                    chars.next(); // consume :
                    while let Some(&spec_char) = chars.peek() {
                        if spec_char == '}' {
                            chars.next(); // consume }
                            break;
                        }
                        chars.next();
                    }
                    break;
                }
                content.push(chars.next().unwrap());
                has_content = true;
            }

            // Analyze the content
            if !has_content || content.is_empty() {
                // Empty {} is positional
                has_positional = true;
                positional_count += 1;
            } else if content.chars().all(|ch| ch.is_ascii_digit()) {
                // Numeric like {0} or {1} is positional
                has_positional = true;
                positional_count += 1;
            } else if content
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            {
                // Starts with letter or underscore, likely a variable name
                has_named = true;
                if !named_vars.contains(&content) {
                    named_vars.push(content);
                }
            } else {
                // Other cases treat as positional
                has_positional = true;
                positional_count += 1;
            }
        }
    }

    (has_positional, has_named, positional_count, named_vars)
}

/// Attribute macro for enabling hot reload on view functions.
///
/// This macro transforms a function returning `impl View` to support hot reloading.
/// When the library is rebuilt during development, the view will automatically update
/// without restarting the application.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
///
/// #[hot_reload]
/// fn sidebar() -> impl View {
///     vstack((
///         text("Sidebar"),
///         text("Content"),
///     ))
/// }
///
/// fn main() -> impl View {
///     hstack((
///         sidebar(),  // This view will hot reload
///         content_panel(),
///     ))
/// }
/// ```
///
/// # How It Works
///
/// The macro:
/// 1. Wraps the function body in a `HotReloadView` that registers with the hot reload system
/// 2. Generates a C-exported symbol (when built with `--cfg waterui_hot_reload_lib`) that
///    the CLI can load to get the updated view
///
/// The generated symbol name follows the pattern: `waterui_hot_reload_<function_name>`
///
/// # Requirements
///
/// - The function must return `impl View`
/// - Hot reload must be enabled via environment variables (set by `water run`)
/// - For development, build with `RUSTFLAGS="--cfg waterui_hot_reload_lib"`
#[proc_macro_attribute]
pub fn hot_reload(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);

    let fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_attrs = &input_fn.attrs;
    let fn_sig = &input_fn.sig;
    let fn_block = &input_fn.block;

    // Create the function ID: module_path::function_name
    let fn_name_str = fn_name.to_string();

    // Generate the export symbol name
    let export_fn_name =
        syn::Ident::new(&format!("waterui_hot_reload_{fn_name_str}"), fn_name.span());

    if std::env::var("WATERUI_ENABLE_HOT_RELOAD").unwrap_or_default() != "1" {
        // If hot reload is not enabled, return the original function unchanged
        let expanded = quote! {
            #(#fn_attrs)*
            #fn_vis #fn_sig #fn_block
        };
        return TokenStream::from(expanded);
    }

    let expanded = quote! {
        #(#fn_attrs)*
        #fn_vis #fn_sig {
            ::waterui::debug::HotReloadView::new(
                concat!(module_path!(), "::", #fn_name_str),
                || #fn_block
            )
        }

        // Generate C export symbol for hot reload library
        // Symbol name: waterui_hot_reload_<fn_name>
        #[cfg(waterui_hot_reload_lib)]
        #[doc(hidden)]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn #export_fn_name() -> *mut () {
            let view = #fn_block;
            Box::into_raw(Box::new(::waterui::AnyView::new(view))).cast()
        }
    };

    TokenStream::from(expanded)
}
