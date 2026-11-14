//! `apple_available!`: the OS-version selection between two realizations.

use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Literal, Span, TokenStream};
use quote::quote_spanned;
use syn::{LitFloat, LitInt, parse::Parser as _};

const USAGE: &str = "`apple_available!` takes one `<major>.<minor>` version, such as `27.0`";

/// The path to `waterui-apple-os`, which the expansion names.
fn apple_os_path(span: Span) -> syn::Result<TokenStream> {
    match crate_name("waterui-apple-os") {
        // `waterui-apple-os` declares `extern crate self as waterui_apple_os`,
        // so its own tests and doctests name it the way a dependent does.
        Ok(FoundCrate::Itself) => Ok(quote_spanned!(span=> ::waterui_apple_os)),
        Ok(FoundCrate::Name(name)) => {
            let ident = syn::Ident::new(&name, span);
            Ok(quote_spanned!(span=> ::#ident))
        }
        Err(error) => Err(syn::Error::new(
            span,
            format!(
                "`apple_available!` requires the `waterui-apple-os` crate as a dependency: {error}"
            ),
        )),
    }
}

/// Reads `27.0`, or a bare `27`, as `(major, minor)` with the literal's span.
fn parse_version(input: TokenStream) -> syn::Result<(u16, u16, Span)> {
    let parser = |stream: syn::parse::ParseStream<'_>| {
        let version = if stream.peek(LitFloat) {
            let literal: LitFloat = stream.parse()?;
            let digits = literal.base10_digits();
            let (major, minor) = digits
                .split_once('.')
                .and_then(|(major, minor)| Some((major.parse().ok()?, minor.parse().ok()?)))
                .ok_or_else(|| syn::Error::new(literal.span(), USAGE))?;
            (major, minor, literal.span())
        } else if stream.peek(LitInt) {
            let literal: LitInt = stream.parse()?;
            (literal.base10_parse()?, 0, literal.span())
        } else {
            return Err(stream.error(USAGE));
        };
        if stream.is_empty() {
            Ok(version)
        } else {
            Err(stream.error(USAGE))
        }
    };
    parser.parse2(input)
}

/// Expands `apple_available!(27.0)`.
pub fn expand(input: TokenStream) -> TokenStream {
    let (major, minor, span) = match parse_version(input) {
        Ok(version) => version,
        Err(error) => return error.into_compile_error(),
    };
    let os = match apple_os_path(span) {
        Ok(path) => path,
        Err(error) => return error.into_compile_error(),
    };
    let version = format!("{major}.{minor}")
        .parse::<Literal>()
        .expect("`<major>.<minor>` is a float literal");
    let (major, minor) = (
        Literal::u16_unsuffixed(major),
        Literal::u16_unsuffixed(minor),
    );
    // Naming `GATED_VERSION` is what ties the call site to the table: a
    // version the framework does not gate has no such item, and one the
    // deployment target already meets has a deprecated one.
    quote_spanned! {span=>
        {
            let () = #os::AppleOs::<#major, #minor>::GATED_VERSION;
            #os::objc2::available!(
                ios = #version,
                macos = #version,
                tvos = #version,
                watchos = #version,
                visionos = #version,
            )
        }
    }
}
