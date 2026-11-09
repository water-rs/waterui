#![doc = include_str!("../README.md")]
//! Procedural macros for `WaterUI` framework.
//!
//! This crate provides derive macros and procedural macros for the `WaterUI` framework,
//! including form generation, reactive signal formatting, and view builder patterns.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
#[cfg(feature = "widgets")]
use sha2::Digest as _;
use std::collections::HashMap;
use syn::{Data, DeriveInput, Fields, ItemFn, Meta, parse_macro_input};
mod apple_available;
mod identifiable;
mod locale;
mod state;
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

/// Selects between two realizations of a component by Apple OS version.
///
/// `apple_available!(27.0)` is `objc2::available!` with that one version for
/// iOS, macOS, tvOS, watchOS and visionOS: Apple OS versions are unified across
/// them since 26, so one number covers every platform and none can be left
/// out. The version must be one the framework gates (`waterui-apple-os` lists
/// them); an unknown version does not compile.
///
/// When the build's Apple deployment target already meets the version the
/// check is statically `true`, and the call site gets a `#[deprecated]`
/// warning: the older realization is dead and is deleted when the framework
/// floor reaches the version.
///
/// The crate using it depends on `waterui-apple-os`, and only builds for Apple
/// targets.
#[proc_macro]
pub fn apple_available(input: TokenStream) -> TokenStream {
    apple_available::expand(input.into()).into()
}

mod javascript;
mod js_api;
mod ts;
mod tsx;

/// Exposes an `impl` block to the page.
///
/// ```rust
/// # use waterui::webview::{Json, WebView, serde};
/// # use waterui::{Binding, Str, Url, View, js_api};
/// # #[derive(serde::Serialize)]
/// # #[serde(crate = "waterui::webview::serde")]
/// # struct Greeting {
/// #     text: String,
/// # }
/// # #[derive(Debug)]
/// # struct ApiError;
/// # impl core::fmt::Display for ApiError {
/// #     fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
/// #         f.write_str("api error")
/// #     }
/// # }
/// # type Theme = Str;
/// # struct App {
/// #     theme: Binding<Theme>,
/// # }
/// #[js_api(namespace = "app")]
/// impl App {
///     fn theme(&self) -> Binding<Theme> { self.theme.clone() }
///     async fn greet(&self, name: String) -> Result<Json<Greeting>, ApiError> {
/// #       let _ = &name;
///         // ...
/// #       Ok(Json(Greeting { text: name }))
///     }
/// }
///
/// # fn page(url: Url, app: App) -> impl View {
/// WebView::open(url).serve(app)
/// # }
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
/// ```rust
/// # use waterui::Str;
/// # use waterui::exec;
/// # use waterui::webview::{JsError, WebView};
/// # async fn apply(webview: &WebView, theme: Str) -> Result<(), JsError> {
/// exec!(webview, "app.setTheme(@{theme})").await?;
/// # Ok(())
/// # }
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
/// ```rust
/// # use waterui::eval;
/// # use waterui::webview::{JsError, WebView};
/// # async fn title_of(webview: &WebView) -> Result<String, JsError> {
/// let title: String = eval!(webview, "document.title").await?;
/// # Ok(title)
/// # }
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
/// ```rust
/// # use waterui::webview::{JsError, ScriptInjectionTime, WebView, WebViewOpen};
/// # use waterui::{exec_file, js_file};
/// # async fn setup(webview: &WebView) -> Result<(), JsError> {
/// webview.exec(&js_file!("scripts/setup.js")).await?;
/// exec_file!(webview, "scripts/setup.js").await?; // the fused form
/// # Ok(())
/// # }
/// # fn on_every_page() -> WebViewOpen {
/// WebView::open("https://waterui.dev").inject(
///     "setup",
///     js_file!("scripts/setup.js"),
///     ScriptInjectionTime::DocumentStart,
/// )
/// # }
/// ```
///
/// Multi-line JavaScript belongs in a file rather than a string literal, and this
/// is how it gets there.
///
/// The same program runs either way. `exec` sends it once to a page that is
/// already loaded; `inject` registers it to run on every load, rendering it
/// through `JsProgram::standalone_script` on the way.
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
/// ```rust
/// # use waterui::prelude::*;
/// # fn labelled(local_var: Str) -> impl View {
/// text!("{slot}", slot = local_var)
/// # }
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

/// Marks an owned `Clone` type as a `waterui::extract::Extractor` over the
/// `.state(&value)` injection channel.
///
/// `State<T>` is the wrapper for a type the app cannot implement traits for —
/// `Binding<Str>`, a third-party value. Naming it for a type the app *does*
/// own reads as an anti-pattern: mark the type `#[state]` once and handlers
/// take the value bare.
///
/// ```rust
/// use waterui::prelude::*;
///
/// #[state]
/// #[derive(Clone)]
/// struct Editor {
///     doc: Binding<Str>,
/// }
/// # impl Editor {
/// #     fn save(&self) {}
/// # }
///
/// # fn view(editor: &Editor) -> impl View {
/// button("Save")
///     .action(|editor: Editor| editor.save())
///     .state(editor)
/// # }
/// ```
///
/// The generated `Extractor` implementation delegates to
/// `waterui::extract::State<Self>`, so `.state(&value)` remains the
/// injection mechanism and a bare `T` parameter shares extraction positions
/// with `State<T>` parameters of the same type — the first `.state()` call
/// feeds the first parameter of that type.
///
/// The type must be `Clone + 'static`; the requirement is enforced at the
/// attribute so a missing `Clone` reports here rather than inside the
/// expansion. For an owned type that should read a value installed directly in
/// the environment instead of through `.state()`, use
/// `waterui::impl_extractor!`.
#[proc_macro_attribute]
pub fn state(args: TokenStream, input: TokenStream) -> TokenStream {
    state::expand(args, input)
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
/// ```rust
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

    // Generate the implementation.
    //
    // The trailing comma inside the inner repetition matters: `#(T),*` over a
    // single field emits `(V)`, which is a parenthesized type rather than a
    // one-element tuple, so a single-field form declared `VStack<(V,)>` while
    // `#view_body` built `VStack<((V,),)>`. Spelling the repetition the same
    // way the body does keeps the two in step at every arity, including zero.
    let expanded = quote! {
        impl #waterui::FormBuilder for #name {
            type View = #waterui::component::stack::VStack<((#(<#field_types as #waterui::FormBuilder>::View,)*),)>;

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
///
/// # Example
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::reactive::binding;
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
///     let form_binding: Binding<UserForm> = binding(UserForm::default());
///     form(&form_binding)
/// }
/// ```
///
/// This is equivalent to manually writing:
///
/// ```rust
/// use waterui::prelude::*;
///
/// #[derive(Default, Clone, Debug, FormBuilder, Project)]
/// pub struct UserForm {
///     pub name: String,
///     pub age: i32,
///     pub notifications: bool,
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

use syn::{Expr, Token, Type, parse::Parse, punctuated::Punctuated};

/// Derive macro for implementing the `Project` trait on structs.
///
/// This macro automatically generates a `Project` implementation that allows
/// decomposing a struct binding into separate bindings for each field.
///
/// # Examples
///
/// ```rust
/// use waterui::Signal;
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
/// let person = person_binding.snapshot();
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

    // The projection mirrors its source field for field, so it is exactly as
    // visible and as exhaustive as the struct it projects.
    let visibility = &input.vis;
    let exhaustiveness = if input
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("non_exhaustive"))
    {
        quote! { #[non_exhaustive] }
    } else {
        quote! {}
    };

    let expanded = quote! {
        /// Projected version of #struct_name with each field wrapped in a Binding.
        #[derive(Debug)]
        #exhaustiveness
        #visibility struct #projected_struct_name #ty_generics #where_clause {
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

/// Attribute macro for enabling view preview functionality.
///
/// This macro marks a function as previewable, generating a C-exported symbol that
/// the preview system can load to render the view without running the full app.
///
/// # Example
///
/// ```rust
/// use waterui::prelude::*;
/// use waterui::preview;
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
/// This symbol is what a generated preview binary resolves to render the view.
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
        // The preview export is gated on `debug_assertions` (the gate the
        // `waterui_meta_*` records use); a release build has no preview host,
        // so without the export the function is dead code there.
        #[cfg_attr(not(debug_assertions), allow(dead_code))]
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

/// The `WidgetKit` families a `#[widget]` may declare — the serialized
/// `WidgetFamily` spellings the extension renders.
#[cfg(feature = "widgets")]
const WIDGET_FAMILIES: &[&str] = &[
    "systemSmall",
    "systemMedium",
    "systemLarge",
    "systemExtraLarge",
    "accessoryCircular",
    "accessoryRectangular",
    "accessoryInline",
];

/// A single `#[widget]` argument: `name = "…"`, `description = "…"`,
/// `kind = "…"`, or `families = [systemSmall, …]`.
#[cfg(feature = "widgets")]
struct WidgetArg {
    name: syn::Ident,
    value: Expr,
}

#[cfg(feature = "widgets")]
impl Parse for WidgetArg {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let name: syn::Ident = input.parse()?;
        input.parse::<Token![=]>()?;
        let value: Expr = input.parse()?;
        Ok(Self { name, value })
    }
}

#[cfg(feature = "widgets")]
fn widget_arg_string(arg: &WidgetArg) -> syn::Result<String> {
    if let Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(lit),
        ..
    }) = &arg.value
    {
        Ok(lit.value())
    } else {
        Err(syn::Error::new_spanned(
            &arg.value,
            format!("`#[widget]`'s `{}` must be a string literal", arg.name),
        ))
    }
}

/// Declares a `WidgetKit` widget: `#[waterui::widget(name = "…", description = "…")]`.
///
/// The attributed function produces the widget's timeline — either a
/// `Timeline`/`TimelineEntry` directly or any `WidgetContent`, which becomes
/// a single dated entry. It may take one `WidgetContext` parameter (the
/// family and display size `WidgetKit` hands the provider). Only the
/// `waterui::widgets` schema nodes are widget content; a `WaterUI` view in
/// that position is a compile-time error — `WidgetKit` renders the extension
/// from the serialized snapshot, and representable embedding cannot cross
/// its process boundary.
///
/// ```rust,ignore
/// use waterui::widgets::*;
///
/// #[waterui::widget(name = "Steps", description = "Today's step count")]
/// fn steps(context: WidgetContext) -> impl WidgetContent {
///     vstack((text("Steps"), text("6,241")))
/// }
/// ```
///
/// `kind` (the `WidgetConfiguration` kind) defaults to the function name;
/// `families` defaults to `[systemSmall, systemMedium, systemLarge]`.
///
/// The attribute emits the function's `waterui_widget_<crate>_<fn>` C
/// export and the `waterui_meta_widget_<crate>_<fn>` declaration record the
/// `water` CLI reads while it generates and packages the extension.
#[cfg(feature = "widgets")]
#[proc_macro_attribute]
pub fn widget(args: TokenStream, input: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(input as ItemFn);
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    let args = parse_macro_input!(args with Punctuated::<WidgetArg, Token![,]>::parse_terminated);

    let fn_name = &input_fn.sig.ident;
    let fn_vis = &input_fn.vis;
    let fn_attrs = &input_fn.attrs;
    let fn_sig = &input_fn.sig;
    let fn_block = &input_fn.block;
    let fn_name_str = fn_name.to_string();

    let params = match widget_params(&args) {
        Ok(params) => params,
        Err(error) => return error.to_compile_error().into(),
    };
    let context_mode = match widget_context_mode(&input_fn.sig) {
        Ok(mode) => mode,
        Err(error) => return error.to_compile_error().into(),
    };

    let call_expr = match context_mode {
        WidgetContextMode::None => quote! { #fn_name() },
        WidgetContextMode::ByValue => quote! { #fn_name(__wui_context) },
        WidgetContextMode::ByRef => quote! { #fn_name(&__wui_context) },
    };
    // A context-free function still decodes — the extension always sends a
    // context, and validating the wire form keeps the contract exercised —
    // but the binding stays unnamed so nothing warns on it.
    let context_binding = if matches!(context_mode, WidgetContextMode::None) {
        quote! {
            let _ = #waterui::widgets::ffi::decode_context(
                __wui_context_ptr,
                __wui_context_len,
            );
        }
    } else {
        quote! {
            let __wui_context: #waterui::widgets::WidgetContext =
                #waterui::widgets::ffi::decode_context(
                    __wui_context_ptr,
                    __wui_context_len,
                );
        }
    };

    let payload = widget_payload(&fn_name_str, &params);
    let payload_lit = syn::LitStr::new(&payload, Span::call_site());

    let expanded = quote! {
        #(#fn_attrs)*
        #fn_vis #fn_sig #fn_block

        #waterui::widgets::__export_widget!(
            #fn_name_str,
            #payload_lit,
            [__wui_context_ptr, __wui_context_len, __wui_out] => {
                #context_binding
                let __wui_timeline = #waterui::widgets::IntoTimeline::into_timeline(#call_expr);
                #waterui::widgets::ffi::Buffer::from_timeline(&__wui_timeline)
            }
        );
    };

    TokenStream::from(expanded)
}

/// The parsed `#[widget]` arguments after defaults.
#[cfg(feature = "widgets")]
struct WidgetParams {
    display_name: String,
    description: String,
    kind: String,
    families: Vec<String>,
}

/// A `#[widget]` function's `WidgetContext` parameter shape.
#[cfg(feature = "widgets")]
enum WidgetContextMode {
    None,
    ByValue,
    ByRef,
}

/// Validates the `#[widget]` attribute's arguments.
#[cfg(feature = "widgets")]
fn widget_params(args: &Punctuated<WidgetArg, Token![,]>) -> syn::Result<WidgetParams> {
    let mut display_name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut families: Option<Vec<String>> = None;
    for arg in args {
        match arg.name.to_string().as_str() {
            "name" | "description" | "kind" => {
                let value = widget_arg_string(arg)?;
                let slot = match arg.name.to_string().as_str() {
                    "name" => &mut display_name,
                    "description" => &mut description,
                    _ => &mut kind,
                };
                if slot.replace(value).is_some() {
                    return Err(syn::Error::new_spanned(
                        &arg.name,
                        format!("duplicate `#[widget]` argument `{}`", arg.name),
                    ));
                }
            }
            "families" => {
                let Expr::Array(array) = &arg.value else {
                    return Err(syn::Error::new_spanned(
                        &arg.value,
                        "`#[widget]`'s `families` must be a list like [systemSmall, systemMedium]",
                    ));
                };
                if array.elems.is_empty() {
                    return Err(syn::Error::new_spanned(
                        array,
                        "`#[widget]` requires at least one supported family",
                    ));
                }
                let mut parsed = Vec::with_capacity(array.elems.len());
                for element in &array.elems {
                    let Some(name) = widget_family_name(element) else {
                        return Err(syn::Error::new_spanned(
                            element,
                            "`#[widget]` families are `WidgetFamily` case names like `systemSmall`",
                        ));
                    };
                    parsed.push(name);
                }
                if families.replace(parsed).is_some() {
                    return Err(syn::Error::new_spanned(
                        &arg.name,
                        "duplicate `#[widget]`'s `families`",
                    ));
                }
            }
            other => {
                return Err(syn::Error::new_spanned(
                    &arg.name,
                    format!(
                        "unknown `#[widget]` argument `{other}`; expected `name`, `description`, `kind` or `families`"
                    ),
                ));
            }
        }
    }

    let Some(display_name) = display_name else {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[widget]` requires `name = \"…\"` — the title the widget gallery shows",
        ));
    };
    let Some(description) = description else {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[widget]` requires `description = \"…\"` — the subtitle the widget gallery shows",
        ));
    };
    Ok(WidgetParams {
        display_name,
        description,
        kind: kind.unwrap_or_default(),
        families: families.unwrap_or_else(|| {
            ["systemSmall", "systemMedium", "systemLarge"]
                .iter()
                .map(ToString::to_string)
                .collect()
        }),
    })
}

/// One `families` element: an ident naming a known `WidgetFamily` case.
#[cfg(feature = "widgets")]
fn widget_family_name(element: &Expr) -> Option<String> {
    let Expr::Path(path) = element else {
        return None;
    };
    let name = path.path.get_ident().map(ToString::to_string)?;
    WIDGET_FAMILIES.contains(&name.as_str()).then_some(name)
}

/// The function may take no parameter or one `WidgetContext` parameter (by
/// value or by reference).
#[cfg(feature = "widgets")]
fn widget_context_mode(sig: &syn::Signature) -> syn::Result<WidgetContextMode> {
    let is_widget_context = |ty: &Type| {
        if let Type::Path(path) = ty
            && let Some(segment) = path.path.segments.last()
        {
            return segment.ident == "WidgetContext";
        }
        false
    };
    match sig.inputs.len() {
        0 => Ok(WidgetContextMode::None),
        1 => {
            let Some(arg) = sig.inputs.iter().next() else {
                unreachable!("the signature's input count was checked above");
            };
            let syn::FnArg::Typed(pat_type) = arg else {
                return Err(syn::Error::new_spanned(
                    arg,
                    "`#[widget]` can only be applied to free functions",
                ));
            };
            match &*pat_type.ty {
                ty if is_widget_context(ty) => Ok(WidgetContextMode::ByValue),
                Type::Reference(reference) if is_widget_context(&reference.elem) => {
                    Ok(WidgetContextMode::ByRef)
                }
                _ => Err(syn::Error::new_spanned(
                    &pat_type.ty,
                    "a `#[widget]` function takes at most one `WidgetContext` parameter",
                )),
            }
        }
        _ => Err(syn::Error::new_spanned(
            &sig.inputs,
            "a `#[widget]` function takes at most one `WidgetContext` parameter",
        )),
    }
}

/// The `waterui_meta_widget_*` record's JSON payload, assembled at
/// expansion time — a literal the declaration macro parks in `.wmeta`.
#[cfg(feature = "widgets")]
fn widget_payload(fn_name: &str, params: &WidgetParams) -> String {
    let kind = if params.kind.is_empty() {
        fn_name
    } else {
        params.kind.as_str()
    };
    serde_json::json!({
        "fn": fn_name,
        "kind": kind,
        "displayName": params.display_name,
        "description": params.description,
        "families": params.families,
    })
    .to_string()
}

#[cfg(all(test, feature = "widgets"))]
mod widget_tests {
    use super::*;
    use syn::parse::Parser as _;

    #[test]
    fn widget_payload_preserves_declaration_strings() {
        let title = "quote\"\\\n\u{1}日本語";
        let params = WidgetParams {
            display_name: title.to_owned(),
            description: "line\nnext".to_owned(),
            kind: "fixture".to_owned(),
            families: vec!["systemSmall".to_owned()],
        };
        let payload: serde_json::Value =
            serde_json::from_str(&widget_payload("snapshot", &params)).unwrap();
        assert_eq!(payload["fn"], "snapshot");
        assert_eq!(payload["displayName"], title);
        assert_eq!(payload["description"], params.description);
        assert_eq!(payload["families"][0], "systemSmall");
    }

    #[test]
    fn widget_declaration_requires_a_family() {
        let args = Punctuated::<WidgetArg, Token![,]>::parse_terminated
            .parse2(quote!(
                name = "Example",
                description = "A widget",
                families = []
            ))
            .unwrap();
        let error = widget_params(&args).err().expect("empty families fail");

        assert!(error.to_string().contains("at least one supported family"));
    }

    #[test]
    fn widget_intent_validator_accepts_a_concrete_type() {
        let input: syn::DeriveInput = syn::parse_quote!(
            struct Record {
                value: String,
            }
        );
        let ident = validate_widget_intent_type(&input).expect("a concrete type accepts");
        assert_eq!(ident, "Record");
    }

    #[test]
    fn widget_intent_validator_rejects_a_generic_type() {
        let input: syn::DeriveInput = syn::parse_quote!(
            struct Record<T> {
                value: T,
            }
        );
        let error = validate_widget_intent_type(&input).err().unwrap();
        assert!(error.to_string().contains("concrete"));
    }

    #[test]
    fn widget_intent_params_rejects_invalid_arguments() {
        let parse = |source: &str| {
            Punctuated::<WidgetArg, Token![,]>::parse_terminated
                .parse_str(source)
                .expect("the argument list parses")
        };
        // `name` is required.
        assert!(
            widget_intent_params(&parse("kind = \"mark-done\""))
                .err()
                .expect("a missing name fails")
                .to_string()
                .contains("name")
        );
        // Duplicate arguments are rejected.
        assert!(
            widget_intent_params(&parse("name = \"a\", name = \"b\""))
                .err()
                .expect("a duplicate name fails")
                .to_string()
                .contains("duplicate")
        );
        // An explicit `kind` must not be empty.
        assert!(
            widget_intent_params(&parse("name = \"a\", kind = \"\""))
                .err()
                .expect("an empty kind fails")
                .to_string()
                .contains("kind")
        );
        // Unknown arguments are rejected.
        assert!(
            widget_intent_params(&parse("name = \"a\", bogus = \"x\""))
                .err()
                .expect("an unknown argument fails")
                .to_string()
                .contains("bogus")
        );
    }
}

/// The parsed `#[widget_intent]` arguments after defaults.
#[cfg(feature = "widgets")]
struct WidgetIntentParams {
    display_name: String,
    kind: Option<String>,
    description: Option<String>,
}

/// Validates `#[widget_intent]`'s `name = "…"`, `kind = "…"`,
/// `description = "…"` arguments.
#[cfg(feature = "widgets")]
fn widget_intent_params(
    args: &Punctuated<WidgetArg, Token![,]>,
) -> syn::Result<WidgetIntentParams> {
    let mut display_name: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut description: Option<String> = None;
    for arg in args {
        let target = match arg.name.to_string().as_str() {
            "name" => &mut display_name,
            "kind" => &mut kind,
            "description" => &mut description,
            other => {
                return Err(syn::Error::new_spanned(
                    &arg.name,
                    format!(
                        "unknown `#[widget_intent]` argument `{other}`; expected `name`, `kind` or `description`"
                    ),
                ));
            }
        };
        if target.is_some() {
            return Err(syn::Error::new_spanned(
                &arg.name,
                format!("duplicate `#[widget_intent]` argument `{}`", arg.name),
            ));
        }
        *target = Some(widget_arg_string(arg)?);
    }
    let Some(display_name) = display_name else {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[widget_intent]` requires `name = \"…\"` — the intent's display title",
        ));
    };
    if kind.as_deref().is_some_and(str::is_empty) {
        return Err(syn::Error::new(
            Span::call_site(),
            "`#[widget_intent]`'s `kind` must not be empty — it is the intent's identifier",
        ));
    }
    Ok(WidgetIntentParams {
        display_name,
        kind,
        description,
    })
}

/// Validates `#[widget_intent]`'s type declaration — a concrete struct or
/// enum — returning its ident for the default kind.
#[cfg(feature = "widgets")]
fn validate_widget_intent_type(input: &syn::DeriveInput) -> syn::Result<&syn::Ident> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "`#[widget_intent]` requires a concrete action type — a generic type cannot declare one intent kind",
        ));
    }
    Ok(&input.ident)
}

/// Declares a `WidgetKit` intent: `#[waterui::widget_intent(name = "…")]`
/// on the action's type declaration. `impl WidgetIntent for T` — the
/// action's `perform` — is written separately.
///
/// The attribute derives `Serialize`/`Deserialize` for the type through
/// `waterui::widgets::serde`, so the application needs no `serde`
/// dependency of its own; keeps the type intact; implements
/// `DeclaredWidgetIntent` for it (a hidden trait carrying the declared
/// kind); and emits the `waterui_intent_<crate>_<hash>` C export plus the
/// `waterui_meta_intent_<crate>_<hash>` record `water` reads while it
/// generates the extension's `AppIntent` conformers — `hash` is the
/// declared kind's `SHA-256` hex digest, so the generated `Swift` type
/// identity is stable across record ordering.
///
/// `kind` defaults to the type's ident; an explicit `kind` disambiguates
/// actions whose type names would collide across modules.
///
/// # Panics
///
/// The declaration's `serde_json` record always serializes.
#[cfg(feature = "widgets")]
#[proc_macro_attribute]
pub fn widget_intent(args: TokenStream, input: TokenStream) -> TokenStream {
    let input_type = parse_macro_input!(input as syn::DeriveInput);
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(error) => return error.into_compile_error().into(),
    };
    let args = parse_macro_input!(args with Punctuated::<WidgetArg, Token![,]>::parse_terminated);

    let type_ident = match validate_widget_intent_type(&input_type) {
        Ok(ident) => ident.clone(),
        Err(error) => return error.to_compile_error().into(),
    };
    let params = match widget_intent_params(&args) {
        Ok(params) => params,
        Err(error) => return error.to_compile_error().into(),
    };
    let kind = params.kind.unwrap_or_else(|| type_ident.to_string());
    let description = params
        .description
        .unwrap_or_else(|| params.display_name.clone());

    // The declared kind's stable hash — the `Swift` type name and the C
    // export/record suffix all derive from it, not from record ordering.
    let hash = {
        use std::fmt::Write as _;
        sha2::Sha256::digest(kind.as_bytes()).iter().fold(
            String::with_capacity(64),
            |mut acc, byte| {
                write!(acc, "{byte:02x}").expect("writing to a String never fails");
                acc
            },
        )
    };
    let record = serde_json::json!({
        "kind": kind,
        "displayName": params.display_name,
        "description": description,
        "swiftType": format!("WUIIntent_{hash}"),
    })
    .to_string();
    let record_lit = syn::LitStr::new(&record, Span::call_site());
    let hash_lit = syn::LitStr::new(&hash, Span::call_site());
    let kind_lit = syn::LitStr::new(&kind, Span::call_site());
    // `serde(crate = "...")` takes a string literal — the same resolved
    // `waterui` path, without the token stream's spacing.
    let serde_crate = syn::LitStr::new(
        &format!("{}::widgets::serde", waterui.to_string().replace(' ', "")),
        Span::call_site(),
    );

    let expanded = quote! {
        #[derive(#waterui::widgets::serde::Serialize, #waterui::widgets::serde::Deserialize)]
        #[serde(crate = #serde_crate)]
        #input_type

        #waterui::widgets::__export_intent!(
            #hash_lit,
            #record_lit,
            [__wui_intent_ptr, __wui_intent_len] => {
                let __wui_action: #type_ident = #waterui::widgets::serde_json::from_slice(
                    unsafe {
                        ::core::slice::from_raw_parts(
                            __wui_intent_ptr,
                            __wui_intent_len,
                        )
                    },
                )
                .expect("a widget intent's payload always decodes");
                <#type_ident as #waterui::widgets::WidgetIntent>::perform(__wui_action);
            }
        );

        impl #waterui::widgets::DeclaredWidgetIntent for #type_ident {
            const KIND: &'static str = #kind_lit;
        }
    };
    TokenStream::from(expanded)
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
            "`#[waterui::test(...)]` accepts an optional view function path followed by `theme = <style>`, `viewport = (width, height)`, and `offscreen`",
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
        if args.offscreen && args.theme.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "`offscreen` mounts the rendered runtime, which needs a style — pass `theme = <style>`",
            ));
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
/// A no-arg view function path mounts the semantic runtime before the test
/// body runs; the test function receives `&mut SemanticApp`. `theme = <style>`
/// carries a [`hydrolysis::Style`](https://docs.rs/hydrolysis) value (for
/// example `hydrolysis_m3::Material3::defaults()`), `viewport = (width,
/// height)` sizes the window, and the bare `offscreen` flag mounts the
/// rendered offscreen runtime instead — the test then takes `&mut
/// OffscreenApp`. `offscreen` requires `theme =`: the rendered runtime is
/// styled by construction.
///
/// ```rust
/// # use waterui::prelude::*;
/// fn login_view() -> impl View {
///     // ...
/// #   button("Login").action(|| {})
/// }
///
/// #[waterui::test(login_view, viewport = (360, 320))]
/// fn login_flow(app: &mut waterui_testing::SemanticApp) {
///     app.query().role(waterui_testing::Role::BUTTON).label("Login").tap();
/// }
/// # fn main() {}
/// ```
///
/// The styled form mounts pixels through the same harness:
///
/// ```ignore
/// # use waterui::prelude::*;
/// # fn login_view() -> impl View { button("Login").action(|| {}) }
/// #[waterui::test(login_view, theme = hydrolysis_m3::Material3::defaults(), offscreen)]
/// fn login_pixels(app: &mut waterui_testing::OffscreenApp) {
///     app.capture_snapshot();
/// }
/// # fn main() {}
/// ```
///
/// # Manual-mount form
///
/// Without a view path the test function receives the configured
/// [`UiBuilder`](https://docs.rs/waterui-testing) by value and mounts in its
/// own body — the form for tests that own `Binding`s the view closes over.
/// A styled manual mount names the style in the parameter type.
///
/// ```rust
/// # use waterui::prelude::*;
/// #[waterui::test]
/// fn stepper_updates_binding(ui: waterui_testing::UiBuilder) {
///     let value = Binding::i32(2);
///     let value_for_view = value.clone();
///     let mut app = ui.mount(move || stepper("Limited", &value_for_view));
///     app.query().label("Limited").increment();
///     assert_eq!(value.snapshot(), 3);
/// }
/// # fn main() {}
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

    let waterui_path = match waterui_crate_path() {
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

    // `catalog!` must expand in the test's own crate — that is where `i18n/`
    // lives — so the configured environment comes from the expansion here
    // rather than from inside `waterui-testing`.
    let mut builder = quote! {
        #testing_path::ui()
            .environment(
                #waterui_path::configure_environment!(#waterui_path::env::Environment::new())
            )
    };
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
            "`#[waterui::bench(...)]` accepts an optional view function path followed by `theme = <style>`, `viewport = (width, height)`, and the budgets `max_p95_us`, `max_mean_us`, `max_rebuild_ratio`, `max_scene_layers`, `max_gpu_surface_layers`, `max_clip_layers`",
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
        if args.theme.is_none() {
            return Err(syn::Error::new(
                Span::call_site(),
                "`#[waterui::bench(...)]` measures rendered frames, which need a style — pass `theme = <style>`",
            ));
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
/// A no-arg view function path mounts the rendered offscreen runtime before
/// the bench body runs; the bench function receives `&mut PerfApp` and records
/// scenarios with `measure`. Benches measure rendered frames, so a
/// `theme = <style>` argument — a [`hydrolysis::Style`](https://docs.rs/hydrolysis)
/// value such as `hydrolysis_m3::Material3::defaults()` — is required.
/// `viewport = (width, height)` sizes the window.
///
/// ```ignore
/// # use waterui::prelude::*;
/// fn dashboard() -> impl View {
///     // ...
/// #   text("Dashboard")
/// }
///
/// #[waterui::bench(dashboard, theme = hydrolysis_m3::Material3::defaults(), viewport = (390, 844), max_p95_us = 8_000)]
/// fn dashboard_redraw(perf: &mut waterui_testing::PerfApp) {
///     perf.measure("steady-redraw", |run| run.redraw());
/// }
/// # fn main() {}
/// ```
///
/// # Manual form
///
/// Without a view path the bench function receives the configured `UiBuilder`
/// — typed `UiBuilder<Styled<S>>` by the `theme =` argument — by value, drives
/// `perf_with` itself, and returns the `PerfReport` — the form for benches
/// that own `Binding`s the view closes over.
///
/// ```ignore
/// # use waterui::prelude::*;
/// # fn counter(value: &Binding<i32>) -> impl View {
/// #     let value = value.clone();
/// #     button("Increment").action(move || *value.get_mut() += 1)
/// # }
/// #[waterui::bench(theme = hydrolysis_m3::Material3::defaults(), max_rebuild_ratio = 0.2)]
/// fn counter_updates(
///     ui: waterui_testing::UiBuilder<waterui_testing::Styled<hydrolysis_m3::Material3>>,
/// ) -> waterui_testing::PerfReport {
///     let value = Binding::i32(0);
///     let value_for_view = value.clone();
///     ui.perf_with(move || counter(&value_for_view), |perf| {
///         perf.measure("increment", |run| {
///             run.app().query().label("Increment").tap();
///             let _ = run;
///         });
///     })
/// }
/// # fn main() {}
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

    let waterui_path = match waterui_crate_path() {
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

    let mut builder = quote! {
        #testing_path::ui()
            .environment(
                #waterui_path::configure_environment!(#waterui_path::env::Environment::new())
            )
    };
    if let Some((width, height)) = &bench_args.viewport {
        builder = quote! { #builder.viewport(#width, #height) };
    }
    if let Some(theme) = &bench_args.theme {
        builder = quote! { #builder.theme(#theme) };
    }
    builder = quote! { #builder.perf_config(__waterui_bench_config) };

    let budget_setters = bench_args.budgets.iter().filter_map(|(name, value)| {
        let setter = syn::Ident::new(&format!("with_{name}"), Span::call_site());
        value
            .as_ref()
            .map(|expr| quote! { .#setter(::core::option::Option::Some(#expr)) })
    });
    let budgets = quote! {
        #testing_path::bench::BenchBudgets::default() #(#budget_setters)*
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

/// Derives the TypeScript projection of a struct or enum.
///
/// The projection is an associated `const SCHEMA` built from the field types'
/// own schemas, so aliases, generic arguments and `cfg`s are resolved by the
/// compiler rather than by reading how a type is spelled. A struct becomes an
/// object type and needs named fields; an enum whose variants are all unit
/// variants becomes a union of string literals, and an enum carrying data
/// becomes a tagged object.
///
/// Use it for the types nested inside a props struct; the root props struct
/// derives [`macro@TsProps`], which also emits the artifact metadata.
///
/// When the TypeScript runtime is reachable — the expansion resolves through
/// `waterui` or `waterui-ts` — the derive also emits the `IntoJs` and `FromJs`
/// conversions that carry a value of the type across the seam, because a
/// nested data type travels both ways: out as a props field, back in as a
/// callback argument. A type that carries a value which only travels
/// outwards — a callback, which the bridge registers rather than reads —
/// declares `#[ts(one_way)]` and gets `IntoJs` alone. A crate that depends on
/// `waterui-ts-schema` alone gets the schema and nothing else.
///
/// A field whose TypeScript property is spelled differently from its Rust name
/// says so with `#[ts(rename = "onTap")]`. The rename travels through the
/// schema and both conversions at once, so a JSX attribute in the case
/// TypeScript writes — `onTap`, `minLength`, `a11yLabel` — still comes from
/// one declaration in one place.
#[proc_macro_derive(TsType, attributes(ts))]
pub fn derive_ts_type(input: TokenStream) -> TokenStream {
    ts::derive_ts_type(input)
}

/// Derives the props contract of a TypeScript view module's root props struct.
///
/// Emits everything [`macro@TsType`] does, plus the contract's encoded form,
/// its hash, and a `#[cfg(debug_assertions)] #[used] static` named
/// `waterui_meta_tsprops_<Type>` parked in the `.wmeta` (`__DATA,__wmeta` on
/// Apple targets) section and holding a self-describing record — the name,
/// NUL, the encoding, NUL. That record is how the contract reaches the
/// `water` CLI: `#[used]` keeps it in the object file, the rlib, and the
/// linked image, and a linked image need not keep a symbol table to name a
/// static — a linked PE keeps none — so the CLI walks the section's records
/// out of a dev-profile host build instead of parsing source. The encoding
/// happens entirely during const evaluation, and the debug gate keeps the
/// bytes out of shipped binaries.
///
/// The record name carries the type name alone, so one binary may hold only
/// one props type of a given name.
///
/// When the TypeScript runtime is reachable the derive also emits `IntoJs`,
/// and only `IntoJs`: props are handed to a module, and nothing reads a props
/// struct back out of JavaScript — requiring it to be readable would rule out
/// the callbacks and views props exist to carry.
#[proc_macro_derive(TsProps, attributes(ts))]
pub fn derive_ts_props(input: TokenStream) -> TokenStream {
    ts::derive_ts_props(input)
}

/// Mounts a TypeScript view module as a `WaterUI` view.
///
/// ```text
/// use waterui::{View, tsx};
///
/// #[derive(waterui::TsProps)]
/// struct PromoProps {
///     headline: String,
///     unread: waterui::Binding<u32>,
/// }
///
/// fn promo(unread: waterui::Binding<u32>) -> impl View {
///     tsx!("./promo.tsx", PromoProps { headline: "Welcome".into(), unread })
/// }
/// ```
///
/// The example is not compiled as a doctest: a doctest has no source file
/// inside the crate, and the path below is resolved relative to the file the
/// macro is written in.
///
/// # The module path and the module id
///
/// The path is written relative to the Rust file that mounts the module, so a
/// `.tsx` beside its view code is `"./promo.tsx"`. The **module id** — the key
/// the bundle publishes that module under, and what a mount failure names — is
/// that file's path relative to the crate's `CARGO_MANIFEST_DIR`, with forward
/// slashes and the extension kept: `src/views/promo.tsx`. The crate directory
/// is the root the CLI's bundler resolves modules against, so a module outside
/// it, an absolute path, or a file that does not exist is a compile error
/// naming the path.
///
/// "The file that mounts the module" is the file the path literal is written
/// in, as it is for `include_str!`: a `macro_rules!` that expands to a `tsx!`
/// with a literal of its own resolves that literal beside the file defining
/// the `macro_rules!`, not beside its caller. A wrapper macro that takes the
/// path from its caller passes the caller's literal through and resolves it
/// beside the caller.
///
/// # Props
///
/// The props argument is a struct literal, which is what names the props type
/// at the mount site: `tsx!("./promo.tsx", PromoProps { … })` expands to
/// `Mount::new::<PromoProps>("src/views/promo.tsx", PromoProps { … })`. A
/// value prepared elsewhere goes in with struct update syntax,
/// `PromoProps { ..props }`. A module that takes no props is written
/// `tsx!("./promo.tsx")` and is typed against `waterui::ts::NoProps`, the empty
/// contract — still a contract the bundle declares a hash for, rather than a
/// special case that skips the check.
///
/// The props type must derive [`macro@TsProps`]; a type that does not is a
/// compile error naming it.
///
/// # What reaches the CLI
///
/// The expansion emits a `#[cfg(debug_assertions)] #[used] static` named
/// `waterui_meta_tsx_<id>` carrying the module id, the props struct's name and
/// its contract hash, so the `water` CLI learns from the compiled artifact
/// which modules the binary mounts and which schema each one must be typed
/// against. The expansion is a pure read: the macro stats the file to check it
/// exists and never opens it. Whether the module has a default export is the
/// bundler's question, because the bundler has already resolved the module
/// graph; a module the bundle does not carry is a typed error at mount naming
/// the id and the ids the bundle does have.
#[proc_macro]
pub fn tsx(input: TokenStream) -> TokenStream {
    tsx::tsx(input)
}
