//! Proc macros for `WaterUI` i18n.
//!
//! This crate provides the `text!` macro for internationalized text.
//! The macro loads translation files from the `i18n/` folder at compile time.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use toml::value::Table;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Result, Token};

const VALID_PLURAL_FIELDS: &[&str] = &["zero", "one", "two", "few", "many", "other"];
const VALID_DUAL_PLURAL_FIELDS: &[&str] = &["one_one", "one_other", "other_one", "other_other"];

fn waterui_crate_path() -> std::result::Result<TokenStream2, TokenStream2> {
    if current_package_name().as_deref() == Some("waterui-internal") {
        return Ok(quote!(crate));
    }

    match crate_name("waterui") {
        Ok(FoundCrate::Itself) => Ok(quote!(::waterui)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Ok(quote!(::#ident))
        }
        Err(_) => Err(quote! {
            compile_error!("`text!` requires the `waterui` crate as a dependency (it may be renamed; Cargo.toml must include it).");
        }),
    }
}

fn current_package_name() -> Option<String> {
    std::env::var("CARGO_PKG_NAME").ok()
}

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
    DualPlural {
        one_one: Option<String>,
        one_other: Option<String>,
        other_one: Option<String>,
        other_other: String,
    },
}

#[derive(Clone, Copy)]
struct PluralFormsRef<'a> {
    zero: Option<&'a String>,
    one: Option<&'a String>,
    two: Option<&'a String>,
    few: Option<&'a String>,
    many: Option<&'a String>,
    other: &'a str,
}

#[derive(Clone, Copy)]
struct DualPluralFormsRef<'a> {
    one_one: Option<&'a String>,
    one_other: Option<&'a String>,
    other_one: Option<&'a String>,
    other_other: &'a str,
}

/// All translations loaded from i18n folder
#[derive(Debug, Default)]
struct TranslationBundle {
    /// Map of locale code -> (key -> value)
    locales: BTreeMap<String, BTreeMap<String, TranslationValue>>,
    /// Translation files loaded from disk (used for build invalidation tracking).
    tracked_files: Vec<PathBuf>,
}

impl TranslationBundle {
    fn load_from_manifest_dir() -> std::result::Result<Self, String> {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
        let i18n_path = PathBuf::from(&manifest_dir).join("i18n");

        let mut bundle = Self::default();

        if !i18n_path.exists() {
            return Ok(bundle);
        }

        // Load all .toml files in i18n/
        let entries = std::fs::read_dir(&i18n_path).map_err(|err| {
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
            let content = std::fs::read_to_string(&path)
                .map_err(|err| format!("Failed to read '{}': {err}", path.display()))?;
            let translations = Self::parse_toml(&content, &path)?;

            bundle.locales.insert(stem.to_string(), translations);
            bundle.tracked_files.push(path);
        }

        Ok(bundle)
    }

    fn parse_plural_field(
        table: &Table,
        key: &str,
        source: &Path,
        translation_key: &str,
    ) -> std::result::Result<Option<String>, String> {
        match table.get(key) {
            None => Ok(None),
            Some(toml::Value::String(value)) => Ok(Some(value.clone())),
            Some(_) => Err(format!(
                "Plural field '{key}' for key '{translation_key}' in '{}' must be a string",
                source.display()
            )),
        }
    }

    fn parse_toml(
        content: &str,
        source: &Path,
    ) -> std::result::Result<BTreeMap<String, TranslationValue>, String> {
        let table: toml::Table = toml::from_str(content)
            .map_err(|err| format!("Failed to parse '{}': {err}", source.display()))?;
        let mut translations = BTreeMap::new();

        for (key, value) in table {
            let tv =
                match value {
                    toml::Value::String(s) => TranslationValue::Simple(s),
                    toml::Value::Table(t) => {
                        let is_dual_plural = t.keys().any(|field| field.contains('_'));
                        if is_dual_plural {
                            for field in t.keys() {
                                if !VALID_DUAL_PLURAL_FIELDS.contains(&field.as_str()) {
                                    return Err(format!(
                                        "Unknown dual plural field '{}' for key '{}' in '{}'",
                                        field,
                                        key,
                                        source.display()
                                    ));
                                }
                            }

                            let one_one = Self::parse_plural_field(&t, "one_one", source, &key)?;
                            let one_other =
                                Self::parse_plural_field(&t, "one_other", source, &key)?;
                            let other_one =
                                Self::parse_plural_field(&t, "other_one", source, &key)?;
                            let other_other =
                            Self::parse_plural_field(&t, "other_other", source, &key)?
                                .filter(|s| !s.trim().is_empty())
                                .ok_or_else(|| {
                                    format!(
                                "Dual plural key '{}' in '{}' must define non-empty 'other_other'",
                                key,
                                source.display()
                            )
                                })?;

                            TranslationValue::DualPlural {
                                one_one,
                                one_other,
                                other_one,
                                other_other,
                            }
                        } else {
                            for field in t.keys() {
                                if !VALID_PLURAL_FIELDS.contains(&field.as_str()) {
                                    return Err(format!(
                                        "Unknown plural field '{}' for key '{}' in '{}'",
                                        field,
                                        key,
                                        source.display()
                                    ));
                                }
                            }

                            let zero = Self::parse_plural_field(&t, "zero", source, &key)?;
                            let one = Self::parse_plural_field(&t, "one", source, &key)?;
                            let two = Self::parse_plural_field(&t, "two", source, &key)?;
                            let few = Self::parse_plural_field(&t, "few", source, &key)?;
                            let many = Self::parse_plural_field(&t, "many", source, &key)?;
                            let other = Self::parse_plural_field(&t, "other", source, &key)?
                                .filter(|s| !s.trim().is_empty())
                                .ok_or_else(|| {
                                    format!(
                                        "Plural key '{}' in '{}' must define non-empty 'other'",
                                        key,
                                        source.display()
                                    )
                                })?;

                            TranslationValue::Plural {
                                zero,
                                one,
                                two,
                                few,
                                many,
                                other,
                            }
                        }
                    }
                    _ => {
                        return Err(format!(
                            "Invalid translation value type for key '{}' in '{}'",
                            key,
                            source.display()
                        ));
                    }
                };
            translations.insert(key, tv);
        }

        Ok(translations)
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
            let base = content.split(['.', '[']).next().unwrap_or("").trim();

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
    let body = quote! { #waterui::reactive::__alloc::format!(#format_str) };
    build_signal_map(waterui, idents, &body)
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

fn build_binding_map(bindings: &[(Ident, Expr)]) -> HashMap<String, &Expr> {
    bindings
        .iter()
        .map(|(name, expr)| (name.to_string(), expr))
        .collect()
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
    for (locale_code, translations) in &bundle.locales {
        if let Some(value) = translations.get(translation_key) {
            let arm =
                generate_translation_arm(waterui, locale_code, value, all_idents, plural_names);
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
pub fn text(input: &TokenStream) -> TokenStream {
    let input = match syn::parse::<TextInput>(input.clone()) {
        Ok(input) => input,
        Err(err) => return err.to_compile_error().into(),
    };
    let expanded = expand_text_macro(&input);
    TokenStream::from(expanded)
}

fn expand_text_macro(input: &TextInput) -> TokenStream2 {
    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(err) => return err,
    };

    let key = input.format_string.value();
    let translation_key = make_key(&key, input.context.as_deref());
    let placeholders = parse_placeholders(&key);

    // Load translations at compile time
    let bundle = match TranslationBundle::load_from_manifest_dir() {
        Ok(bundle) => bundle,
        Err(err) => {
            let message = LitStr::new(&err, Span::call_site());
            return quote! { compile_error!(#message); };
        }
    };

    let plural_names = collect_plural_names(&placeholders);
    let binding_map = build_binding_map(&input.bindings);
    let (captures, all_idents) = build_captures_and_idents(&waterui, &placeholders, &binding_map);
    let locale_arms = build_locale_arms(
        &bundle,
        &translation_key,
        &waterui,
        &all_idents,
        &plural_names,
    );

    let tracked_file_lits: Vec<LitStr> = bundle
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
            #(let _ = include_bytes!(#tracked_file_lits);)*

            #waterui::text::Text::localized_with({
                #(#captures)*

                move |_env: &#waterui::Environment, locale: &#waterui::locale::Locale| {
                    let resolve = |locale_key: &str| -> Option<#waterui::text::TextConfig> {
                        match locale_key {
                            #(#locale_arms)*
                            _ => None,
                        }
                    };

                    for fallback_locale in #waterui::locale::locale::get_fallback_chain(locale) {
                        // Match translation tables by language-id form (without extensions),
                        // e.g. "en-GB", even if runtime locale is "en-GB-u-hc-h23".
                        let locale_key = fallback_locale.id().to_string();
                        if let Some(text) = resolve(locale_key.as_str()) {
                            return text;
                        }
                    }

                    if let Some(text) = resolve("en") {
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
    locale_code: &str,
    value: &TranslationValue,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> TokenStream2 {
    match value {
        TranslationValue::Simple(text) => {
            generate_simple_translation_arm(waterui, locale_code, text, all_idents)
        }
        TranslationValue::Plural {
            zero,
            one,
            two,
            few,
            many,
            other,
        } => generate_plural_translation_arm(
            waterui,
            locale_code,
            PluralFormsRef {
                zero: zero.as_ref(),
                one: one.as_ref(),
                two: two.as_ref(),
                few: few.as_ref(),
                many: many.as_ref(),
                other,
            },
            all_idents,
            plural_names,
        ),
        TranslationValue::DualPlural {
            one_one,
            one_other,
            other_one,
            other_other,
        } => generate_dual_plural_translation_arm(
            waterui,
            locale_code,
            DualPluralFormsRef {
                one_one: one_one.as_ref(),
                one_other: one_other.as_ref(),
                other_one: other_one.as_ref(),
                other_other,
            },
            all_idents,
            plural_names,
        ),
    }
}

fn generate_simple_translation_arm(
    waterui: &TokenStream2,
    locale_code: &str,
    text: &str,
    all_idents: &[Ident],
) -> TokenStream2 {
    let format_lit = translation_format_lit(text);
    let text_config = build_text_config_expr(waterui, &format_lit, all_idents);
    quote! {
        #locale_code => {
            Some(#text_config)
        }
    }
}

fn push_plural_category_arm(
    category_arms: &mut Vec<TokenStream2>,
    waterui: &TokenStream2,
    category: &TokenStream2,
    text: Option<&String>,
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
    locale_code: &str,
    forms: PluralFormsRef<'_>,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> TokenStream2 {
    let plural_var = plural_names.first().copied().unwrap_or("count");
    let plural_ident = Ident::new(plural_var, proc_macro2::Span::call_site());
    let mut category_arms = Vec::new();
    let zero = quote! { #waterui::locale::PluralCategory::Zero };
    let one = quote! { #waterui::locale::PluralCategory::One };
    let two = quote! { #waterui::locale::PluralCategory::Two };
    let few = quote! { #waterui::locale::PluralCategory::Few };
    let many = quote! { #waterui::locale::PluralCategory::Many };
    push_plural_category_arm(&mut category_arms, waterui, &zero, forms.zero);
    push_plural_category_arm(&mut category_arms, waterui, &one, forms.one);
    push_plural_category_arm(&mut category_arms, waterui, &two, forms.two);
    push_plural_category_arm(&mut category_arms, waterui, &few, forms.few);
    push_plural_category_arm(&mut category_arms, waterui, &many, forms.many);

    let other_format_lit = translation_format_lit(forms.other);
    let locale_ident = unique_ident("__waterui_locale_value", all_idents);
    let body = quote! {
        let category = #waterui::locale::select_plural(&#locale_ident, #plural_ident);
        match category {
            #(#category_arms)*
            _ => #waterui::reactive::__alloc::format!(#other_format_lit),
        }
    };
    let content = build_signal_map(waterui, all_idents, &body);

    quote! {
        #locale_code => {
            let #locale_ident = locale.clone();
            Some(#waterui::text::TextConfig::new(#content))
        }
    }
}

fn optional_format_expr(
    waterui: &TokenStream2,
    format_lit: Option<LitStr>,
    fallback: &LitStr,
) -> TokenStream2 {
    format_lit.map_or_else(
        || quote! { #waterui::reactive::__alloc::format!(#fallback) },
        |lit| quote! { #waterui::reactive::__alloc::format!(#lit) },
    )
}

fn generate_dual_plural_translation_arm(
    waterui: &TokenStream2,
    locale_code: &str,
    forms: DualPluralFormsRef<'_>,
    all_idents: &[Ident],
    plural_names: &[&str],
) -> TokenStream2 {
    let plural_var_1 = plural_names.first().copied().unwrap_or("count");
    let plural_var_2 = plural_names.get(1).copied().unwrap_or(plural_var_1);
    let plural_ident_1 = Ident::new(plural_var_1, proc_macro2::Span::call_site());
    let plural_ident_2 = Ident::new(plural_var_2, proc_macro2::Span::call_site());

    let one_one_format = forms.one_one.map(|text| translation_format_lit(text));
    let one_other_format = forms.one_other.map(|text| translation_format_lit(text));
    let other_one_format = forms.other_one.map(|text| translation_format_lit(text));
    let other_other_lit = translation_format_lit(forms.other_other);
    let locale_ident = unique_ident("__waterui_locale_value", all_idents);

    let one_one_expr = optional_format_expr(waterui, one_one_format, &other_other_lit);
    let one_other_expr = optional_format_expr(waterui, one_other_format, &other_other_lit);
    let other_one_expr = optional_format_expr(waterui, other_one_format, &other_other_lit);

    let body = quote! {
        let category_1 = #waterui::locale::select_plural(&#locale_ident, #plural_ident_1);
        let category_2 = #waterui::locale::select_plural(&#locale_ident, #plural_ident_2);
        match (category_1, category_2) {
            (#waterui::locale::PluralCategory::One, #waterui::locale::PluralCategory::One) => {
                #one_one_expr
            }
            (#waterui::locale::PluralCategory::One, _) => {
                #one_other_expr
            }
            (_, #waterui::locale::PluralCategory::One) => {
                #other_one_expr
            }
            _ => #waterui::reactive::__alloc::format!(#other_other_lit),
        }
    };
    let content = build_signal_map(waterui, all_idents, &body);

    quote! {
        #locale_code => {
            let #locale_ident = locale.clone();
            Some(#waterui::text::TextConfig::new(#content))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::TranslationBundle;

    #[test]
    fn parse_toml_requires_plural_other() {
        let content = r#"
"I have {#count} apple" = { one = "I have {count} apple" }
"#;
        let source = PathBuf::from("i18n/en.toml");
        let err = TranslationBundle::parse_toml(content, &source)
            .expect_err("missing 'other' should fail");
        assert!(err.contains("must define non-empty 'other'"));
    }

    #[test]
    fn parse_toml_rejects_unknown_plural_fields() {
        let content = r#"
"I have {#count} apple" = { one = "I have {count} apple", manyy = "oops", other = "I have {count} apples" }
"#;
        let source = PathBuf::from("i18n/en.toml");
        let err = TranslationBundle::parse_toml(content, &source)
            .expect_err("unknown plural field should fail");
        assert!(err.contains("Unknown plural field"));
    }

    #[test]
    fn parse_toml_accepts_dual_plural_fields() {
        let content = r#"
"I have {#apples} apple and {#oranges} orange" = { one_one = "I have {apples} apple and {oranges} orange", one_other = "I have {apples} apple and {oranges} oranges", other_one = "I have {apples} apples and {oranges} orange", other_other = "I have {apples} apples and {oranges} oranges" }
"#;
        let source = PathBuf::from("i18n/en.toml");
        let parsed = TranslationBundle::parse_toml(content, &source)
            .expect("valid dual plural should parse");
        assert!(parsed.contains_key("I have {#apples} apple and {#oranges} orange"));
    }

    #[test]
    fn parse_toml_rejects_unknown_dual_plural_fields() {
        let content = r#"
"I have {#apples} apple and {#oranges} orange" = { one_one = "x", one_other = "x", other_one = "x", other_others = "x" }
"#;
        let source = PathBuf::from("i18n/en.toml");
        let err = TranslationBundle::parse_toml(content, &source)
            .expect_err("unknown dual plural field should fail");
        assert!(err.contains("Unknown dual plural field"));
    }
}

pub fn catalog(input: &TokenStream) -> TokenStream {
    if !input.is_empty() {
        return syn::Error::new(Span::call_site(), "catalog! does not accept arguments")
            .to_compile_error()
            .into();
    }

    let waterui = match waterui_crate_path() {
        Ok(path) => path,
        Err(err) => return TokenStream::from(err),
    };

    let bundle = match TranslationBundle::load_from_manifest_dir() {
        Ok(bundle) => bundle,
        Err(err) => {
            let message = LitStr::new(&err, Span::call_site());
            return TokenStream::from(quote! { compile_error!(#message); });
        }
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
                    .add_toml(#locale_lit, include_str!(#path_lit))
                    .expect("catalog! generated invalid translation file");
            })
        })
        .collect();

    TokenStream::from(quote! {{
        let mut __catalog = #waterui::locale::TranslationCatalog::new();
        #(#inserts)*
        __catalog
    }})
}
