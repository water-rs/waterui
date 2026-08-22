//! `@{...}` interpolation for the `exec!` / `eval!` / `js_file!` macros.
//!
//! The sigil is `@{}` rather than `${}` because `${}` is JavaScript's own
//! template-literal interpolation — stealing it would make `` js!("`hi ${name}`") ``
//! impossible to write. `@{` is not valid JavaScript anywhere, so there is no
//! ambiguity; `@@{` escapes a literal `@{`.

use core::fmt::Write as _;

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, LitStr, Token};

/// The `<receiver>, "<source>"` shape shared by `exec!` and `eval!`.
pub struct ScriptCall {
    pub receiver: Expr,
    pub source: LitStr,
}

impl Parse for ScriptCall {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let receiver = input.parse()?;
        input.parse::<Token![,]>()?;
        let source = input.parse()?;
        Ok(Self { receiver, source })
    }
}

/// The `<receiver>, "<path>"` shape of `js_file!`.
pub struct ScriptFile {
    pub receiver: Option<Expr>,
    pub path: LitStr,
}

impl Parse for ScriptFile {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse::<LitStr>();
        if let Ok(path) = path {
            return Ok(Self {
                receiver: None,
                path,
            });
        }
        let receiver = input.parse()?;
        input.parse::<Token![,]>()?;
        Ok(Self {
            receiver: Some(receiver),
            path: input.parse()?,
        })
    }
}

/// Source with its `@{...}` holes replaced by parameter names, and the Rust
/// expressions those parameters bind to.
pub struct Interpolated {
    pub source: String,
    pub args: Vec<Expr>,
}

/// Rewrites `@{expr}` holes into `__wa0`, `__wa1`, … and collects the expressions.
///
/// # Errors
///
/// Returns an error for an unterminated hole, an empty hole, or a hole whose
/// contents are not a Rust expression.
pub fn interpolate(literal: &LitStr) -> syn::Result<Interpolated> {
    let raw = literal.value();
    let mut source = String::with_capacity(raw.len());
    let mut args = Vec::new();
    let mut rest = raw.as_str();

    while let Some(at) = rest.find('@') {
        source.push_str(&rest[..at]);
        rest = &rest[at..];

        // `@@{` escapes a literal `@{`; a lone `@` is ordinary JavaScript.
        if let Some(tail) = rest.strip_prefix("@@{") {
            source.push_str("@{");
            rest = tail;
            continue;
        }
        let Some(tail) = rest.strip_prefix("@{") else {
            source.push('@');
            rest = &rest[1..];
            continue;
        };

        let end = find_closing_brace(tail).ok_or_else(|| {
            syn::Error::new(
                literal.span(),
                "unterminated `@{` in JavaScript source; close it with `}`",
            )
        })?;
        let expression = tail[..end].trim();
        if expression.is_empty() {
            return Err(syn::Error::new(
                literal.span(),
                "empty `@{}` in JavaScript source",
            ));
        }
        let expression: Expr = syn::parse_str(expression).map_err(|error| {
            syn::Error::new(
                literal.span(),
                format!("`@{{{expression}}}` is not a Rust expression: {error}"),
            )
        })?;

        write!(source, "__wa{}", args.len()).expect("writing to a String cannot fail");
        args.push(expression);
        rest = &tail[end + 1..];
    }
    source.push_str(rest);

    Ok(Interpolated { source, args })
}

/// Finds the `}` closing a hole, allowing nested braces so `@{Foo { a: 1 }}`
/// works.
fn find_closing_brace(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, character) in text.char_indices() {
        match character {
            '{' => depth += 1,
            '}' if depth == 0 => return Some(index),
            '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Builds the `JsExpr`/`JsProgram` construction for an interpolated literal.
pub fn build(kind: &TokenStream, interpolated: &Interpolated) -> TokenStream {
    let source = &interpolated.source;
    let args = &interpolated.args;
    quote! {
        #kind::__from_parts(
            #source,
            ::std::vec![
                #(
                    ::waterui::webview::serde_json::to_value(&(#args))
                        .expect("a value interpolated into JavaScript must serialize")
                ),*
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{find_closing_brace, interpolate};
    use syn::LitStr;

    fn literal(value: &str) -> LitStr {
        LitStr::new(value, proc_macro2::Span::call_site())
    }

    #[test]
    fn holes_become_positional_parameters() {
        let result = interpolate(&literal("app.set(@{theme}, @{count})")).expect("interpolates");
        assert_eq!(result.source, "app.set(__wa0, __wa1)");
        assert_eq!(result.args.len(), 2);
    }

    /// The whole reason the sigil is `@{}`: template literals must survive.
    #[test]
    fn javascript_template_literals_are_untouched() {
        let result = interpolate(&literal("`hello ${name}`")).expect("interpolates");
        assert_eq!(result.source, "`hello ${name}`");
        assert!(result.args.is_empty());
    }

    #[test]
    fn a_doubled_sigil_escapes_a_literal_hole() {
        let result = interpolate(&literal("css`color: @@{c}`")).expect("interpolates");
        assert_eq!(result.source, "css`color: @{c}`");
        assert!(result.args.is_empty());
    }

    #[test]
    fn a_lone_at_sign_is_ordinary_source() {
        let result = interpolate(&literal("import '@scope/pkg'")).expect("interpolates");
        assert_eq!(result.source, "import '@scope/pkg'");
    }

    #[test]
    fn nested_braces_close_at_the_right_place() {
        // The inner `}` closes the nested brace at 11; the hole closes at 12.
        assert_eq!(find_closing_brace("Foo { a: 1 }} rest"), Some(12));
        let result = interpolate(&literal("f(@{Foo { a: 1 }})")).expect("interpolates");
        assert_eq!(result.source, "f(__wa0)");
    }

    #[test]
    fn malformed_holes_are_compile_errors() {
        assert!(interpolate(&literal("f(@{unterminated")).is_err());
        assert!(interpolate(&literal("f(@{})")).is_err());
        assert!(interpolate(&literal("f(@{let})")).is_err());
    }
}
