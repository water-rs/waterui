//! `tsx!` — mount a TypeScript view module from Rust.
//!
//! The macro does two things and nothing else: it turns the path written at
//! the call site into the module id the bundle publishes that module under,
//! and it records the mount in a `waterui_meta_tsx_*` `#[used] static` so the
//! `water` CLI learns which modules the binary mounts. Everything else is
//! [`Mount`](../../../waterui_ts/struct.Mount.html), an ordinary view.
//!
//! The expansion is a pure read. The macro stats one file to check it exists
//! and never opens it: parsing the `.tsx` to find its default export would be
//! recovering semantics from source text, which the CLI's bundler — the one
//! tool that has already resolved the module graph — does instead. A module
//! the bundle does not carry is a typed error at mount, naming the id and
//! the ids the bundle does have.

use std::path::{Component, Path, PathBuf};

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Expr, ExprStruct, LitStr, Token};

use crate::ts::{ts_path, ts_schema_path};

/// `tsx!("./promo.tsx", PromoProps { … })`, parsed.
#[derive(Debug)]
struct Tsx {
    /// The module path as written, relative to the file that mounts it.
    path: LitStr,
    /// The props struct literal, absent when the module takes no props.
    props: Option<ExprStruct>,
}

impl Parse for Tsx {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse::<LitStr>().map_err(|error| {
            syn::Error::new(
                error.span(),
                "tsx! takes the module's path as a string literal: \
                 tsx!(\"./promo.tsx\", PromoProps { … })",
            )
        })?;
        if input.is_empty() {
            return Ok(Self { path, props: None });
        }
        input.parse::<Token![,]>()?;
        if input.is_empty() {
            return Ok(Self { path, props: None });
        }
        let expression = input.parse::<Expr>()?;
        let Expr::Struct(props) = expression else {
            return Err(syn::Error::new_spanned(
                expression,
                "the props argument is a struct literal, so the props type is named where the \
                 module is mounted: write `PromoProps { … }` — a prepared value goes in with \
                 struct update syntax, `PromoProps { ..props }`",
            ));
        };
        if !input.is_empty() {
            return Err(input.error("tsx! takes a module path and at most one props literal"));
        }
        Ok(Self {
            path,
            props: Some(props),
        })
    }
}

/// Resolve `literal` — written relative to `caller` — into the module id the
/// bundle publishes the module under.
///
/// The id is the file's path relative to the crate's `CARGO_MANIFEST_DIR`,
/// with forward slashes and the extension kept, because the crate directory
/// is the root the CLI's bundler resolves modules against. Both ends are
/// canonicalized first, so `./`, `../` and a symlinked checkout all reduce to
/// the same id.
fn module_id(literal: &LitStr, caller: &Path, manifest_dir: &Path) -> syn::Result<String> {
    let written = literal.value();
    let span = literal.span();
    if written.is_empty() {
        return Err(syn::Error::new(span, "a module path cannot be empty"));
    }
    let written = PathBuf::from(&written);
    if written.is_absolute() {
        return Err(syn::Error::new(
            span,
            format!(
                "`{}` is an absolute path: a module is named relative to the Rust file that \
                 mounts it, so that moving the pair moves the reference with them",
                written.display()
            ),
        ));
    }
    let directory = caller.parent().ok_or_else(|| {
        syn::Error::new(
            span,
            format!(
                "the file mounting this module, `{}`, has no directory to resolve `{}` against",
                caller.display(),
                written.display()
            ),
        )
    })?;
    // `.` components are dropped before the join: they never change which
    // file is named, and leaving them in would put `src/./promo.tsx` in the
    // message a reader has to match against what they wrote.
    let mut candidate = directory.to_path_buf();
    for component in written.components() {
        if !matches!(component, Component::CurDir) {
            candidate.push(component.as_os_str());
        }
    }
    if !candidate.is_file() {
        return Err(syn::Error::new(
            span,
            format!(
                "no such TypeScript module: `{}` resolves to `{}`, which does not exist",
                written.display(),
                candidate.display()
            ),
        ));
    }
    // Canonicalized on both sides: the file is known to exist, and comparing
    // one real path against one symlinked one would call a module outside its
    // own crate.
    let candidate = candidate.canonicalize().map_err(|error| {
        syn::Error::new(
            span,
            format!("`{}` cannot be resolved: {error}", candidate.display()),
        )
    })?;
    let root = manifest_dir.canonicalize().map_err(|error| {
        syn::Error::new(
            span,
            format!(
                "the crate directory `{}` cannot be resolved: {error}",
                manifest_dir.display()
            ),
        )
    })?;
    let relative = candidate.strip_prefix(&root).map_err(|_| {
        syn::Error::new(
            span,
            format!(
                "`{}` is outside the crate: it resolves to `{}`, and a module id is that path \
                 relative to `{}`, the root the bundler resolves modules against",
                written.display(),
                candidate.display(),
                root.display()
            ),
        )
    })?;
    join_with_slashes(relative, span)
}

/// The path as a module id: forward slashes, whatever the host uses.
fn join_with_slashes(relative: &Path, span: Span) -> syn::Result<String> {
    let mut id = String::new();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err(syn::Error::new(
                span,
                format!(
                    "`{}` is not a plain relative path and cannot name a module",
                    relative.display()
                ),
            ));
        };
        let segment = segment.to_str().ok_or_else(|| {
            syn::Error::new(
                span,
                format!(
                    "`{}` is not valid UTF-8, and a module id crosses into JavaScript as a string",
                    relative.display()
                ),
            )
        })?;
        if !id.is_empty() {
            id.push('/');
        }
        id.push_str(segment);
    }
    Ok(id)
}

/// The metadata static's name: the prefix the CLI enumerates, plus the id with
/// everything an identifier cannot hold replaced.
///
/// Two mounts of the same module produce the same name, and two different
/// modules may collide after the replacement — neither matters, because each
/// expansion is a block of its own and the CLI reads the payload rather than
/// the name.
fn meta_ident(module: &str) -> syn::Ident {
    let mut name = String::from("waterui_meta_tsx_");
    for character in module.chars() {
        name.push(if character.is_ascii_alphanumeric() {
            character
        } else {
            '_'
        });
    }
    format_ident!("{name}")
}

/// Expand `tsx!`.
pub fn tsx(input: TokenStream) -> TokenStream {
    // The span of the first token is the call site's, and its file is the
    // Rust file the module path is written relative to. It is read from the
    // `proc_macro` token stream before `syn` reaches it, because
    // `proc_macro2` only carries `local_file` under a feature that costs
    // every dependent crate the location tables.
    let caller = input
        .clone()
        .into_iter()
        .next()
        .map_or_else(proc_macro::Span::call_site, |token| token.span())
        .local_file();
    let parsed = match syn::parse::<Tsx>(input) {
        Ok(parsed) => parsed,
        Err(error) => return error.into_compile_error().into(),
    };
    match expand(&parsed, caller.as_deref()) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// The expansion, once the input parses.
fn expand(parsed: &Tsx, caller: Option<&Path>) -> syn::Result<TokenStream2> {
    let span = parsed.path.span();
    let Some(ts) = ts_path() else {
        return Err(syn::Error::new(
            span,
            "tsx! mounts a TypeScript module through the WaterUI runtime, which this crate \
             cannot reach: depend on `waterui` with its `ts` feature, or on `waterui-ts`",
        ));
    };
    let schema = ts_schema_path()?;
    let caller = caller.ok_or_else(|| {
        syn::Error::new(
            span,
            "the compiler did not say which file this tsx! is written in, so the module path \
             has nothing to be relative to: this happens when the expansion has no source file \
             of its own, such as inside a doctest or a generated file",
        )
    })?;
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        syn::Error::new(
            span,
            "CARGO_MANIFEST_DIR is not set, so the crate directory a module id is relative to \
             is unknown: tsx! expands inside a Cargo build",
        )
    })?;
    let module = module_id(&parsed.path, caller, Path::new(&manifest_dir))?;
    let meta = meta_ident(&module);

    // A module that takes no props is still typed against a contract: the
    // empty one, whose hash the bundle declares like any other.
    let (props_type, props_value) = parsed.props.as_ref().map_or_else(
        || (quote!(#ts::NoProps), quote!(#ts::NoProps {})),
        |props| {
            let path = &props.path;
            (quote!(#path), quote!(#props))
        },
    );
    let name = quote!(#schema::struct_name(&<#props_type as #schema::TsType>::SCHEMA));
    let hash = quote!(<#props_type as #schema::TsProps>::CONTRACT_HASH);
    let meta_doc = format!("The mount of `{module}`, read back by the `water` CLI.");

    Ok(quote! {
        {
            #[doc = #meta_doc]
            #[cfg(debug_assertions)]
            #[used]
            #[allow(non_upper_case_globals)]
            static #meta: [u8; #schema::mount_encoded_len(#module, #name, #hash) + 1] =
                #schema::encode_mount(#module, #name, #hash);

            #ts::Mount::new::<#props_type>(#module, #props_value)
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{meta_ident, module_id};

    /// A literal with the span tests need; the value is what `module_id`
    /// reads.
    fn literal(value: &str) -> syn::LitStr {
        syn::LitStr::new(value, proc_macro2::Span::call_site())
    }

    /// This crate's own directory, and a file inside it that certainly
    /// exists, so the resolution runs against a real tree.
    fn manifest() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn a_sibling_module_is_named_from_the_crate_root() {
        let caller = manifest().join("src/tsx.rs");
        let id = module_id(&literal("./lib.rs"), &caller, manifest())
            .expect("a file beside the caller resolves");
        assert_eq!(id, "src/lib.rs");
    }

    #[test]
    fn a_path_without_a_leading_dot_resolves_the_same_way() {
        let caller = manifest().join("src/tsx.rs");
        let id =
            module_id(&literal("lib.rs"), &caller, manifest()).expect("a bare relative path works");
        assert_eq!(id, "src/lib.rs");
    }

    #[test]
    fn a_parent_hop_is_reduced_to_one_id() {
        let caller = manifest().join("src/scripts/setup.js");
        let id = module_id(&literal("../lib.rs"), &caller, manifest())
            .expect("a parent hop inside the crate resolves");
        assert_eq!(id, "src/lib.rs");
    }

    #[test]
    fn a_missing_file_names_the_path_it_resolved_to() {
        let caller = manifest().join("src/tsx.rs");
        let error = module_id(&literal("./promo.tsx"), &caller, manifest())
            .expect_err("the fixture does not exist");
        let message = error.to_string();
        assert!(message.contains("./promo.tsx"), "{message}");
        assert!(message.contains("src/promo.tsx"), "{message}");
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let caller = manifest().join("src/tsx.rs");
        let error = module_id(&literal("/etc/hosts"), &caller, manifest())
            .expect_err("a module is named relative to its mount site");
        assert!(error.to_string().contains("absolute"), "{error}");
    }

    #[test]
    fn a_module_outside_the_crate_is_refused() {
        let caller = manifest().join("src/tsx.rs");
        let error = module_id(&literal("../../Cargo.toml"), &caller, manifest())
            .expect_err("the workspace manifest is outside this crate");
        assert!(error.to_string().contains("outside the crate"), "{error}");
    }

    #[test]
    fn an_empty_path_is_refused() {
        let caller = manifest().join("src/tsx.rs");
        module_id(&literal(""), &caller, manifest()).expect_err("a module path names a file");
    }

    /// The grammar, without touching the filesystem: what parses, and what
    /// the refusals say.
    mod grammar {
        use quote::quote;

        use crate::tsx::Tsx;

        fn parse(input: proc_macro2::TokenStream) -> syn::Result<Tsx> {
            syn::parse2::<Tsx>(input)
        }

        #[test]
        fn a_module_path_alone_is_a_mount_with_no_props() {
            let parsed = parse(quote!("./promo.tsx")).expect("a path alone parses");
            assert_eq!(parsed.path.value(), "./promo.tsx");
            assert!(parsed.props.is_none());
        }

        #[test]
        fn a_trailing_comma_is_still_a_mount_with_no_props() {
            let parsed = parse(quote!("./promo.tsx",)).expect("a trailing comma parses");
            assert!(parsed.props.is_none());
        }

        #[test]
        fn a_struct_literal_names_the_props_type() {
            let parsed = parse(quote!("./promo.tsx", PromoProps { headline: title }))
                .expect("a struct literal parses");
            let props = parsed.props.expect("the props are carried");
            assert!(props.path.is_ident("PromoProps"));
        }

        #[test]
        fn a_props_expression_that_is_not_a_struct_literal_is_refused() {
            let error = parse(quote!("./promo.tsx", make_props()))
                .expect_err("the props type has to be named at the mount site");
            let message = error.to_string();
            assert!(message.contains("struct literal"), "{message}");
            assert!(message.contains("..props"), "{message}");
        }

        #[test]
        fn a_module_path_that_is_not_a_string_literal_is_refused() {
            let error =
                parse(quote!(MODULE, PromoProps {})).expect_err("the path is a string literal");
            assert!(error.to_string().contains("string literal"), "{error}");
        }

        #[test]
        fn a_third_argument_is_refused() {
            parse(quote!("./promo.tsx", PromoProps {}, extra))
                .expect_err("tsx! takes a path and at most one props literal");
        }
    }

    #[test]
    fn the_metadata_name_carries_the_prefix_the_cli_enumerates() {
        assert_eq!(
            meta_ident("src/views/promo.tsx"),
            "waterui_meta_tsx_src_views_promo_tsx"
        );
    }
}
