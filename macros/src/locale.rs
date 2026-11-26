//! Proc macros for `WaterUI` i18n.
//!
//! This crate provides the `text!` macro for internationalized text.
//! The macro loads translation files from the `i18n/` folder at compile time.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::result::Result as StdResult;

use icu_locale_core::{LanguageIdentifier, Locale as IcuLocale};
use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Result, Token};
use waterui_locale_parser::{
    DualPluralForms, PluralCategory, PluralForms, TranslationFile, TranslationValue,
};

const PLACEHOLDER_RULE: &str = "placeholders are bare identifiers (`{count}`), optionally with a format spec (`{blur:.1}`) or a plural marker (`{#count}`), and an expression is bound with `name = expr`";

fn waterui_crate_path() -> std::result::Result<TokenStream2, TokenStream2> {
    if current_package_name().as_deref() == Some("waterui-internal") {
        return Ok(own_crate_path("waterui-internal"));
    }

    match crate_name("waterui") {
        Ok(FoundCrate::Itself) => Ok(own_crate_path(
            current_package_name().as_deref().unwrap_or("waterui"),
        )),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(_) => Err(quote! {
            compile_error!("`text!` requires the `waterui` crate as a dependency (it may be renamed; Cargo.toml must include it).");
        }),
    }
}

/// How the crate being compiled refers to itself.
///
/// Normally that is `crate`, but a doctest is compiled as its own crate that
/// merely *depends* on the one it documents, so `crate` there names the example
/// and every path the macro expands to goes missing. The documented crate is an
/// ordinary dependency of that example, so it is named outright instead.
///
/// `UNSTABLE_RUSTDOC_TEST_PATH` is set only by rustdoc when it compiles a
/// doctest, which is what makes it the tell; the surrounding package is
/// otherwise indistinguishable from a normal build.
fn own_crate_path(package: &str) -> TokenStream2 {
    if std::env::var_os("UNSTABLE_RUSTDOC_TEST_PATH").is_none() {
        return quote!(crate);
    }
    let ident = Ident::new(&package.replace('-', "_"), Span::call_site());
    quote!(::#ident)
}

fn current_package_name() -> Option<String> {
    std::env::var("CARGO_PKG_NAME").ok()
}

/// All translations loaded from i18n folder.
#[derive(Debug, Default)]
struct TranslationBundle {
    /// Map of canonical locale tag -> (parsed identifier, parsed translation file).
    locales: BTreeMap<String, (LanguageIdentifier, TranslationFile)>,
    /// Translation files loaded from disk (used for build invalidation tracking).
    tracked_files: Vec<PathBuf>,
}

impl TranslationBundle {
    fn load_from_manifest_dir() -> std::result::Result<Self, String> {
        // CLI-generated backend crates live outside the application crate root
        // (managed_backends/, the CLI's build cache), so their
        // `CARGO_MANIFEST_DIR` cannot see the app's `i18n/`. Their build
        // scripts pass the directory through `WATERUI_I18N_DIR` instead, which
        // makes `catalog!`/`text!` embed the app's translations even though the
        // macro expands in the generated crate.
        let i18n_path = std::env::var("WATERUI_I18N_DIR")
            .ok()
            .filter(|dir| !dir.is_empty())
            .map_or_else(
                || {
                    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
                    PathBuf::from(&manifest_dir).join("i18n")
                },
                PathBuf::from,
            );

        Self::load_from_dir(&i18n_path)
    }

    fn load_from_dir(i18n_path: &Path) -> StdResult<Self, String> {
        let mut bundle = Self::default();

        if !i18n_path.exists() {
            return Ok(bundle);
        }

        // Load all .toml files in i18n/
        let entries = fs::read_dir(i18n_path).map_err(|err| {
            format!(
                "Failed to read i18n directory '{}': {err}",
                i18n_path.display()
            )
        })?;
        let mut locale_files: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|e| e == "toml"))
            .collect();
        locale_files.sort();

        for path in locale_files {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| format!("Invalid locale filename '{}'", path.display()))?;
            let parsed_locale = stem.parse::<IcuLocale>().map_err(|err| {
                format!(
                    "Invalid BCP-47 locale tag '{stem}' in '{}': {err}",
                    path.display()
                )
            })?;
            let content = std::fs::read_to_string(&path)
                .map_err(|err| format!("Failed to read '{}': {err}", path.display()))?;
            let translations = TranslationFile::parse_named(&content, &path.display().to_string())
                .map_err(|err| err.to_string())?;

            bundle
                .locales
                .insert(stem.to_string(), (parsed_locale.id, translations));
            bundle.tracked_files.push(path);
        }

        Ok(bundle)
    }
}

/// A parsed placeholder from the format string.
#[derive(Debug, Clone)]
enum Placeholder {
    /// Regular placeholder: {name}
    Regular(String),
    /// Plural source placeholder: {#name}
    Plural(String),
}

/// Parsed text! macro input.
struct TextInput {
    /// The format string literal (translation key).
    format_string: LitStr,
    /// Optional context (after @).
    context: Option<String>,
    /// Explicit bindings (name = expr).
    bindings: Vec<(Ident, Expr)>,
}

impl Parse for TextInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        // Parse the format string
        let format_string: LitStr = input.parse()?;

        // Check for @ context
        let context = if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            // Context can be a string literal or an identifier
            if input.peek(LitStr) {
                let ctx: LitStr = input.parse()?;
                Some(ctx.value())
            } else {
                let ctx: Ident = input.parse()?;
                Some(ctx.to_string())
            }
        } else {
            None
        };

        // Parse optional explicit bindings: , name = expr, ...
        let mut bindings = Vec::new();
        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;

            // Parse name = expr
            let name: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            let expr: Expr = input.parse()?;
            bindings.push((name, expr));
        }

        Ok(Self {
            format_string,
            context,
            bindings,
        })
    }
}

/// Parse placeholders from a format string.
///
/// Errors on the first placeholder that does not name a slot (`{}`, `{0}`, or
/// any other non-identifier base): placeholders are slot keys, so one the
/// macro cannot name must fail at compile time rather than survive expansion
/// and render its braces literally.
fn parse_placeholders(format_string: &str) -> std::result::Result<Vec<Placeholder>, String> {
    let mut placeholders = Vec::new();
    let mut chars = format_string.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            // Check for escaped brace
            if chars.peek() == Some(&'{') {
                chars.next();
                continue;
            }

            let is_plural = if chars.peek() == Some(&'#') {
                chars.next();
                true
            } else {
                false
            };

            let mut raw = String::from("{");
            if is_plural {
                raw.push('#');
            }
            let mut content = String::new();
            while let Some(&c) = chars.peek() {
                raw.push(c);
                chars.next();
                if c == '}' {
                    break;
                }
                if c == ':' {
                    while let Some(&spec_c) = chars.peek() {
                        raw.push(spec_c);
                        chars.next();
                        if spec_c == '}' {
                            break;
                        }
                    }
                    break;
                }
                content.push(c);
            }

            let content = content.trim();
            let content = content.strip_suffix('=').unwrap_or(content);
            let base = content.split(['.', '[']).next().unwrap_or("").trim();

            if !is_valid_ident(base) {
                return Err(format!(
                    "invalid placeholder `{raw}` in `text!` format string: {PLACEHOLDER_RULE}"
                ));
            }

            let name = base.to_string();
            if is_plural {
                placeholders.push(Placeholder::Plural(name));
            } else {
                placeholders.push(Placeholder::Regular(name));
            }
        }
    }

    Ok(placeholders)
}

/// A slot key is a plain identifier: `syn` also accepts raw identifiers
/// (`r#fn`), which `Ident::new` later panics on, and a translation catalog
/// key is never spelled with `r#`.
fn is_valid_ident(name: &str) -> bool {
    !name.starts_with("r#") && syn::parse_str::<Ident>(name).is_ok()
}

fn build_zip_expr_and_pattern(
    waterui: &TokenStream2,
    idents: &[Ident],
) -> (TokenStream2, TokenStream2) {
    let mut iter = idents.iter();
    let first = iter
        .next()
        .expect("zip expressions require at least one identifier");
    let mut expr = quote! { #first.clone() };
    let mut pattern = quote! { #first };

    for ident in iter {
        expr = quote! { #waterui::reactive::zip::zip(#expr, #ident.clone()) };
        pattern = quote! { (#pattern, #ident) };
    }

    (expr, pattern)
}

fn build_signal_map(waterui: &TokenStream2, idents: &[Ident], body: &TokenStream2) -> TokenStream2 {
    match idents.len() {
        0 => quote! { #waterui::reactive::constant(#body) },
        1 => {
            let ident = &idents[0];
            quote! {
                {
                    let __signal = #ident.clone();
                    #waterui::reactive::SignalExt::map(&__signal, move |#ident| {
                        #body
                    })
                }
            }
        }
        _ => {
            let (zip_expr, pattern) = build_zip_expr_and_pattern(waterui, idents);
            quote! {
                {
                    let __zipped = #zip_expr;
                    #waterui::reactive::SignalExt::map(&__zipped, move |#pattern| {
                        #body
                    })
                }
            }
        }
    }
}

fn build_format_signal(
    waterui: &TokenStream2,
    format_str: &LitStr,
    idents: &[Ident],
) -> TokenStream2 {
    let locale_ident = unique_ident("__waterui_format_locale", idents);
    let bindings = localized_argument_bindings(waterui, idents, &locale_ident);
    let body = quote! {
        #bindings
        #waterui::reactive::__alloc::format!(#format_str)
    };
    let signal = build_signal_map(waterui, idents, &body);
    quote! {{
        let #locale_ident = locale.clone();
        #signal
    }}
}

fn localized_argument_bindings(
    waterui: &TokenStream2,
    idents: &[Ident],
    locale_ident: &Ident,
) -> TokenStream2 {
    quote! {
        #(let #idents = {
            use #waterui::locale::{
                __LocalizedArgFallback as _,
                __LocalizedArgNumber as _,
                __LocalizedArgToPrimitive as _,
                __LocalizedArgToPrimitiveDisplay as _,
            };
            (&&&&#waterui::locale::__LocalizedArgProbe(&#idents)).__waterui_localized_arg(&#locale_ident)
        };)*
    }
}

fn unique_ident(base: &str, existing: &[Ident]) -> Ident {
    let mut name = base.to_string();
    while existing.iter().any(|ident| *ident == name) {
        name.push('_');
    }
    Ident::new(&name, Span::call_site())
}

/// Generate the translation key from format string and optional context.
fn make_key(format_string: &str, context: Option<&str>) -> String {
    context.map_or_else(
        || format_string.to_string(),
        |ctx| format!("{format_string}#{ctx}"),
    )
}

fn collect_plural_names(placeholders: &[Placeholder]) -> Vec<&str> {
    placeholders
        .iter()
        .filter_map(|placeholder| match placeholder {
            Placeholder::Plural(name) => Some(name.as_str()),
            Placeholder::Regular(_) => None,
        })
        .collect()
}

fn push_syn_error(errors: &mut Option<syn::Error>, err: syn::Error) {
    if let Some(existing) = errors.as_mut() {
        existing.combine(err);
    } else {
        *errors = Some(err);
    }
}

fn build_binding_map<'a>(
    format_string: &str,
    placeholders: &[Placeholder],
    bindings: &'a [(Ident, Expr)],
) -> Result<HashMap<String, &'a Expr>> {
    let placeholder_names: HashSet<&str> = placeholders
        .iter()
        .map(|placeholder| match placeholder {
            Placeholder::Regular(name) | Placeholder::Plural(name) => name.as_str(),
        })
        .collect();

    let mut seen = HashSet::with_capacity(bindings.len());
    let mut map = HashMap::with_capacity(bindings.len());
    let mut errors = None;

    for (name, expr) in bindings {
        let key = name.to_string();
        if !seen.insert(key.clone()) {
            push_syn_error(
                &mut errors,
                syn::Error::new(name.span(), format!("duplicate binding `{key}` in `text!`")),
            );
            continue;
        }
        if !placeholder_names.contains(key.as_str()) {
            push_syn_error(
                &mut errors,
                syn::Error::new(
                    name.span(),
                    format!("`{key}` is not a placeholder in {format_string:?}"),
                ),
            );
            continue;
        }
        map.insert(key, expr);
    }

    if let Some(err) = errors {
        return Err(err);
    }
    Ok(map)
}

fn build_captures_and_idents(
    waterui: &TokenStream2,
    placeholders: &[Placeholder],
    binding_map: &HashMap<String, &Expr>,
) -> (Vec<TokenStream2>, Vec<Ident>) {
    let mut captures = Vec::new();
    let mut all_names = Vec::new();

    if !placeholders.is_empty() {
        // Bring `to_owned()` into scope (relies on auto-ref for literals like `0`).
        captures.push(quote! {
            use #waterui::reactive::__alloc::borrow::ToOwned as _;
        });
    }

    for placeholder in placeholders {
        let name = match placeholder {
            Placeholder::Regular(name) | Placeholder::Plural(name) => name,
        };

        if all_names.contains(name) {
            continue;
        }

        all_names.push(name.clone());
        let name_ident = Ident::new(name, proc_macro2::Span::call_site());

        if let Some(expr) = binding_map.get(name) {
            captures.push(quote! {
                let #name_ident = (#expr).to_owned();
            });
        } else {
            captures.push(quote! {
                let #name_ident = (#name_ident).to_owned();
            });
        }
    }

    let all_idents = all_names
        .iter()
        .map(|name| Ident::new(name, Span::call_site()))
        .collect();
    (captures, all_idents)
}

fn build_locale_arms(
    bundle: &TranslationBundle,
    translation_key: &str,
    waterui: &TokenStream2,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> Vec<TokenStream2> {
    let mut locale_arms = Vec::new();
    for (locale_id, translations) in bundle.locales.values() {
        if let Some(value) = translations.get(translation_key) {
            let arm = generate_translation_arm(waterui, locale_id, value, all_idents, plural_names);
            locale_arms.push(arm);
        }
    }
    locale_arms
}

fn translation_format_lit(text: &str) -> LitStr {
    let format_str = text.replace("{#", "{");
    LitStr::new(&format_str, Span::call_site())
}

fn build_text_config_expr(
    waterui: &TokenStream2,
    format_lit: &LitStr,
    all_idents: &[Ident],
) -> TokenStream2 {
    if all_idents.is_empty() {
        quote! { #waterui::text::TextConfig::new(#format_lit) }
    } else {
        let content = build_format_signal(waterui, format_lit, all_idents);
        quote! { #waterui::text::TextConfig::new(#content) }
    }
}

/// Macro for creating localized text.
///
/// The `text!` macro loads translations from `i18n/` folder at compile time
/// and generates a `Text` view that renders based on the current locale.
///
/// # Translation Files
///
/// Create TOML files in your crate's `i18n/` folder:
///
/// ```toml
/// # i18n/en.toml
/// "Hello, World!" = "Hello, World!"
/// "I have {#count} apple" = { one = "I have {count} apple", other = "I have {count} apples" }
///
/// # i18n/zh.toml
/// "Hello, World!" = "你好，世界！"
/// "I have {#count} apple" = { other = "我有{count}个苹果" }
/// ```
///
/// # Syntax
///
/// ```rust
/// # use waterui::prelude::*;
/// # fn plain() -> impl View {
/// // Simple text
/// text!("Hello, World!")
/// # }
///
/// # fn named(name: Str) -> impl View {
/// // Named placeholders (captured from scope)
/// text!("Hello, {name}")
/// # }
///
/// # fn plural(count: i32) -> impl View {
/// // Plural source (for CLDR plural rules)
/// text!("I have {#count} apple")
/// # }
///
/// # fn homograph() -> impl View {
/// // Context for homographs
/// text!("Right" @ "direction")
/// # }
///
/// # fn explicit(get_name: impl Fn() -> Str) -> impl View {
/// // Explicit binding
/// text!("Hello, {name}", name = get_name())
/// # }
/// ```
pub fn text(input: &TokenStream) -> TokenStream {
    let input = match syn::parse::<TextInput>(input.clone()) {
        Ok(input) => input,
        Err(err) => return err.to_compile_error().into(),
    };
    let expanded = expand_text_macro(&input);
    TokenStream::from(expanded)
}

fn compile_error(message: &str) -> TokenStream2 {
    let message = LitStr::new(message, Span::call_site());
    quote! { compile_error!(#message) }
}

fn syn_errors_expr(err: syn::Error) -> TokenStream2 {
    let errors = err.into_iter().map(|error| {
        let item = error.to_compile_error();
        quote! { #item; }
    });
    quote! { { #(#errors)* } }
}

fn expand_text_macro(input: &TextInput) -> TokenStream2 {
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(err) => return err,
    };

    let key = input.format_string.value();
    let translation_key = make_key(&key, input.context.as_deref());
    let placeholders = match parse_placeholders(&key) {
        Ok(placeholders) => placeholders,
        Err(err) => return compile_error(&err),
    };
    let binding_map = match build_binding_map(&key, &placeholders, &input.bindings) {
        Ok(map) => map,
        Err(err) => return syn_errors_expr(err),
    };

    // Load translations at compile time
    let bundle = match TranslationBundle::load_from_manifest_dir() {
        Ok(bundle) => bundle,
        Err(err) => return compile_error(&err),
    };

    let plural_names = collect_plural_names(&placeholders);
    let (captures, all_idents) = build_captures_and_idents(&waterui, &placeholders, &binding_map);
    let locale_arms = build_locale_arms(
        &bundle,
        &translation_key,
        &waterui,
        &all_idents,
        &plural_names,
    );

    let tracked_file_list: Vec<LitStr> = bundle
        .tracked_files
        .iter()
        .filter_map(|path| path.to_str())
        .map(|path| LitStr::new(path, Span::call_site()))
        .collect();

    // Default fallback - use the key itself
    let default_format = key.replace("{#", "{");
    let default_format_lit = LitStr::new(&default_format, Span::call_site());
    let default_body = build_text_config_expr(&waterui, &default_format_lit, &all_idents);

    // Generate the localized Text
    quote! {
        {
            // Track translation file changes to invalidate macro expansion.
            #(let _ = include_bytes!(#tracked_file_list);)*

            #waterui::text::Text::localized_with({
                #(#captures)*

                move |_env: &#waterui::Environment, locale: &#waterui::locale::Locale| {
                    let resolve = |locale_key: &#waterui::locale::Locale| -> Option<#waterui::text::TextConfig> {
                        #(#locale_arms)*
                        None
                    };

                    if let Some(text) =
                        #waterui::locale::locale::find_in_fallback_chain(locale, &resolve)
                    {
                        return text;
                    }

                    if let Some(text) = resolve(&#waterui::locale::locale::locales::EN) {
                        return text;
                    }

                    #default_body
                }
            })
        }
    }
}

fn generate_translation_arm(
    waterui: &TokenStream2,
    locale: &LanguageIdentifier,
    value: &TranslationValue,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> TokenStream2 {
    match value {
        TranslationValue::Simple(text) => {
            generate_simple_translation_arm(waterui, locale, text, all_idents)
        }
        TranslationValue::Plural(forms) => {
            generate_plural_translation_arm(waterui, locale, forms, all_idents, plural_names)
        }
        TranslationValue::DualPlural(forms) => {
            generate_dual_plural_translation_arm(waterui, locale, forms, all_idents, plural_names)
        }
        _ => TokenStream2::new(),
    }
}

fn generate_simple_translation_arm(
    waterui: &TokenStream2,
    locale: &LanguageIdentifier,
    text: &str,
    all_idents: &[Ident],
) -> TokenStream2 {
    let condition = locale_match_condition(locale);
    let format_lit = translation_format_lit(text);
    let text_config = build_text_config_expr(waterui, &format_lit, all_idents);
    quote! {
        if #condition {
            return Some(#text_config);
        }
    }
}

fn locale_match_condition(locale: &LanguageIdentifier) -> TokenStream2 {
    let language = LitStr::new(locale.language.as_str(), Span::call_site());
    let script = locale
        .script
        .as_ref()
        .map(|script| LitStr::new(script.as_str(), Span::call_site()));
    let region = locale
        .region
        .as_ref()
        .map(|region| LitStr::new(region.as_str(), Span::call_site()));
    let variants = locale
        .variants
        .iter()
        .map(|variant| LitStr::new(variant.as_str(), Span::call_site()))
        .collect::<Vec<_>>();
    let variant_count = variants.len();
    let variants_match = if variants.is_empty() {
        quote! { locale_key.id().variants.is_empty() }
    } else {
        quote! {
            locale_key
                .id()
                .variants
                .iter()
                .map(|variant| variant.as_str())
                .eq([#(#variants),*].into_iter())
        }
    };

    let script_match = script.map_or_else(
        || quote! { locale_key.id().script.is_none() },
        |script| {
            quote! {
                locale_key.id().script.as_ref().map(|value| value.as_str()) == Some(#script)
            }
        },
    );
    let region_match = region.map_or_else(
        || quote! { locale_key.id().region.is_none() },
        |region| {
            quote! {
                locale_key.id().region.as_ref().map(|value| value.as_str()) == Some(#region)
            }
        },
    );

    quote! {
        locale_key.id().language.as_str() == #language
            && #script_match
            && #region_match
            && locale_key.id().variants.len() == #variant_count
            && #variants_match
    }
}

fn push_plural_category_arm(
    category_arms: &mut Vec<TokenStream2>,
    waterui: &TokenStream2,
    category: &TokenStream2,
    text: Option<&str>,
) {
    if let Some(text) = text {
        let format_lit = translation_format_lit(text);
        category_arms.push(quote! {
            #category => {
                #waterui::reactive::__alloc::format!(#format_lit)
            }
        });
    }
}

fn generate_plural_translation_arm(
    waterui: &TokenStream2,
    locale: &LanguageIdentifier,
    forms: &PluralForms,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> TokenStream2 {
    let condition = locale_match_condition(locale);
    let plural_var = plural_names.first().copied().unwrap_or("count");
    let plural_ident = Ident::new(plural_var, Span::call_site());
    let mut category_arms = Vec::new();
    let zero = quote! { #waterui::locale::PluralCategory::Zero };
    let one = quote! { #waterui::locale::PluralCategory::One };
    let two = quote! { #waterui::locale::PluralCategory::Two };
    let few = quote! { #waterui::locale::PluralCategory::Few };
    let many = quote! { #waterui::locale::PluralCategory::Many };
    push_plural_category_arm(&mut category_arms, waterui, &zero, forms.zero.as_deref());
    push_plural_category_arm(&mut category_arms, waterui, &one, forms.one.as_deref());
    push_plural_category_arm(&mut category_arms, waterui, &two, forms.two.as_deref());
    push_plural_category_arm(&mut category_arms, waterui, &few, forms.few.as_deref());
    push_plural_category_arm(&mut category_arms, waterui, &many, forms.many.as_deref());

    let other_format_lit = translation_format_lit(&forms.other);
    let locale_ident = unique_ident("__waterui_locale_value", all_idents);
    let localized_bindings = localized_argument_bindings(waterui, all_idents, &locale_ident);
    let body = quote! {
        let category = #waterui::locale::select_plural(&#locale_ident, &#plural_ident);
        #localized_bindings
        match category {
            #(#category_arms)*
            _ => #waterui::reactive::__alloc::format!(#other_format_lit),
        }
    };
    let content = build_signal_map(waterui, all_idents, &body);

    quote! {
        if #condition {
            let #locale_ident = locale.clone();
            return Some(#waterui::text::TextConfig::new(#content));
        }
    }
}

fn plural_category_tokens(waterui: &TokenStream2, category: PluralCategory) -> TokenStream2 {
    match category {
        PluralCategory::Zero => quote! { #waterui::locale::PluralCategory::Zero },
        PluralCategory::One => quote! { #waterui::locale::PluralCategory::One },
        PluralCategory::Two => quote! { #waterui::locale::PluralCategory::Two },
        PluralCategory::Few => quote! { #waterui::locale::PluralCategory::Few },
        PluralCategory::Many => quote! { #waterui::locale::PluralCategory::Many },
        PluralCategory::Other | _ => quote! { #waterui::locale::PluralCategory::Other },
    }
}

fn generate_dual_plural_translation_arm(
    waterui: &TokenStream2,
    locale: &LanguageIdentifier,
    forms: &DualPluralForms,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> TokenStream2 {
    let condition = locale_match_condition(locale);
    let plural_var_1 = plural_names.first().copied().unwrap_or("count");
    let plural_var_2 = plural_names.get(1).copied().unwrap_or(plural_var_1);
    let plural_ident_1 = Ident::new(plural_var_1, Span::call_site());
    let plural_ident_2 = Ident::new(plural_var_2, Span::call_site());

    let mut match_arms = Vec::new();
    for ((cat1, cat2), text) in forms.iter_forms() {
        if cat1 != PluralCategory::Other && cat2 != PluralCategory::Other {
            let pat1 = plural_category_tokens(waterui, cat1);
            let pat2 = plural_category_tokens(waterui, cat2);
            let lit = translation_format_lit(text);
            match_arms.push(quote! {
                (#pat1, #pat2) => #waterui::reactive::__alloc::format!(#lit),
            });
        }
    }
    for ((cat1, cat2), text) in forms.iter_forms() {
        if cat1 != PluralCategory::Other && cat2 == PluralCategory::Other {
            let pat1 = plural_category_tokens(waterui, cat1);
            let lit = translation_format_lit(text);
            match_arms.push(quote! {
                (#pat1, _) => #waterui::reactive::__alloc::format!(#lit),
            });
        }
    }
    for ((cat1, cat2), text) in forms.iter_forms() {
        if cat1 == PluralCategory::Other && cat2 != PluralCategory::Other {
            let pat2 = plural_category_tokens(waterui, cat2);
            let lit = translation_format_lit(text);
            match_arms.push(quote! {
                (_, #pat2) => #waterui::reactive::__alloc::format!(#lit),
            });
        }
    }

    let other_other_lit = translation_format_lit(forms.other_other());
    let locale_ident = unique_ident("__waterui_locale_value", all_idents);
    let localized_bindings = localized_argument_bindings(waterui, all_idents, &locale_ident);

    let body = quote! {
        let category_1 = #waterui::locale::select_plural(&#locale_ident, &#plural_ident_1);
        let category_2 = #waterui::locale::select_plural(&#locale_ident, &#plural_ident_2);
        #localized_bindings
        match (category_1, category_2) {
            #(#match_arms)*
            _ => #waterui::reactive::__alloc::format!(#other_other_lit),
        }
    };
    let content = build_signal_map(waterui, all_idents, &body);

    quote! {
        if #condition {
            let #locale_ident = locale.clone();
            return Some(#waterui::text::TextConfig::new(#content));
        }
    }
}

/// The path the expanding crate uses to name `waterui-locale` items.
///
/// `TranslationCatalog` and `Locale` are defined in `waterui-locale` and
/// re-exported at `waterui::locale`. Most callers have the `waterui` facade as
/// a dependency; generated crates that stay slim and do not depend on the
/// facade go through the component crate directly instead.
fn locale_crate_path() -> StdResult<TokenStream2, TokenStream2> {
    if let Ok(waterui) = waterui_crate_path() {
        return Ok(quote!(#waterui::locale));
    }
    match crate_name("waterui-locale") {
        Ok(FoundCrate::Itself) => Ok(own_crate_path("waterui-locale")),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(_) => Err(quote! {
            compile_error!("`catalog!` requires the `waterui` or `waterui-locale` crate as a dependency (either may be renamed; Cargo.toml must include one).");
        }),
    }
}

pub fn catalog(input: &TokenStream) -> TokenStream {
    if !input.is_empty() {
        return syn::Error::new(Span::call_site(), "catalog! does not accept arguments")
            .to_compile_error()
            .into();
    }

    let locale_mod = match locale_crate_path() {
        Ok(path) => path,
        Err(err) => return TokenStream::from(err),
    };

    let bundle = match TranslationBundle::load_from_manifest_dir() {
        Ok(bundle) => bundle,
        Err(err) => return TokenStream::from(compile_error(&err)),
    };

    let inserts: Vec<_> = bundle
        .tracked_files
        .iter()
        .filter_map(|path| {
            let locale = path.file_stem()?.to_str()?;
            let locale_lit = LitStr::new(locale, Span::call_site());
            let path_lit = LitStr::new(path.to_str()?, Span::call_site());
            Some(quote! {
                __catalog = __catalog
                    .add_toml(
                        #locale_lit
                            .parse::<#locale_mod::Locale>()
                            .expect("catalog! validated BCP-47 locale tag"),
                        include_str!(#path_lit),
                    )
                    .expect("catalog! validated translation file");
            })
        })
        .collect();

    TokenStream::from(quote! {{
        let mut __catalog = #locale_mod::TranslationCatalog::new();
        #(#inserts)*
        __catalog
    }})
}

#[cfg(test)]
mod tests {
    use std::{env, fs, process};

    use proc_macro2::Span;
    use syn::{Expr, Ident, parse_quote};
    use waterui_locale_parser::TranslationFile;

    use super::{TranslationBundle, build_binding_map, parse_placeholders};

    #[test]
    fn parse_placeholders_accepts_named_plural_spec_and_escaped_forms() {
        let placeholders = parse_placeholders("{{literal}} {name} {#count} {blur:.1} {a.b} {a[0]}")
            .expect("named, plural, spec, and field-access placeholders should parse");
        let forms: Vec<String> = placeholders
            .iter()
            .map(|placeholder| format!("{placeholder:?}"))
            .collect();
        assert_eq!(
            forms,
            [
                "Regular(\"name\")",
                "Plural(\"count\")",
                "Regular(\"blur\")",
                "Regular(\"a\")",
                "Regular(\"a\")",
            ]
        );
    }

    #[test]
    fn parse_placeholders_rejects_empty_placeholder() {
        let err = parse_placeholders("Count: {}").expect_err("`{}` should fail");
        assert!(err.contains("`{}`"));
        assert!(err.contains("bare identifiers"));
    }

    #[test]
    fn parse_placeholders_rejects_positional_placeholder() {
        let err = parse_placeholders("{0} items").expect_err("`{0}` should fail");
        assert!(err.contains("`{0}`"));
        assert!(err.contains("bare identifiers"));
    }

    #[test]
    fn parse_placeholders_rejects_positional_placeholder_with_format_spec() {
        let err = parse_placeholders(concat!("size ", "{1", ":>4}"))
            .expect_err("positional placeholder with format spec should fail");
        assert!(err.contains(concat!("`{1", ":>4}`")));
        assert!(err.contains("bare identifiers"));
    }

    #[test]
    fn parse_placeholders_rejects_raw_identifier_placeholder() {
        let err = parse_placeholders("{r#fn}").expect_err("`{r#fn}` should fail");
        assert!(err.contains("`{r#fn}`"));
    }

    #[test]
    fn parse_placeholders_rejects_non_identifier_placeholder() {
        let err = parse_placeholders("{foo-bar}").expect_err("`{foo-bar}` should fail");
        assert!(err.contains("`{foo-bar}`"));
        assert!(err.contains("bare identifiers"));
    }

    #[test]
    fn parse_toml_requires_plural_other() {
        let content = r#"
"I have {#count} apple" = { one = "I have {count} apple" }
"#;
        let err = TranslationFile::parse_named(content, "i18n/en.toml")
            .expect_err("missing 'other' should fail");
        assert!(err.to_string().contains("must define non-empty 'other'"));
    }

    #[test]
    fn parse_toml_rejects_unknown_plural_fields() {
        let content = r#"
"I have {#count} apple" = { one = "I have {count} apple", manyy = "oops", other = "I have {count} apples" }
"#;
        let err = TranslationFile::parse_named(content, "i18n/en.toml")
            .expect_err("unknown plural field should fail");
        assert!(err.to_string().contains("Unknown plural field"));
    }

    #[test]
    fn parse_toml_accepts_dual_plural_fields() {
        let content = r#"
"I have {#apples} apple and {#oranges} orange" = { one_one = "I have {apples} apple and {oranges} orange", one_other = "I have {apples} apple and {oranges} oranges", other_one = "I have {apples} apples and {oranges} orange", other_other = "I have {apples} apples and {oranges} oranges" }
"#;
        let parsed = TranslationFile::parse_named(content, "i18n/en.toml")
            .expect("valid dual plural should parse");
        assert!(parsed.contains_key("I have {#apples} apple and {#oranges} orange"));
    }

    #[test]
    fn parse_toml_rejects_unknown_dual_plural_fields() {
        let content = r#"
"I have {#apples} apple and {#oranges} orange" = { one_one = "x", one_other = "x", other_one = "x", other_others = "x" }
"#;
        let err = TranslationFile::parse_named(content, "i18n/en.toml")
            .expect_err("unknown dual plural field should fail");
        assert!(err.to_string().contains("Unknown dual plural field"));
    }

    #[test]
    fn repro_50_dual_plural_expresses_few_many() {
        // Polish needs `few` and `many` for both counts; a dual table must accept them.
        let content = r#"
"{#apples} jabłko i {#oranges} pomarańcza" = { one_one = "a", one_few = "b", few_one = "c", few_few = "d", other_other = "e" }
"#;
        TranslationFile::parse_named(content, "i18n/pl.toml")
            .expect("dual plural tables must be able to express CLDR few/many categories");
    }

    #[test]
    fn generate_dual_plural_arm_emits_arbitrary_cldr_pairs_and_fallbacks() {
        let content = r#"
"{#apples} items and {#oranges} items" = { zero_two = "zt", one_few = "of", few_many = "fm", one_other = "oo", other_many = "om", other_other = "other" }
"#;
        let file = TranslationFile::parse_named(content, "i18n/ar-EG.toml")
            .expect("dual plural categories should parse");
        let Some(super::TranslationValue::DualPlural(forms)) =
            file.get("{#apples} items and {#oranges} items")
        else {
            panic!("expected DualPlural");
        };
        let waterui = quote::quote!(::waterui);
        let locale: icu_locale_core::LanguageIdentifier = "ar-EG".parse().unwrap();
        let idents = [
            Ident::new("apples", Span::call_site()),
            Ident::new("oranges", Span::call_site()),
        ];
        let tokens = super::generate_dual_plural_translation_arm(
            &waterui,
            &locale,
            forms,
            &idents,
            &["apples", "oranges"],
        )
        .to_string();

        assert!(
            tokens
                .contains("PluralCategory :: Zero , :: waterui :: locale :: PluralCategory :: Two")
        );
        assert!(
            tokens
                .contains("PluralCategory :: One , :: waterui :: locale :: PluralCategory :: Few")
        );
        assert!(
            tokens
                .contains("PluralCategory :: Few , :: waterui :: locale :: PluralCategory :: Many")
        );
        assert!(tokens.contains("PluralCategory :: One , _"));
        assert!(tokens.contains("_ , :: waterui :: locale :: PluralCategory :: Many"));
        // Exact matches must precede wildcard fallbacks.
        let exact_pos = tokens
            .find("PluralCategory :: One , :: waterui :: locale :: PluralCategory :: Few")
            .unwrap();
        let fallback_pos = tokens.find("PluralCategory :: One , _").unwrap();
        assert!(exact_pos < fallback_pos);

        // Also cover CJK + de + en-US dual-plural files in one bundle check.
        for tag in ["zh-CN", "ja", "ko", "de", "en-US"] {
            let parsed = TranslationFile::parse_named(
                r#""{#a} {#b}" = { other_other = "{a} {b}" }"#,
                &format!("i18n/{tag}.toml"),
            )
            .expect("locale dual plural should parse");
            assert!(parsed.contains_key("{#a} {#b}"));
        }
    }

    #[test]
    fn load_from_dir_rejects_invalid_locale_filename() {
        let dir = env::temp_dir().join(format!("waterui-locale-invalid-tag-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp i18n dir");
        fs::write(dir.join("not a locale!.toml"), r#"hello = "Hello""#)
            .expect("write invalid locale file");

        let err = TranslationBundle::load_from_dir(&dir)
            .expect_err("invalid locale filename should produce an error");
        let _ = fs::remove_dir_all(&dir);
        assert!(err.contains("Invalid BCP-47 locale tag 'not a locale!'"));
    }

    #[test]
    fn build_binding_map_accepts_regular_plural_and_repeated_placeholders() {
        let format_str = "{#count} items for {name} ({count})";
        let placeholders = parse_placeholders(format_str).expect("valid placeholders");
        let bindings: Vec<(Ident, Expr)> = vec![
            (Ident::new("count", Span::call_site()), parse_quote!(42)),
            (Ident::new("name", Span::call_site()), parse_quote!("Alice")),
        ];
        let map = build_binding_map(format_str, &placeholders, &bindings)
            .expect("matching bindings should succeed");
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("count"));
        assert!(map.contains_key("name"));
    }

    #[test]
    fn build_binding_map_rejects_unused_binding() {
        let format_str = "{count} items";
        let placeholders = parse_placeholders(format_str).expect("valid placeholders");
        let bindings: Vec<(Ident, Expr)> = vec![(
            Ident::new("cnt", Span::call_site()),
            parse_quote!(files.len()),
        )];
        let err = build_binding_map(format_str, &placeholders, &bindings)
            .expect_err("unused binding should fail");
        assert_eq!(
            err.to_string(),
            "`cnt` is not a placeholder in \"{count} items\""
        );
    }

    #[test]
    fn build_binding_map_rejects_duplicate_and_unused_bindings_together() {
        let format_str = "{count} items";
        let placeholders = parse_placeholders(format_str).expect("valid placeholders");
        let bindings: Vec<(Ident, Expr)> = vec![
            (Ident::new("count", Span::call_site()), parse_quote!(1)),
            (Ident::new("count", Span::call_site()), parse_quote!(2)),
            (Ident::new("extra", Span::call_site()), parse_quote!(3)),
        ];
        let err = build_binding_map(format_str, &placeholders, &bindings)
            .expect_err("duplicate and unused bindings should fail");
        let messages: Vec<String> = err.into_iter().map(|e| e.to_string()).collect();
        assert_eq!(
            messages,
            vec![
                "duplicate binding `count` in `text!`".to_string(),
                "`extra` is not a placeholder in \"{count} items\"".to_string(),
            ]
        );
    }
}
