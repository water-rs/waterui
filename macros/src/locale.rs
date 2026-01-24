//! Proc macros for WaterUI i18n.
//!
//! This crate provides the `text!` macro for internationalized text.
//! The macro loads translation files from the `i18n/` folder at compile time.

use std::collections::BTreeMap;
use std::path::PathBuf;

use icu_locid::Locale;
use icu_locid_transform::fallback::LocaleFallbacker;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Result, Token, parse_macro_input};

/// Parsed translation value from TOML
#[derive(Debug, Clone)]
enum TranslationValue {
    Simple(String),
    Plural {
        zero: Option<String>,
        one: Option<String>,
        two: Option<String>,
        few: Option<String>,
        many: Option<String>,
        other: String,
    },
}

/// All translations loaded from i18n folder
#[derive(Debug, Default)]
struct TranslationBundle {
    /// Map of locale code -> (key -> value)
    locales: BTreeMap<String, BTreeMap<String, TranslationValue>>,
}

impl TranslationBundle {
    fn load_from_manifest_dir() -> Self {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        let i18n_path = PathBuf::from(&manifest_dir).join("i18n");

        let mut bundle = Self::default();

        if !i18n_path.exists() {
            return bundle;
        }

        // Load all .toml files in i18n/
        if let Ok(entries) = std::fs::read_dir(&i18n_path) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(translations) = Self::parse_toml(&content) {
                                bundle.locales.insert(stem.to_string(), translations);
                            }
                        }
                    }
                }
            }
        }

        bundle
    }

    fn parse_toml(
        content: &str,
    ) -> std::result::Result<BTreeMap<String, TranslationValue>, toml::de::Error> {
        let table: toml::Table = toml::from_str(content)?;
        let mut translations = BTreeMap::new();

        for (key, value) in table {
            let tv = match value {
                toml::Value::String(s) => TranslationValue::Simple(s),
                toml::Value::Table(t) => {
                    // Plural forms
                    let get_str = |k: &str| t.get(k).and_then(|v| v.as_str()).map(String::from);
                    TranslationValue::Plural {
                        zero: get_str("zero"),
                        one: get_str("one"),
                        two: get_str("two"),
                        few: get_str("few"),
                        many: get_str("many"),
                        other: get_str("other").unwrap_or_default(),
                    }
                }
                _ => continue,
            };
            translations.insert(key, tv);
        }

        Ok(translations)
    }

    /// Get translation for a key and locale with proper CLDR fallback chain
    fn get(&self, locale_str: &str, key: &str) -> Option<&TranslationValue> {
        // Parse locale string into ICU Locale
        let locale: Locale = locale_str.parse().unwrap_or_else(|_| "en".parse().unwrap());

        // Use ICU4X LocaleFallbacker for proper CLDR fallback chain
        let fallbacker = LocaleFallbacker::new();
        let mut fallback_iter = fallbacker
            .for_config(Default::default())
            .fallback_for(locale.into());

        loop {
            let current = fallback_iter.get();
            let locale_key = current.to_string();

            // Try this locale in the fallback chain
            if let Some(translations) = self.locales.get(&locale_key) {
                if let Some(value) = translations.get(key) {
                    return Some(value);
                }
            }

            // Check if we've reached the end of the fallback chain
            if current.is_und() {
                break;
            }
            fallback_iter.step();
        }

        // Final fallback to English if not already tried
        if locale_str != "en" {
            if let Some(translations) = self.locales.get("en") {
                return translations.get(key);
            }
        }

        None
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
fn parse_placeholders(format_string: &str) -> Vec<Placeholder> {
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

            let mut content = String::new();
            while let Some(&c) = chars.peek() {
                if c == '}' {
                    chars.next();
                    break;
                }
                if c == ':' {
                    chars.next();
                    while let Some(&spec_c) = chars.peek() {
                        if spec_c == '}' {
                            chars.next();
                            break;
                        }
                        chars.next();
                    }
                    break;
                }
                content.push(c);
                chars.next();
            }

            let content = content.trim();
            if content.is_empty() {
                continue;
            }

            let content = content.strip_suffix('=').unwrap_or(content);
            let base = content
                .split(|ch| ch == '.' || ch == '[')
                .next()
                .unwrap_or("")
                .trim();

            if is_valid_ident(base) {
                let name = base.to_string();
                if is_plural {
                    placeholders.push(Placeholder::Plural(name));
                } else {
                    placeholders.push(Placeholder::Regular(name));
                }
            }
        }
    }

    placeholders
}

fn is_valid_ident(name: &str) -> bool {
    syn::parse_str::<Ident>(name).is_ok()
}

fn build_zip_expr_and_pattern(idents: &[Ident]) -> (TokenStream2, TokenStream2) {
    let mut iter = idents.iter();
    let first = iter
        .next()
        .expect("zip expressions require at least one identifier");
    let mut expr = quote! { #first.clone() };
    let mut pattern = quote! { #first };

    for ident in iter {
        expr = quote! { ::waterui::reactive::zip::zip(#expr, #ident.clone()) };
        pattern = quote! { (#pattern, #ident) };
    }

    (expr, pattern)
}

fn build_signal_map(idents: &[Ident], body: TokenStream2) -> TokenStream2 {
    match idents.len() {
        0 => quote! { ::waterui::reactive::constant(#body) },
        1 => {
            let ident = &idents[0];
            quote! {
                {
                    let __signal = #ident.clone();
                    ::waterui::reactive::SignalExt::map(&__signal, move |#ident| {
                        #body
                    })
                }
            }
        }
        _ => {
            let (zip_expr, pattern) = build_zip_expr_and_pattern(idents);
            quote! {
                {
                    let __zipped = #zip_expr;
                    ::waterui::reactive::SignalExt::map(&__zipped, move |#pattern| {
                        #body
                    })
                }
            }
        }
    }
}

fn build_format_signal(format_str: &LitStr, idents: &[Ident]) -> TokenStream2 {
    let body = quote! { ::waterui::reactive::__alloc::format!(#format_str) };
    build_signal_map(idents, body)
}

fn unique_ident(base: &str, existing: &[Ident]) -> Ident {
    let mut name = base.to_string();
    while existing.iter().any(|ident| ident.to_string() == name) {
        name.push('_');
    }
    Ident::new(&name, Span::call_site())
}

/// Generate the translation key from format string and optional context.
fn make_key(format_string: &str, context: Option<&str>) -> String {
    match context {
        Some(ctx) => format!("{format_string}#{ctx}"),
        None => format_string.to_string(),
    }
}

/// Macro for creating localized text.
///
/// The `text!` macro loads translations from `i18n/` folder at compile time
/// and generates a `LocalizedText` view that renders based on the current locale.
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
/// ```rust,ignore
/// // Simple text
/// text!("Hello, World!")
///
/// // Named placeholders (captured from scope)
/// text!("Hello, {name}")
///
/// // Plural source (for CLDR plural rules)
/// text!("I have {#count} apple")
///
/// // Context for homographs
/// text!("Right" @ "direction")
///
/// // Explicit binding
/// text!("Hello, {name}", name = get_name())
/// ```
pub(crate) fn text(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as TextInput);
    let expanded = expand_text_macro(input);
    TokenStream::from(expanded)
}

fn expand_text_macro(input: TextInput) -> TokenStream2 {
    let key = input.format_string.value();
    let translation_key = make_key(&key, input.context.as_deref());
    let placeholders = parse_placeholders(&key);

    // Load translations at compile time
    let bundle = TranslationBundle::load_from_manifest_dir();

    // Collect all placeholder names
    let mut plural_names: Vec<&str> = Vec::new();

    for ph in &placeholders {
        if let Placeholder::Plural(name) = ph {
            plural_names.push(name);
        }
    }

    // Build binding expressions
    let binding_map: std::collections::HashMap<String, &Expr> = input
        .bindings
        .iter()
        .map(|(name, expr)| (name.to_string(), expr))
        .collect();

    // Generate capture statements for all placeholders
    let mut captures = Vec::new();
    let mut all_names: Vec<String> = Vec::new();

    for ph in &placeholders {
        let name = match ph {
            Placeholder::Regular(n) | Placeholder::Plural(n) => n,
        };

        if !all_names.contains(name) {
            all_names.push(name.clone());
            let name_ident = Ident::new(name, proc_macro2::Span::call_site());

            if let Some(expr) = binding_map.get(name) {
                // Clone the expression to get an owned value (works for both &T and T)
                captures.push(quote! {
                    let #name_ident = ::waterui::reactive::__alloc::borrow::ToOwned::to_owned(#expr);
                });
            } else {
                // Auto-capture from scope, use ToOwned to handle both &T and T
                captures.push(quote! {
                    let #name_ident = ::waterui::reactive::__alloc::borrow::ToOwned::to_owned(#name_ident);
                });
            }
        }
    }

    let all_idents: Vec<Ident> = all_names
        .iter()
        .map(|name| Ident::new(name, Span::call_site()))
        .collect();

    // Generate match arms for each locale using fallback resolution
    let mut locale_arms = Vec::new();

    for locale_code in bundle.locales.keys() {
        // Use bundle.get() to resolve with fallback chain (e.g., zh-TW → zh-Hant → zh → en)
        if let Some(value) = bundle.get(locale_code, &translation_key) {
            let arm = generate_translation_arm(locale_code, value, &all_idents, &plural_names);
            locale_arms.push(arm);
        }
    }

    // Default fallback - use the key itself
    let default_format = key.replace("{#", "{");
    let default_format_lit = LitStr::new(&default_format, Span::call_site());
    let default_body = if all_idents.is_empty() {
        quote! { ::waterui::text::Text::new(#default_format_lit) }
    } else {
        let content = build_format_signal(&default_format_lit, &all_idents);
        quote! {
            ::waterui::text::Text::new(#content)
        }
    };

    // Generate the LocalizedText
    quote! {
        ::waterui::locale::LocalizedText::new({
            #(#captures)*

            move |locale: &::waterui::locale::Locale| {
                let lang = locale.language.as_str();
                let region = locale.region.as_ref().map(|r| r.as_str());
                let locale_key = match region {
                    Some(r) => ::std::format!("{}-{}", lang, r),
                    None => lang.to_string(),
                };

                match locale_key.as_str() {
                    #(#locale_arms)*
                    _ => match lang {
                        #(#locale_arms)*
                        _ => {
                            #default_body
                        }
                    }
                }
            }
        })
    }
}

fn generate_translation_arm(
    locale_code: &str,
    value: &TranslationValue,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> TokenStream2 {
    let locale_pattern = locale_code;

    match value {
        TranslationValue::Simple(text) => {
            // Replace {#name} with {name} for format!
            let format_str = text.replace("{#", "{");
            let format_lit = LitStr::new(&format_str, Span::call_site());

            if all_idents.is_empty() {
                quote! {
                    #locale_pattern => {
                        ::waterui::text::Text::new(#format_lit)
                    }
                }
            } else {
                let content = build_format_signal(&format_lit, all_idents);
                quote! {
                    #locale_pattern => {
                        ::waterui::text::Text::new(#content)
                    }
                }
            }
        }
        TranslationValue::Plural {
            zero,
            one,
            two,
            few,
            many,
            other,
        } => {
            // Get the first plural source variable
            let plural_var = plural_names.first().copied().unwrap_or("count");
            let plural_ident = Ident::new(plural_var, proc_macro2::Span::call_site());

            // Generate plural category match arms
            let mut category_arms = Vec::new();

            if let Some(text) = zero {
                let format_str = text.replace("{#", "{");
                let format_lit = LitStr::new(&format_str, Span::call_site());
                category_arms.push(quote! {
                    ::waterui::locale::PluralCategory::Zero => {
                        ::waterui::reactive::__alloc::format!(#format_lit)
                    }
                });
            }
            if let Some(text) = one {
                let format_str = text.replace("{#", "{");
                let format_lit = LitStr::new(&format_str, Span::call_site());
                category_arms.push(quote! {
                    ::waterui::locale::PluralCategory::One => {
                        ::waterui::reactive::__alloc::format!(#format_lit)
                    }
                });
            }
            if let Some(text) = two {
                let format_str = text.replace("{#", "{");
                let format_lit = LitStr::new(&format_str, Span::call_site());
                category_arms.push(quote! {
                    ::waterui::locale::PluralCategory::Two => {
                        ::waterui::reactive::__alloc::format!(#format_lit)
                    }
                });
            }
            if let Some(text) = few {
                let format_str = text.replace("{#", "{");
                let format_lit = LitStr::new(&format_str, Span::call_site());
                category_arms.push(quote! {
                    ::waterui::locale::PluralCategory::Few => {
                        ::waterui::reactive::__alloc::format!(#format_lit)
                    }
                });
            }
            if let Some(text) = many {
                let format_str = text.replace("{#", "{");
                let format_lit = LitStr::new(&format_str, Span::call_site());
                category_arms.push(quote! {
                    ::waterui::locale::PluralCategory::Many => {
                        ::waterui::reactive::__alloc::format!(#format_lit)
                    }
                });
            }

            let other_format = other.replace("{#", "{");
            let other_format_lit = LitStr::new(&other_format, Span::call_site());
            let locale_ident = unique_ident("__waterui_locale_value", all_idents);
            let body = quote! {
                let category = ::waterui::locale::select_plural(&#locale_ident, #plural_ident);
                match category {
                    #(#category_arms)*
                    _ => ::waterui::reactive::__alloc::format!(#other_format_lit),
                }
            };
            let content = build_signal_map(all_idents, body);

            quote! {
                #locale_pattern => {
                    let #locale_ident = locale.clone();
                    ::waterui::text::Text::new(#content)
                }
            }
        }
    }
}
