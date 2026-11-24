use core::mem::replace;
use std::env::var;

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Nothing, Parse, ParseStream};
use syn::token::{Brace, Else};
use syn::visit_mut::{self, VisitMut};
use syn::{
    Block, Expr, ExprGroup, ExprIf, ExprMacro, ExprMatch, ExprParen, ExprReturn, Ident, ImplItemFn,
    ItemFn, ItemImpl, ReturnType, Signature, Stmt, StmtMacro, TraitItemFn, Type, parse_quote,
};

pub fn expand_attribute(args: TokenStream, input: &TokenStream) -> TokenStream {
    if let Err(error) = syn::parse::<Nothing>(args) {
        return error.to_compile_error().into();
    }
    let runtime = match runtime_paths() {
        Ok(path) => path,
        Err(error) => return error.into(),
    };

    if let Ok(mut method) = syn::parse::<ImplItemFn>(input.clone()) {
        if let Err(error) = validate_signature(&method.sig) {
            return error.to_compile_error().into();
        }
        lower_block(&mut method.block, &runtime);
        return quote!(#method).into();
    }

    if let Ok(mut function) = syn::parse::<ItemFn>(input.clone()) {
        if let Err(error) = validate_signature(&function.sig) {
            return error.to_compile_error().into();
        }
        lower_block(&mut function.block, &runtime);
        return quote!(#function).into();
    }

    if let Ok(item_impl) = syn::parse::<ItemImpl>(input.clone()) {
        return syn::Error::new_spanned(
            item_impl.impl_token,
            "#[view_builder] applies to a function or method, not an entire impl block",
        )
        .to_compile_error()
        .into();
    }

    if let Ok(trait_method) = syn::parse::<TraitItemFn>(input.clone()) {
        return syn::Error::new_spanned(
            trait_method.sig.fn_token,
            "#[view_builder] requires a function or method body",
        )
        .to_compile_error()
        .into();
    }

    syn::Error::new(
        Span::call_site(),
        "#[view_builder] can only be applied to a function or method",
    )
    .to_compile_error()
    .into()
}

pub fn expand_macro(input: TokenStream) -> TokenStream {
    let runtime = match runtime_paths() {
        Ok(path) => path,
        Err(error) => return error.into(),
    };
    let mut builder_input = syn::parse_macro_input!(input as BuilderInput);
    lower_block(&mut builder_input.block, &runtime);
    let block = builder_input.block;
    quote!(#block).into()
}

/// The crate root exporting `OneOf2`..`OneOf8` that lowered branches name.
#[derive(Clone)]
struct Runtime {
    /// The crate root exporting `OneOf2`..`OneOf8`.
    root: TokenStream2,
}

fn runtime_paths() -> Result<Runtime, TokenStream2> {
    if matches!(
        current_package_name().as_deref(),
        Some("waterui" | "waterui-internal")
    ) {
        return Ok(Runtime {
            root: quote!(::waterui),
        });
    }

    if let Some(root) = dependency_path("waterui-core") {
        return Ok(Runtime { root });
    }

    if let Some(root) = dependency_path("waterui") {
        return Ok(Runtime { root });
    }

    Err(quote! {
        compile_error!(
            "`view!`/`#[view_builder]` require either the `waterui` crate or the `waterui-core` crate as a dependency."
        );
    })
}

/// The path an expansion names `package` by. Inside `package` itself that is
/// still its own name, never `crate`: a doctest, example or integration test
/// of the package is a crate of its own, and every crate these macros name
/// declares `extern crate self` under its name so the library resolves it too.
pub fn dependency_path(package: &str) -> Option<TokenStream2> {
    match crate_name(package) {
        Ok(FoundCrate::Itself) => {
            let ident = Ident::new(&package.replace('-', "_"), Span::call_site());
            Some(quote!(::#ident))
        }
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Some(quote!(::#ident))
        }
        Err(_) => None,
    }
}

fn current_package_name() -> Option<String> {
    var("CARGO_PKG_NAME").ok()
}

fn validate_signature(signature: &Signature) -> syn::Result<()> {
    if let Some(asyncness) = signature.asyncness {
        return Err(syn::Error::new_spanned(
            asyncness,
            "#[view_builder] does not support async functions",
        ));
    }

    let ReturnType::Type(_, ty) = &signature.output else {
        return Err(syn::Error::new_spanned(
            &signature.ident,
            "#[view_builder] requires an explicit `-> impl View` return type",
        ));
    };

    if !matches!(ty.as_ref(), Type::ImplTrait(_)) {
        return Err(syn::Error::new_spanned(
            ty,
            "#[view_builder] requires a `-> impl View` return type",
        ));
    }

    Ok(())
}

struct BuilderInput {
    block: Block,
}

impl Parse for BuilderInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(Self {
            block: Block {
                brace_token: Brace::default(),
                stmts: Block::parse_within(input)?,
            },
        })
    }
}

/// A wrapper the lowering applies to a branch's value.
#[derive(Clone, Copy)]
enum FinalWrap {
    /// `Some`, for the taken branch of an `if` with no `else`.
    Some,
    /// Variant `index` of `OneOf{arity}`.
    OneOf { arity: usize, index: usize },
}

/// The widest `OneOfN` the runtime defines; a choice among more branches
/// nests the remainder in its last variant.
const MAX_ONE_OF: usize = 8;

/// The wrappers, innermost first, that put branch `index` of `arity`
/// branches into the `OneOfN` the branches unify to.
fn one_of_wraps(index: usize, arity: usize) -> Vec<FinalWrap> {
    if arity <= MAX_ONE_OF {
        return vec![FinalWrap::OneOf { arity, index }];
    }
    let last = MAX_ONE_OF - 1;
    if index < last {
        return vec![FinalWrap::OneOf {
            arity: MAX_ONE_OF,
            index,
        }];
    }
    let mut wraps = one_of_wraps(index - last, arity - last);
    wraps.push(FinalWrap::OneOf {
        arity: MAX_ONE_OF,
        index: last,
    });
    wraps
}

fn lower_block(block: &mut Block, runtime: &Runtime) {
    lower_block_with(block, runtime, &[]);
}

/// Lowers `block`, applying `wraps` (innermost first) to its tail value.
fn lower_block_with(block: &mut Block, runtime: &Runtime, wraps: &[FinalWrap]) {
    let last_index = block.stmts.len().checked_sub(1);

    for (index, stmt) in block.stmts.iter_mut().enumerate() {
        let is_last = Some(index) == last_index;
        lower_stmt(stmt, runtime, is_last, wraps);
    }

    if (last_index.is_none()
        || !matches!(
            block.stmts.last(),
            Some(
                Stmt::Expr(_, None)
                    | Stmt::Macro(StmtMacro {
                        semi_token: None,
                        ..
                    })
            )
        ))
        && !wraps.is_empty()
    {
        block.stmts.push(Stmt::Expr(
            wrap_plain_expr(parse_quote!(()), runtime, wraps),
            None,
        ));
    }
}

fn lower_stmt(stmt: &mut Stmt, runtime: &Runtime, is_last: bool, wraps: &[FinalWrap]) {
    match stmt {
        Stmt::Local(local) => {
            if let Some(init) = &mut local.init {
                lower_returns_in_expr(&mut init.expr, runtime);
                if let Some((_, diverge)) = &mut init.diverge {
                    lower_returns_in_expr(diverge, runtime);
                }
            }
        }
        Stmt::Item(_) => {}
        Stmt::Expr(expr, semicolon) => {
            if is_last && semicolon.is_none() {
                let original = replace(expr, parse_quote!(()));
                *expr = lower_wrapped_expr(original, runtime, wraps);
            } else {
                lower_returns_in_expr(expr, runtime);
            }
        }
        Stmt::Macro(stmt_macro) => {
            if is_last && stmt_macro.semi_token.is_none() {
                let expr = Expr::Macro(ExprMacro {
                    attrs: stmt_macro.attrs.clone(),
                    mac: stmt_macro.mac.clone(),
                });
                *stmt = Stmt::Expr(lower_wrapped_expr(expr, runtime, wraps), None);
            }
        }
    }
}

/// Lowers `expr` and applies `wraps` (innermost first) to its value.
///
/// A plain block takes the wrappers on its own tail value instead of being
/// passed whole as their argument: `OneOf2::A({ view })` would carry the
/// braces the user wrote, at their span, into `unused_braces`.
fn lower_wrapped_expr(expr: Expr, runtime: &Runtime, wraps: &[FinalWrap]) -> Expr {
    match expr {
        Expr::Block(mut expr_block)
            if !wraps.is_empty() && expr_block.label.is_none() && expr_block.attrs.is_empty() =>
        {
            lower_block_with(&mut expr_block.block, runtime, wraps);
            Expr::Block(expr_block)
        }
        expr => wrap_plain_expr(lower_result_expr(expr, runtime), runtime, wraps),
    }
}

fn lower_result_expr(mut expr: Expr, runtime: &Runtime) -> Expr {
    lower_returns_in_expr(&mut expr, runtime);

    match expr {
        Expr::If(expr_if) => lower_if_expr(expr_if, runtime),
        Expr::Match(expr_match) => lower_match_expr(expr_match, runtime),
        Expr::Block(mut expr_block) => {
            lower_block(&mut expr_block.block, runtime);
            Expr::Block(expr_block)
        }
        Expr::Reference(mut expr_reference) => {
            expr_reference.expr = Box::new(lower_result_expr(*expr_reference.expr, runtime));
            Expr::Reference(expr_reference)
        }
        Expr::Paren(ExprParen {
            attrs,
            paren_token,
            expr,
        }) => Expr::Paren(ExprParen {
            attrs,
            paren_token,
            expr: Box::new(lower_result_expr(*expr, runtime)),
        }),
        Expr::Group(ExprGroup {
            attrs,
            group_token,
            expr,
        }) => Expr::Group(ExprGroup {
            attrs,
            group_token,
            expr: Box::new(lower_result_expr(*expr, runtime)),
        }),
        other => other,
    }
}

fn lower_if_expr(mut expr_if: ExprIf, runtime: &Runtime) -> Expr {
    let else_expr = if let Some((_, else_branch)) = expr_if.else_branch.take() {
        lower_block_with(&mut expr_if.then_branch, runtime, &one_of_wraps(0, 2));
        lower_wrapped_expr(*else_branch, runtime, &one_of_wraps(1, 2))
    } else {
        lower_block_with(&mut expr_if.then_branch, runtime, &[FinalWrap::Some]);
        parse_quote!(::core::option::Option::None)
    };
    // A lowered `else` block is already the branch's block; bracing it again
    // would nest the user's braces in ours.
    let else_branch = match else_expr {
        Expr::Block(_) => else_expr,
        other => parse_quote!({ #other }),
    };
    expr_if.else_branch = Some((Else::default(), Box::new(else_branch)));
    Expr::If(expr_if)
}

fn lower_match_expr(mut expr_match: ExprMatch, runtime: &Runtime) -> Expr {
    let arm_count = expr_match.arms.len();

    for (index, arm) in expr_match.arms.iter_mut().enumerate() {
        let original = replace(arm.body.as_mut(), parse_quote!(()));
        *arm.body = wrap_match_arm_body(original, runtime, index, arm_count);
    }

    Expr::Match(expr_match)
}

fn wrap_match_arm_body(expr: Expr, runtime: &Runtime, index: usize, arm_count: usize) -> Expr {
    // A single arm needs no choice; arm `i` of `n` is variant `i` of the
    // `OneOfN` every arm unifies to.
    let wraps = if arm_count == 1 {
        Vec::new()
    } else {
        one_of_wraps(index, arm_count)
    };

    lower_wrapped_expr(expr, runtime, &wraps)
}

/// Applies `wraps` to `expr`, innermost first.
fn wrap_plain_expr(expr: Expr, runtime: &Runtime, wraps: &[FinalWrap]) -> Expr {
    let root = &runtime.root;
    wraps.iter().fold(expr, |expr, wrapper| match *wrapper {
        FinalWrap::Some => parse_quote!(::core::option::Option::Some(#expr)),
        FinalWrap::OneOf { arity, index } => {
            let ty = Ident::new(&format!("OneOf{arity}"), Span::call_site());
            let variant = Ident::new(
                &char::from(b'A' + u8::try_from(index).expect("a OneOf variant index fits in u8"))
                    .to_string(),
                Span::call_site(),
            );
            parse_quote!(#root::#ty::#variant(#expr))
        }
    })
}

fn lower_returns_in_expr(expr: &mut Expr, runtime: &Runtime) {
    ReturnLowerer { runtime }.visit_expr_mut(expr);
}

struct ReturnLowerer<'a> {
    runtime: &'a Runtime,
}

impl VisitMut for ReturnLowerer<'_> {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Closure(_) | Expr::Async(_) => {}
            Expr::Return(ExprReturn {
                expr: Some(inner), ..
            }) => {
                let original = replace(inner.as_mut(), parse_quote!(()));
                *inner.as_mut() = lower_result_expr(original, self.runtime);
            }
            _ => visit_mut::visit_expr_mut(self, expr),
        }
    }
}
