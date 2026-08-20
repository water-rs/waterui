//! Procedural macros for `WaterUI` framework.
//!
//! This crate provides derive macros and procedural macros for the `WaterUI` framework,
//! including form generation, reactive signal formatting, and view builder patterns.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use std::collections::HashMap;
use syn::{Data, DeriveInput, Fields, ItemFn, Meta, parse_macro_input};
mod identifiable;
mod locale;
mod view_builder;

fn waterui_crate_path() -> syn::Result<TokenStream2> {
    if std::env::var("CARGO_PKG_NAME").as_deref() == Ok("waterui-internal") {
        return Ok(quote!(crate));
    }

    match crate_name("waterui") {
        Ok(FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(error) => Err(syn::Error::new(
            Span::call_site(),
            format!(
                "WaterUI macro requires the `waterui` crate as a dependency; \
                 Cargo.toml may rename it: {error}"
            ),
        )),
    }
}

mod javascript;
mod js_api;

/// Exposes an `impl` block to the page.
///
/// ```ignore
/// #[js_api(namespace = "app")]
/// impl App {
///     fn theme(&self) -> Binding<Theme> { self.theme.clone() }
///     async fn greet(&self, name: String) -> Result<Json<Greeting>, ApiError> { ... }
/// }
///
/// WebView::open(url).serve(app)
/// ```
///
/// An `async fn` becomes a handler the page calls as
/// `await app.greet({ name: "Lexo" })`; a `fn(&self) -> impl JsField` becomes
/// mirrored state at `app.state.theme`. The distinction is made from the
/// method's shape rather than its return type, because a macro cannot see types
/// and matching on how one is spelled would break on a type alias.
///
/// Control it with `#[js(skip)]` and `#[js(rename = "...")]`.
#[proc_macro_attribute]
pub fn js_api(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = syn::parse_macro_input!(args as js_api::Args);
    let block = syn::parse_macro_input!(input as syn::ItemImpl);
    match js_api::expand(&args, block) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Runs a JavaScript program for its effects.
///
/// ```ignore
/// exec!(webview, "app.setTheme(@{theme})").await?;
/// ```
///
/// `@{expr}` holes are passed to the engine as bound arguments rather than
/// spliced into the source, so a value can never be read back as code. The sigil
/// is `@{}` because `${}` belongs to JavaScript's own template literals; write
/// `@@{` for a literal `@{`.
#[proc_macro]
pub fn exec(input: TokenStream) -> TokenStream {
    let call = syn::parse_macro_input!(input as javascript::ScriptCall);
    let interpolated = match javascript::interpolate(&call.source) {
        Ok(interpolated) => interpolated,
        Err(error) => return error.to_compile_error().into(),
    };
    let receiver = &call.receiver;
    let program = javascript::build(&quote::quote!(::waterui::webview::JsProgram), &interpolated);
    quote::quote!(#receiver.exec(&#program)).into()
}

/// Evaluates a JavaScript expression and decodes its result.
///
/// ```ignore
/// let title: String = eval!(webview, "document.title").await?;
/// ```
///
/// Interpolation works exactly as in [`exec!`].
#[proc_macro]
pub fn eval(input: TokenStream) -> TokenStream {
    let call = syn::parse_macro_input!(input as javascript::ScriptCall);
    let interpolated = match javascript::interpolate(&call.source) {
        Ok(interpolated) => interpolated,
        Err(error) => return error.to_compile_error().into(),
    };
    let receiver = &call.receiver;
    let expr = javascript::build(&quote::quote!(::waterui::webview::JsExpr), &interpolated);
    quote::quote!(#receiver.eval(&#expr)).into()
}

/// A JavaScript program read from a file at compile time.
///
/// ```ignore
/// webview.inject(js_file!("scripts/setup.js"), ScriptInjectionTime::DocumentStart);
/// exec_file!(webview, "scripts/setup.js").await?;
/// ```
///
/// Multi-line JavaScript belongs in a file rather than a string literal, and this
/// is how it gets there.
#[proc_macro]
pub fn js_file(input: TokenStream) -> TokenStream {
    let file = syn::parse_macro_input!(input as javascript::ScriptFile);
    let path = &file.path;
    quote::quote!(::waterui::webview::JsProgram::raw(
        ::std::include_str!(#path)
    ))
    .into()
}

/// Runs a JavaScript program from a file for its effects.
///
/// See [`js_file!`]; this is the fused form of `receiver.exec(js_file!(path))`.
#[proc_macro]
pub fn exec_file(input: TokenStream) -> TokenStream {
    let file = syn::parse_macro_input!(input as javascript::ScriptFile);
    let Some(receiver) = file.receiver else {
        return syn::Error::new(
            file.path.span(),
            "exec_file! takes a web view and a path: exec_file!(webview, \"scripts/setup.js\")",
        )
        .to_compile_error()
        .into();
    };
    let path = &file.path;
    quote::quote!(#receiver.exec(&::waterui::webview::JsProgram::raw(::std::include_str!(#path))))
        .into()
}

#[proc_macro]
/// Expands `text!(...)` into a localized `Text` view with compile-time catalog loading.
///
/// **i18n contract — do not relax**: placeholder names in the format string
/// are translation slot keys. The compile-time translation extractor and the
/// runtime locale resolver both rely on a 1:1 mapping between placeholder
/// name and the identifier it binds to in the surrounding scope. This is why
/// `text!` does NOT accept arbitrary expressions in placeholder positions.
///
/// If a binding's local identifier does not match the desired slot key,
/// alias it explicitly:
///
/// ```ignore
/// text!("{slot}", slot = local_var)
/// ```
///
/// The flow-markdown example's `_text` clone aliases at the top of its
/// section function are the canonical workaround for this constraint, and
/// are intentionally preserved.
pub fn text(input: TokenStream) -> TokenStream {
    locale::text(&input)
}

#[proc_macro]
/// Expands `catalog!()` into a compile-time `TranslationCatalog` built from `i18n/`.
pub fn catalog(input: TokenStream) -> TokenStream {
    locale::catalog(&input)
}

#[proc_macro]
/// Expands a builder block that unifies `if`/`match` branch view types while
/// preserving normal Rust block and semicolon semantics.
pub fn view(input: TokenStream) -> TokenStream {
    view_builder::expand_macro(input)
}

#[proc_macro_attribute]
/// Rewrites a function or method body so tail-position `if`/`match` branches
/// can return different concrete `View` types under a shared `impl View`.
pub fn view_builder(args: TokenStream, input: TokenStream) -> TokenStream {
    view_builder::expand_attribute(args, &input)
}

/// Derives `Identifiable` using a struct field as the stable identifier.
///
/// Mark exactly one named or tuple field with `#[id]`.
///
/// The generated implementation clones the identifier when
/// `Identifiable::id` is called, so the field type must implement `Hash`,
/// `Ord`, and `Clone`.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::Identifiable;
///
/// #[derive(Clone, Identifiable)]
/// struct Contact {
///     #[id]
///     id: u64,
///     name: String,
/// }
///
/// #[derive(Clone, Identifiable)]
/// struct Article {
///     #[id]
///     slug: String,
///     title: String,
/// }
/// ```
#[proc_macro_derive(Identifiable, attributes(id))]
pub fn derive_identifiable(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    identifiable::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

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
/// | `Str`, `alloc::string::String` | `TextField` | Single-line text input |
/// | `bool` | `Toggle` | Switch/checkbox for boolean values |
/// | `i32` | `Stepper` | Numeric input with +/- buttons |
/// | `f32`, `f64` | `Slider` | Slider with 0.0-1.0 range |
/// | `Color` | `ColorPicker` | Color selection widget |
///
/// Other widths (`i8`/`u32`/`usize`/…) have no `FormBuilder` implementation:
/// convert the field to one of the types above, or implement `FormBuilder`
/// yourself for a newtype around it.
///
/// # Panics
///
/// This function will panic if the struct contains fields without identifiers,
/// which should not happen with named fields in normal Rust structs.
#[proc_macro_derive(FormBuilder)]
pub fn derive_form_builder(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
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
            <#field_type as #waterui::FormBuilder>::view(
                &projected.#field_name,
                #label_text,
                #waterui::Str::from(#placeholder)
            )
        }
    });

    // Check if we need to require Project trait
    let requires_project = !fields.is_empty();

    let view_body = if requires_project {
        quote! {
            // Use the Project trait to get individual field bindings
            let projected = <Self as #waterui::reactive::project::Project>::project(binding);

            // Create a vstack with all form fields
            #waterui::component::stack::vstack((
                #(#field_views,)*
            ))
        }
    } else {
        // Empty struct case
        quote! {
            #waterui::component::stack::vstack(())
        }
    };

    let field_types = fields.iter().map(|field| &field.ty);

    // Generate the implementation
    let expanded = quote! {
        impl #waterui::FormBuilder for #name {
            type View = #waterui::component::stack::VStack<((#(<#field_types as #waterui::FormBuilder>::View),*),)>;

            fn view<L: #waterui::component::IntoLabel>(
                binding: &#waterui::Binding<Self>,
                _label: L,
                _placeholder: #waterui::Str,
            ) -> Self::View {
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
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
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
        #[derive(Default, Clone, Debug, #waterui::FormBuilder, #waterui::Project)]
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
/// ```rust,ignore
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
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };

    match &input.data {
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields_named) => derive_project_struct(&input, fields_named, &waterui),
            Fields::Unnamed(fields_unnamed) => {
                derive_project_tuple_struct(&input, fields_unnamed, &waterui)
            }
            Fields::Unit => derive_project_unit_struct(&input, &waterui),
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

fn derive_project_struct(
    input: &DeriveInput,
    fields: &syn::FieldsNamed,
    waterui: &TokenStream2,
) -> TokenStream {
    let struct_name = &input.ident;
    let (_impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Create the projected struct type
    let projected_struct_name =
        syn::Ident::new(&format!("{struct_name}Projected"), struct_name.span());

    // Generate fields for the projected struct
    let projected_fields = fields.named.iter().map(|field| {
        let field_name = &field.ident;
        let field_type = &field.ty;
        let doc = format!(
            "Projected binding for the `{}` field.",
            field_name
                .as_ref()
                .map_or_else(String::new, ToString::to_string)
        );
        quote! {
            #[doc = #doc]
            pub #field_name: #waterui::reactive::Binding<#field_type>
        }
    });

    // Generate the projection logic
    let field_projections = fields.named.iter().map(|field| {
        let field_name = &field.ident;
        quote! {
            #field_name: {
                let source = source.clone();
                #waterui::reactive::Binding::mapping(
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

        impl #impl_generics_with_static #waterui::reactive::project::Project for #struct_name #ty_generics #where_clause {
            type Projected = #projected_struct_name #ty_generics;

            fn project(source: &#waterui::reactive::Binding<Self>) -> Self::Projected {
                #projected_struct_name {
                    #(#field_projections,)*
                }
            }
        }
    };

    TokenStream::from(expanded)
}

fn derive_project_tuple_struct(
    input: &DeriveInput,
    fields: &syn::FieldsUnnamed,
    waterui: &TokenStream2,
) -> TokenStream {
    let struct_name = &input.ident;
    let (_impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Generate tuple type for projection
    let field_types: Vec<&Type> = fields.unnamed.iter().map(|field| &field.ty).collect();
    let projected_tuple = if field_types.len() == 1 {
        quote! { (#waterui::reactive::Binding<#(#field_types)*>,) }
    } else {
        quote! { (#(#waterui::reactive::Binding<#field_types>),*) }
    };

    // Generate field projections using index access
    let field_projections = fields.unnamed.iter().enumerate().map(|(index, _)| {
        let idx = syn::Index::from(index);
        quote! {
            {
                let source = source.clone();
                #waterui::reactive::Binding::mapping(
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
        impl #impl_generics_with_static #waterui::reactive::project::Project for #struct_name #ty_generics #where_clause {
            type Projected = #projected_tuple;

            fn project(source: &#waterui::reactive::Binding<Self>) -> Self::Projected {
                #projection_tuple
            }
        }
    };

    TokenStream::from(expanded)
}

fn derive_project_unit_struct(input: &DeriveInput, waterui: &TokenStream2) -> TokenStream {
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
        impl #impl_generics_with_static #waterui::reactive::project::Project for #struct_name #ty_generics #where_clause {
            type Projected = ();

            fn project(_source: &#waterui::reactive::Binding<Self>) -> Self::Projected {
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
/// ```rust,ignore
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
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
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
                        use #waterui::reactive::SignalExt;
                        SignalExt::map(#arg.clone(), |arg| #waterui::reactive::__format!(#format_str, arg))
                    }
                }
                .into()
            }
            2 => {
                let arg1 = &args[0];
                let arg2 = &args[1];
                quote! {
                    {
                        use #waterui::reactive::{SignalExt, zip::zip};
                        let __arg1 = #arg1.clone();
                        let __arg2 = #arg2.clone();
                        SignalExt::map(zip(__arg1, &__arg2), |(arg1, arg2)| {
                            #waterui::reactive::__format!(#format_str, arg1, arg2)
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
                        use #waterui::reactive::{SignalExt, zip::zip};
                        let __arg1 = #arg1.clone();
                        let __arg2 = #arg2.clone();
                        let __arg3 = #arg3.clone();
                        let __inner = zip(__arg1, &__arg2);
                        SignalExt::map(
                            zip(__inner, &__arg3),
                            |((arg1, arg2), arg3)| #waterui::reactive::__format!(#format_str, arg1, arg2, arg3)
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
                        use #waterui::reactive::{SignalExt, zip::zip};
                        let __arg1 = #arg1.clone();
                        let __arg2 = #arg2.clone();
                        let __arg3 = #arg3.clone();
                        let __arg4 = #arg4.clone();
                        let __inner1 = zip(__arg1, &__arg2);
                        let __inner2 = zip(__arg3, &__arg4);
                        SignalExt::map(
                            zip(__inner1, &__inner2),
                            |((arg1, arg2), (arg3, arg4))| #waterui::reactive::__format!(#format_str, arg1, arg2, arg3, arg4)
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
                use #waterui::reactive::constant;
                constant(#waterui::reactive::__format!(#format_str))
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
                    use #waterui::reactive::SignalExt;
                    let __var = #var.clone();
                    SignalExt::map(&__var, |#var| {
                        #waterui::reactive::__format!(#format_str)
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
                    use #waterui::reactive::{SignalExt, zip::zip};
                    let __var1 = #var1.clone();
                    let __var2 = #var2.clone();
                    let __zipped = zip(__var1, &__var2);
                    SignalExt::map(&__zipped, |(#var1, #var2)| {
                        #waterui::reactive::__format!(#format_str)
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
                    use #waterui::reactive::{SignalExt, zip::zip};
                    let __var1 = #var1.clone();
                    let __var2 = #var2.clone();
                    let __var3 = #var3.clone();
                    let __inner = zip(__var1, &__var2);
                    let __zipped = zip(__inner, &__var3);
                    SignalExt::map(
                        &__zipped,
                        |((#var1, #var2), #var3)| {
                            #waterui::reactive::__format!(#format_str)
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
                    use #waterui::reactive::{SignalExt, zip::zip};
                    let __var1 = #var1.clone();
                    let __var2 = #var2.clone();
                    let __var3 = #var3.clone();
                    let __var4 = #var4.clone();
                    let __inner1 = zip(__var1, &__var2);
                    let __inner2 = zip(__var3, &__var4);
                    let __zipped = zip(__inner1, &__inner2);
                    SignalExt::map(
                        &__zipped,
                        |((#var1, #var2), (#var3, #var4))| {
                            #waterui::reactive::__format!(#format_str)
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

/// Attribute macro for enabling view preview functionality.
///
/// This macro marks a function as previewable, generating a C-exported symbol that
/// the preview system can load to render the view without running the full app.
///
/// # Example
///
/// ```ignore
/// use waterui::prelude::*;
///
/// // Simple function with no arguments
/// #[preview]
/// fn sidebar() -> impl View {
///     vstack((
///         text("Sidebar"),
///         text("Content"),
///     ))
/// }
///
/// // Function with arguments - provide defaults in the macro
/// #[preview(count = 5, name = "John")]
/// fn user_card(count: i32, name: &str) -> impl View {
///     text(format!("{name}: {count}"))
/// }
/// ```
///
/// # How It Works
///
/// The macro generates a C-exported symbol with the naming pattern:
/// `waterui_preview_<crate_name>_<function_name>`
///
/// For example, `card` in `my-crate` becomes `waterui_preview_my_crate_card`.
///
/// This symbol can be loaded by the preview support app to render the view.
///
/// # Symbol Naming
///
/// Preview function names must be unique within a crate because Rust procedural macros do not
/// receive the surrounding module path. The CLI reports duplicate names during `--all` discovery.
///
/// This allows even private functions to be previewable since the symbol is always public.
///
/// # Panics
///
/// Panics during macro expansion only if internal parameter-default validation becomes
/// inconsistent after the explicit presence checks above.
#[proc_macro_attribute]
pub fn preview(args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    let args = parse_macro_input!(args with Punctuated::<PreviewArg, Token![,]>::parse_terminated);

    let fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_attrs = &input_fn.attrs;
    let fn_sig = &input_fn.sig;
    let fn_block = &input_fn.block;
    let fn_name_str = fn_name.to_string();

    // Parse function parameters and collect default values from macro args
    let params = input_fn
        .sig
        .inputs
        .iter()
        .map(|arg| match arg {
            syn::FnArg::Typed(pat_type) => {
                let syn::Pat::Ident(pat_ident) = &*pat_type.pat else {
                    return Err(syn::Error::new_spanned(
                        &pat_type.pat,
                        "`#[preview]` parameters must use identifier patterns",
                    ));
                };
                Ok(pat_ident.ident.clone())
            }
            syn::FnArg::Receiver(receiver) => Err(syn::Error::new_spanned(
                receiver,
                "`#[preview]` can only be applied to free functions",
            )),
        })
        .collect::<syn::Result<Vec<_>>>();
    let params = match params {
        Ok(params) => params,
        Err(error) => return error.to_compile_error().into(),
    };

    // Build a map of parameter defaults from the macro arguments
    let mut defaults = HashMap::<String, Expr>::with_capacity(args.len());
    for arg in args {
        let name = arg.name.to_string();
        if !params.iter().any(|param| param == &arg.name) {
            return syn::Error::new_spanned(
                arg.name,
                format!("unknown `#[preview]` parameter default `{name}`"),
            )
            .to_compile_error()
            .into();
        }
        if defaults.insert(name.clone(), arg.value).is_some() {
            return syn::Error::new_spanned(
                arg.name,
                format!("duplicate `#[preview]` parameter default `{name}`"),
            )
            .to_compile_error()
            .into();
        }
    }

    // Check that all parameters have defaults if the function has arguments
    if !params.is_empty() {
        for param_name in &params {
            if !defaults.contains_key(&param_name.to_string()) {
                return syn::Error::new_spanned(
                    param_name,
                    format!(
                        "Function parameter `{param_name}` needs a default value in #[preview({param_name} = ...)]"
                    ),
                )
                .to_compile_error()
                .into();
            }
        }
    }

    // Generate the call expression with defaults
    let call_args: Vec<_> = params
        .iter()
        .map(|name| {
            defaults
                .get(&name.to_string())
                .cloned()
                .expect("checked above")
        })
        .collect();

    let call_expr = if params.is_empty() {
        quote! { #fn_name() }
    } else {
        quote! { #fn_name(#(#call_args),*) }
    };

    // Generate the export function - we use a helper macro to construct the symbol name
    // at compile time using CARGO_PKG_NAME
    let expanded = quote! {
        #(#fn_attrs)*
        #fn_vis #fn_sig #fn_block

        // Generate C export symbol for preview
        // Symbol name: waterui_preview_<crate_name>_<fn_name>
        // Uses a helper that expands CARGO_PKG_NAME at compile time
        #waterui::__export_preview!(#fn_name_str, {
            let view = #call_expr;
            Box::into_raw(Box::new(#waterui::AnyView::new(view))).cast()
        });
    };

    TokenStream::from(expanded)
}

/// A single argument in the preview macro like `name = "value"`
struct PreviewArg {
    name: syn::Ident,
    value: Expr,
}

impl Parse for PreviewArg {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let value: Expr = input.parse()?;
        Ok(Self { name, value })
    }
}

fn testing_crate_path() -> syn::Result<proc_macro2::TokenStream> {
    match crate_name("waterui-testing") {
        Ok(FoundCrate::Itself) => Ok(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(_) => Err(syn::Error::new(
            Span::call_site(),
            "`#[waterui::test(...)]` and `#[waterui::bench(...)]` require the `waterui-testing` crate as a dependency (it may be renamed in Cargo.toml).",
        )),
    }
}

fn is_unit_output(output: &syn::ReturnType) -> bool {
    match output {
        syn::ReturnType::Default => true,
        syn::ReturnType::Type(_, ty) => {
            matches!(ty.as_ref(), syn::Type::Tuple(tuple) if tuple.elems.is_empty())
        }
    }
}

struct WateruiTestArgs {
    /// `Some` mounts this no-arg view function; `None` is the manual-mount
    /// form whose test function receives the configured `UiBuilder` by value.
    view: Option<syn::Path>,
    theme: Option<Expr>,
    viewport: Option<(Expr, Expr)>,
    offscreen: bool,
}

impl WateruiTestArgs {
    fn parse_named(&mut self, input: syn::parse::ParseStream) -> syn::Result<()> {
        let name: syn::Ident = input.parse()?;
        if name == "offscreen" {
            self.offscreen = true;
            return Ok(());
        }
        input.parse::<Token![=]>()?;
        if name == "theme" {
            self.theme = Some(input.parse()?);
            return Ok(());
        }
        if name == "viewport" {
            let content;
            syn::parenthesized!(content in input);
            let width: Expr = content.parse()?;
            content.parse::<Token![,]>()?;
            let height: Expr = content.parse()?;
            self.viewport = Some((width, height));
            return Ok(());
        }
        Err(syn::Error::new_spanned(
            name,
            "`#[waterui::test(...)]` accepts an optional view function path followed by `theme = <installer>`, `viewport = (width, height)`, and `offscreen`",
        ))
    }

    /// Whether the next tokens are a named argument (`ident =` or the bare
    /// `offscreen` flag) rather than the leading view function path.
    fn peeks_named(input: syn::parse::ParseStream) -> bool {
        let fork = input.fork();
        let Ok(ident) = fork.parse::<syn::Ident>() else {
            return false;
        };
        if ident == "offscreen" && (fork.is_empty() || fork.peek(Token![,])) {
            return true;
        }
        fork.peek(Token![=])
    }
}

impl Parse for WateruiTestArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = Self {
            view: None,
            theme: None,
            viewport: None,
            offscreen: false,
        };
        let mut first = true;
        while !input.is_empty() {
            if !first {
                input.parse::<Token![,]>()?;
                if input.is_empty() {
                    break;
                }
            }
            if first && !Self::peeks_named(input) {
                args.view = Some(input.parse()?);
            } else {
                args.parse_named(input)?;
            }
            first = false;
        }
        if args.offscreen && args.view.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "`offscreen` only applies when the macro mounts a view function; the manual-mount form calls `mount_offscreen` itself",
            ));
        }
        Ok(args)
    }
}

fn parse_test_view_arg(args: TokenStream) -> Result<WateruiTestArgs, TokenStream> {
    syn::parse::<WateruiTestArgs>(args).map_err(|err| err.to_compile_error().into())
}

fn single_typed_parameter<'a>(
    input_fn: &'a ItemFn,
    macro_label: &str,
    expected: &str,
) -> Result<&'a syn::PatType, TokenStream> {
    if input_fn.sig.inputs.len() != 1 {
        return Err(syn::Error::new_spanned(
            &input_fn.sig.inputs,
            format!("`{macro_label}` test function must take exactly one parameter: {expected}"),
        )
        .to_compile_error()
        .into());
    }

    match input_fn.sig.inputs.first() {
        Some(syn::FnArg::Typed(arg)) => Ok(arg),
        _ => Err(syn::Error::new_spanned(
            &input_fn.sig.inputs,
            format!("`{macro_label}` test function must take one explicit parameter: {expected}"),
        )
        .to_compile_error()
        .into()),
    }
}

/// The mounting form takes its session parameter by `&mut`.
fn validate_mounted_parameter<'a>(
    input_fn: &'a ItemFn,
    macro_label: &str,
    expected: &str,
) -> Result<&'a syn::PatType, TokenStream> {
    let typed_arg = single_typed_parameter(input_fn, macro_label, expected)?;

    let syn::Type::Reference(reference) = typed_arg.ty.as_ref() else {
        return Err(syn::Error::new_spanned(
            &typed_arg.ty,
            format!("`{macro_label}` parameter must be a mutable reference: {expected}"),
        )
        .to_compile_error()
        .into());
    };
    if reference.mutability.is_none() {
        return Err(syn::Error::new_spanned(
            &typed_arg.ty,
            format!("`{macro_label}` parameter must be `&mut ...`"),
        )
        .to_compile_error()
        .into());
    }

    Ok(typed_arg)
}

/// The manual-mount form takes the configured `UiBuilder` by value.
fn validate_builder_parameter<'a>(
    input_fn: &'a ItemFn,
    macro_label: &str,
) -> Result<&'a syn::PatType, TokenStream> {
    let typed_arg = single_typed_parameter(input_fn, macro_label, "the `UiBuilder` by value")?;

    if matches!(typed_arg.ty.as_ref(), syn::Type::Reference(_)) {
        return Err(syn::Error::new_spanned(
            &typed_arg.ty,
            format!(
                "the manual-mount form of `{macro_label}` takes the `UiBuilder` by value; mount it in the test body"
            ),
        )
        .to_compile_error()
        .into());
    }

    Ok(typed_arg)
}

/// Signature checks shared by `#[waterui::test]` and `#[waterui::bench]`.
fn validate_harness_fn_shape(input_fn: &ItemFn, macro_label: &str) -> Result<(), TokenStream> {
    if input_fn.sig.constness.is_some() {
        return Err(syn::Error::new_spanned(
            input_fn.sig.constness,
            format!("`{macro_label}` does not support const functions"),
        )
        .to_compile_error()
        .into());
    }
    if let syn::Safety::Unsafe(unsafe_token) = &input_fn.sig.safety {
        return Err(syn::Error::new_spanned(
            unsafe_token,
            format!("`{macro_label}` does not support unsafe functions"),
        )
        .to_compile_error()
        .into());
    }
    if input_fn.sig.abi.is_some() {
        return Err(syn::Error::new_spanned(
            &input_fn.sig.abi,
            format!("`{macro_label}` does not support extern ABI"),
        )
        .to_compile_error()
        .into());
    }
    if input_fn.sig.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            &input_fn.sig.variadic,
            format!("`{macro_label}` does not support variadic arguments"),
        )
        .to_compile_error()
        .into());
    }
    if !input_fn.sig.generics.params.is_empty() || input_fn.sig.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &input_fn.sig.generics,
            format!("`{macro_label}` does not support generic test functions"),
        )
        .to_compile_error()
        .into());
    }
    for attr in &input_fn.attrs {
        if attr.path().is_ident("test") {
            return Err(syn::Error::new_spanned(
                attr,
                format!("Do not combine `#[test]` with `{macro_label}`"),
            )
            .to_compile_error()
            .into());
        }
    }
    Ok(())
}

fn validate_test_fn(input_fn: &ItemFn, mounts_view: bool) -> Result<&syn::PatType, TokenStream> {
    const LABEL: &str = "#[waterui::test(...)]";
    validate_harness_fn_shape(input_fn, LABEL)?;
    if !is_unit_output(&input_fn.sig.output) {
        return Err(syn::Error::new_spanned(
            &input_fn.sig.output,
            "`#[waterui::test(...)]` test functions must not return a value",
        )
        .to_compile_error()
        .into());
    }

    if mounts_view {
        validate_mounted_parameter(input_fn, LABEL, "a `&mut` app session")
    } else {
        validate_builder_parameter(input_fn, LABEL)
    }
}

/// Attribute macro for `WaterUI` accessibility-first unit tests.
///
/// # Mounting form
///
/// A no-arg view function path mounts before the test body runs; the test
/// function receives the app session by `&mut`. `theme = <installer>` swaps
/// the theme package, `viewport = (width, height)` sizes the window, and the
/// bare `offscreen` flag mounts the GPU-backed offscreen runtime (the test
/// then takes `&mut OffscreenApp`).
///
/// ```ignore
/// fn login_view() -> impl View {
///     // ...
/// }
///
/// #[waterui::test(login_view, theme = hydrolysis_m3::install, viewport = (360, 320))]
/// fn login_flow(app: &mut waterui_testing::SemanticApp) {
///     app.query().role(waterui_testing::Role::BUTTON).label("Login").tap();
/// }
/// ```
///
/// # Manual-mount form
///
/// Without a view path the test function receives the configured
/// [`UiBuilder`](https://docs.rs/waterui-testing) by value and mounts in its
/// own body — the form for tests that own `Binding`s the view closes over.
///
/// ```ignore
/// #[waterui::test(theme = hydrolysis_m3::install)]
/// fn stepper_updates_binding(ui: waterui_testing::UiBuilder) {
///     let value = Binding::i32(2);
///     let value_for_view = value.clone();
///     let mut app = ui.mount(move || stepper("Limited", &value_for_view));
///     app.query().label("Limited").increment();
///     assert_eq!(value.get(), 3);
/// }
/// ```
///
/// The macro always expands into a regular `#[test]` wrapper.
#[proc_macro_attribute]
pub fn ui_test(args: TokenStream, input: TokenStream) -> TokenStream {
    let test_args = match parse_test_view_arg(args) {
        Ok(test_args) => test_args,
        Err(err) => return err,
    };
    let input_fn = parse_macro_input!(input as ItemFn);
    let typed_arg = match validate_test_fn(&input_fn, test_args.view.is_some()) {
        Ok(typed_arg) => typed_arg,
        Err(err) => return err,
    };

    let testing_path = match testing_crate_path() {
        Ok(path) => path,
        Err(error) => return error.to_compile_error().into(),
    };

    let attrs = &input_fn.attrs;
    let visibility = &input_fn.vis;
    let fn_name = &input_fn.sig.ident;
    let fn_body = &input_fn.block;
    let arg_pattern = &typed_arg.pat;
    let arg_type = &typed_arg.ty;
    let async_wrapper = input_fn.sig.asyncness.is_some();

    let mut builder = quote! { #testing_path::ui() };
    if let Some((width, height)) = &test_args.viewport {
        builder = quote! { #builder.viewport(#width, #height) };
    }
    if let Some(theme) = &test_args.theme {
        builder = quote! { #builder.theme(#theme) };
    }

    let bind_arg = if let Some(view_fn) = &test_args.view {
        let mount = if test_args.offscreen {
            quote! { mount_offscreen }
        } else {
            quote! { mount }
        };
        quote! {
            let mut __waterui_test_mounted_app = #builder.#mount(#view_fn);
            let #arg_pattern: #arg_type = &mut __waterui_test_mounted_app;
        }
    } else {
        quote! {
            let #arg_pattern: #arg_type = #builder;
        }
    };

    let run_body = if async_wrapper {
        quote! {
            #testing_path::block_on(async {
                #bind_arg
                #fn_body
            });
        }
    } else {
        quote! {
            #bind_arg
            #fn_body
        }
    };

    let expanded = quote! {
        #(#attrs)*
        #[test]
        #visibility fn #fn_name() {
            #run_body
        }
    };

    TokenStream::from(expanded)
}

struct WateruiBenchArgs {
    /// `Some` mounts this no-arg view function; `None` is the manual-mount
    /// form whose bench function receives the configured `UiBuilder` by value
    /// and returns the recorded `PerfReport`.
    view: Option<syn::Path>,
    theme: Option<Expr>,
    viewport: Option<(Expr, Expr)>,
    budgets: [(&'static str, Option<Expr>); 6],
}

const BENCH_BUDGET_ARGS: [&str; 6] = [
    "max_p95_us",
    "max_mean_us",
    "max_rebuild_ratio",
    "max_scene_layers",
    "max_gpu_surface_layers",
    "max_clip_layers",
];

impl WateruiBenchArgs {
    fn parse_named(&mut self, input: syn::parse::ParseStream) -> syn::Result<()> {
        let name: syn::Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        if name == "theme" {
            self.theme = Some(input.parse()?);
            return Ok(());
        }
        if name == "viewport" {
            let content;
            syn::parenthesized!(content in input);
            let width: Expr = content.parse()?;
            content.parse::<Token![,]>()?;
            let height: Expr = content.parse()?;
            self.viewport = Some((width, height));
            return Ok(());
        }
        if let Some((slot_name, slot)) = self
            .budgets
            .iter_mut()
            .find(|(slot_name, _)| name == slot_name)
        {
            if slot.is_some() {
                return Err(syn::Error::new_spanned(
                    name,
                    format!("duplicate `#[waterui::bench(...)]` budget `{slot_name}`"),
                ));
            }
            *slot = Some(input.parse()?);
            return Ok(());
        }
        Err(syn::Error::new_spanned(
            name,
            "`#[waterui::bench(...)]` accepts an optional view function path followed by `theme = <installer>`, `viewport = (width, height)`, and the budgets `max_p95_us`, `max_mean_us`, `max_rebuild_ratio`, `max_scene_layers`, `max_gpu_surface_layers`, `max_clip_layers`",
        ))
    }

    /// Whether the next tokens are a named `ident = ...` argument rather than
    /// the leading view function path.
    fn peeks_named(input: syn::parse::ParseStream) -> bool {
        let fork = input.fork();
        if fork.parse::<syn::Ident>().is_err() {
            return false;
        }
        fork.peek(Token![=])
    }
}

impl Parse for WateruiBenchArgs {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut args = Self {
            view: None,
            theme: None,
            viewport: None,
            budgets: BENCH_BUDGET_ARGS.map(|name| (name, None)),
        };
        let mut first = true;
        while !input.is_empty() {
            if !first {
                input.parse::<Token![,]>()?;
                if input.is_empty() {
                    break;
                }
            }
            if first && !Self::peeks_named(input) {
                args.view = Some(input.parse()?);
            } else {
                args.parse_named(input)?;
            }
            first = false;
        }
        Ok(args)
    }
}

fn validate_bench_fn(input_fn: &ItemFn, mounts_view: bool) -> Result<&syn::PatType, TokenStream> {
    const LABEL: &str = "#[waterui::bench(...)]";
    validate_harness_fn_shape(input_fn, LABEL)?;
    if input_fn.sig.asyncness.is_some() {
        return Err(syn::Error::new_spanned(
            input_fn.sig.asyncness,
            "`#[waterui::bench(...)]` does not support async functions; drive frames through `PerfRun` instead",
        )
        .to_compile_error()
        .into());
    }

    if mounts_view {
        if !is_unit_output(&input_fn.sig.output) {
            return Err(syn::Error::new_spanned(
                &input_fn.sig.output,
                "the mounting form of `#[waterui::bench(...)]` must not return a value; record scenarios through `&mut PerfApp`",
            )
            .to_compile_error()
            .into());
        }
        validate_mounted_parameter(input_fn, LABEL, "`&mut PerfApp`")
    } else {
        if is_unit_output(&input_fn.sig.output) {
            return Err(syn::Error::new_spanned(
                &input_fn.sig.output,
                "the manual form of `#[waterui::bench(...)]` must return the recorded `PerfReport`",
            )
            .to_compile_error()
            .into());
        }
        validate_builder_parameter(input_fn, LABEL)
    }
}

/// Attribute macro for `WaterUI` GPU frame benchmarks.
///
/// Expands into a plain `#[test]` named `waterui_bench_<name>` (that prefix is
/// how `water bench` discovers benches through `cargo nextest`). Without the
/// `WATERUI_BENCH_*` environment variables the bench runs in smoke mode
/// (0 warmups, 2 samples, 1 repetition, no budget enforcement), so a plain
/// `cargo nextest run` keeps every bench compiling and executing as a cheap
/// correctness test. `water bench` sets the environment for full measurement
/// runs, where the budget arguments are enforced in-process — a blown budget
/// is a failed test.
///
/// # Mounting form
///
/// A no-arg view function path mounts offscreen before the bench body runs;
/// the bench function receives `&mut PerfApp` and records scenarios with
/// `measure`. `theme = <installer>` swaps the theme package and
/// `viewport = (width, height)` sizes the window.
///
/// ```ignore
/// fn dashboard() -> impl View {
///     // ...
/// }
///
/// #[waterui::bench(dashboard, theme = hydrolysis_m3::install, viewport = (390, 844), max_p95_us = 8_000)]
/// fn dashboard_redraw(perf: &mut waterui_testing::PerfApp) {
///     perf.measure("steady-redraw", |run| run.redraw());
/// }
/// ```
///
/// # Manual form
///
/// Without a view path the bench function receives the configured
/// `UiBuilder` by value, drives `perf_with` itself, and returns the
/// `PerfReport` — the form for benches that own `Binding`s the view closes
/// over.
///
/// ```ignore
/// #[waterui::bench(theme = hydrolysis_m3::install, max_rebuild_ratio = 0.2)]
/// fn counter_updates(ui: waterui_testing::UiBuilder) -> waterui_testing::PerfReport {
///     let value = Binding::i32(0);
///     let value_for_view = value.clone();
///     ui.perf_with(move || counter(&value_for_view), |perf| {
///         perf.measure("increment", |run| {
///             run.app().query().label("Increment").tap();
///             let _ = run;
///         });
///     })
/// }
/// ```
///
/// # Budgets
///
/// `max_p95_us`, `max_mean_us`, `max_rebuild_ratio`, `max_scene_layers`,
/// `max_gpu_surface_layers`, and `max_clip_layers` are all optional and apply
/// to every measurement the bench records. `water bench` can only tighten
/// them further via its `--max-*` flags.
#[proc_macro_attribute]
pub fn bench(args: TokenStream, input: TokenStream) -> TokenStream {
    let bench_args = match syn::parse::<WateruiBenchArgs>(args) {
        Ok(bench_args) => bench_args,
        Err(err) => return err.to_compile_error().into(),
    };
    let input_fn = parse_macro_input!(input as ItemFn);
    let typed_arg = match validate_bench_fn(&input_fn, bench_args.view.is_some()) {
        Ok(typed_arg) => typed_arg,
        Err(err) => return err,
    };

    let testing_path = match testing_crate_path() {
        Ok(path) => path,
        Err(error) => return error.to_compile_error().into(),
    };

    let attrs = &input_fn.attrs;
    let visibility = &input_fn.vis;
    let bench_name = input_fn.sig.ident.to_string();
    let test_fn_name = syn::Ident::new(
        &format!("waterui_bench_{bench_name}"),
        input_fn.sig.ident.span(),
    );
    let fn_body = &input_fn.block;
    let arg_pattern = &typed_arg.pat;
    let arg_type = &typed_arg.ty;

    let mut builder = quote! { #testing_path::ui() };
    if let Some((width, height)) = &bench_args.viewport {
        builder = quote! { #builder.viewport(#width, #height) };
    }
    if let Some(theme) = &bench_args.theme {
        builder = quote! { #builder.theme(#theme) };
    }
    builder = quote! { #builder.perf_config(__waterui_bench_config) };

    let budget_fields = bench_args.budgets.iter().map(|(name, value)| {
        let field = syn::Ident::new(name, Span::call_site());
        value.as_ref().map_or_else(
            || quote! { #field: ::core::option::Option::None },
            |expr| quote! { #field: ::core::option::Option::Some(#expr) },
        )
    });
    let budgets = quote! {
        #testing_path::bench::BenchBudgets { #(#budget_fields),* }
    };

    let run_closure = bench_args.view.as_ref().map_or_else(
        || {
            quote! {
                |__waterui_bench_config| {
                    let #arg_pattern: #arg_type = #builder;
                    let __waterui_bench_report: #testing_path::PerfReport = #fn_body;
                    __waterui_bench_report
                }
            }
        },
        |view_fn| {
            quote! {
                |__waterui_bench_config| {
                    #builder.perf_with(#view_fn, |__waterui_bench_perf| {
                        let #arg_pattern: #arg_type = __waterui_bench_perf;
                        #fn_body
                    })
                }
            }
        },
    );

    let expanded = quote! {
        #(#attrs)*
        #[test]
        #visibility fn #test_fn_name() {
            #testing_path::bench::run_bench(
                env!("CARGO_PKG_NAME"),
                #bench_name,
                #budgets,
                #run_closure,
            );
        }
    };

    TokenStream::from(expanded)
}
