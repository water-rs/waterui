use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::parse::{Nothing, Parse, ParseStream};
use syn::visit_mut::{self, VisitMut};
use syn::{
    Block, Expr, ExprGroup, ExprIf, ExprLet, ExprMacro, ExprMatch, ExprParen, ExprReturn, Ident,
    ImplItemFn, ItemFn, ItemImpl, ReturnType, Signature, Stmt, StmtMacro, TraitItemFn, Type,
    parse_quote,
};

pub fn expand_attribute(args: TokenStream, input: TokenStream) -> TokenStream {
    if let Err(error) = syn::parse::<Nothing>(args) {
        return error.to_compile_error().into();
    }
    let watch_path = match runtime_watch_path() {
        Ok(path) => path,
        Err(error) => return error.into(),
    };

    if let Ok(mut method) = syn::parse::<ImplItemFn>(input.clone()) {
        if let Err(error) = validate_signature(&method.sig) {
            return error.to_compile_error().into();
        }
        lower_block(&mut method.block, &watch_path);
        return quote!(#method).into();
    }

    if let Ok(mut function) = syn::parse::<ItemFn>(input.clone()) {
        if let Err(error) = validate_signature(&function.sig) {
            return error.to_compile_error().into();
        }
        lower_block(&mut function.block, &watch_path);
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
    let watch_path = match runtime_watch_path() {
        Ok(path) => path,
        Err(error) => return error.into(),
    };
    let mut builder_input = syn::parse_macro_input!(input as BuilderInput);
    lower_block(&mut builder_input.block, &watch_path);
    let block = builder_input.block;
    quote!(#block).into()
}

fn runtime_watch_path() -> Result<TokenStream2, TokenStream2> {
    if matches!(
        current_package_name().as_deref(),
        Some("waterui") | Some("waterui-internal")
    ) {
        return Ok(quote!(crate::component::dynamic::watch));
    }

    if let Some(path) = dependency_path("waterui-core") {
        return Ok(quote!(#path::dynamic::watch));
    }

    if let Some(path) = dependency_path("waterui") {
        return Ok(quote!(#path::component::dynamic::watch));
    }

    Err(quote! {
        compile_error!(
            "`view!`/`#[view_builder]` require either the `waterui` crate or the `waterui-core` crate as a dependency."
        );
    })
}

fn dependency_path(package: &str) -> Option<TokenStream2> {
    match crate_name(package) {
        Ok(FoundCrate::Itself) => Some(quote!(crate)),
        Ok(FoundCrate::Name(name)) => {
            let ident = Ident::new(&name, Span::call_site());
            Some(quote!(::#ident))
        }
        Err(_) => None,
    }
}

fn current_package_name() -> Option<String> {
    std::env::var("CARGO_PKG_NAME").ok()
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
                brace_token: Default::default(),
                stmts: Block::parse_within(input)?,
            },
        })
    }
}

#[derive(Clone, Copy)]
enum FinalWrap {
    Some,
    Ok,
    Err,
}

fn lower_block(block: &mut Block, watch_path: &TokenStream2) {
    lower_block_with(block, watch_path, None);
}

fn lower_block_with(block: &mut Block, watch_path: &TokenStream2, final_wrap: Option<FinalWrap>) {
    let last_index = block.stmts.len().checked_sub(1);

    for (index, stmt) in block.stmts.iter_mut().enumerate() {
        let is_last = Some(index) == last_index;
        lower_stmt(stmt, watch_path, is_last, final_wrap);
    }

    if last_index.is_none()
        || !matches!(
            block.stmts.last(),
            Some(Stmt::Expr(_, None))
                | Some(Stmt::Macro(StmtMacro {
                    semi_token: None,
                    ..
                }))
        )
    {
        if let Some(wrapper) = final_wrap {
            block
                .stmts
                .push(Stmt::Expr(wrap_plain_expr(parse_quote!(()), wrapper), None));
        }
    }
}

fn lower_stmt(
    stmt: &mut Stmt,
    watch_path: &TokenStream2,
    is_last: bool,
    final_wrap: Option<FinalWrap>,
) {
    match stmt {
        Stmt::Local(local) => {
            if let Some(init) = &mut local.init {
                lower_returns_in_expr(&mut init.expr, watch_path);
                if let Some((_, diverge)) = &mut init.diverge {
                    lower_returns_in_expr(diverge, watch_path);
                }
            }
        }
        Stmt::Item(_) => {}
        Stmt::Expr(expr, semicolon) => {
            if is_last && semicolon.is_none() {
                let original = core::mem::replace(expr, parse_quote!(()));
                *expr = lower_final_expr(original, watch_path, final_wrap);
            } else {
                lower_returns_in_expr(expr, watch_path);
            }
        }
        Stmt::Macro(stmt_macro) => {
            if is_last && stmt_macro.semi_token.is_none() {
                let expr = Expr::Macro(ExprMacro {
                    attrs: stmt_macro.attrs.clone(),
                    mac: stmt_macro.mac.clone(),
                });
                *stmt = Stmt::Expr(lower_final_expr(expr, watch_path, final_wrap), None);
            }
        }
    }
}

fn lower_final_expr(expr: Expr, watch_path: &TokenStream2, final_wrap: Option<FinalWrap>) -> Expr {
    let lowered = lower_result_expr(expr, watch_path);
    final_wrap.map_or(lowered.clone(), |wrapper| wrap_plain_expr(lowered, wrapper))
}

fn lower_result_expr(mut expr: Expr, watch_path: &TokenStream2) -> Expr {
    lower_returns_in_expr(&mut expr, watch_path);

    match expr {
        Expr::If(expr_if) => lower_if_expr(expr_if, watch_path),
        Expr::Match(expr_match) => lower_match_expr(expr_match, watch_path),
        Expr::Block(mut expr_block) => {
            lower_block(&mut expr_block.block, watch_path);
            Expr::Block(expr_block)
        }
        Expr::Reference(mut expr_reference) => {
            expr_reference.expr = Box::new(lower_result_expr(*expr_reference.expr, watch_path));
            Expr::Reference(expr_reference)
        }
        Expr::Paren(ExprParen {
            attrs,
            paren_token,
            expr,
        }) => Expr::Paren(ExprParen {
            attrs,
            paren_token,
            expr: Box::new(lower_result_expr(*expr, watch_path)),
        }),
        Expr::Group(ExprGroup {
            attrs,
            group_token,
            expr,
        }) => Expr::Group(ExprGroup {
            attrs,
            group_token,
            expr: Box::new(lower_result_expr(*expr, watch_path)),
        }),
        other => other,
    }
}

fn lower_if_expr(mut expr_if: ExprIf, watch_path: &TokenStream2) -> Expr {
    let attrs = expr_if.attrs.clone();
    let condition_ident = Ident::new("__waterui_condition", Span::call_site());
    let signal_ident = Ident::new("__waterui_condition_signal", Span::call_site());

    let else_expr = match expr_if.else_branch.take() {
        Some((else_token, else_branch)) => {
            lower_block_with(&mut expr_if.then_branch, watch_path, Some(FinalWrap::Ok));
            let _ = else_token;
            wrap_lowered_expr(*else_branch, watch_path, FinalWrap::Err)
        }
        None => {
            lower_block_with(&mut expr_if.then_branch, watch_path, Some(FinalWrap::Some));
            parse_quote!(::core::option::Option::None)
        }
    };
    let then_branch = expr_if.then_branch;

    match *expr_if.cond {
        Expr::Let(ExprLet { pat, expr, .. }) => syn::parse2(quote! {{
            let #signal_ident = #expr;
            #watch_path(#signal_ident.clone(), move |#condition_ident| {
                #(#attrs)*
                if let #pat = #condition_ident #then_branch else { #else_expr }
            })
        }})
        .expect("view_builder lowered if-let expression must stay valid Rust"),
        condition => syn::parse2(quote! {{
            let #signal_ident = #condition;
            #watch_path(#signal_ident.clone(), move |#condition_ident| {
                #(#attrs)*
                if #condition_ident #then_branch else { #else_expr }
            })
        }})
        .expect("view_builder lowered if expression must stay valid Rust"),
    }
}

fn lower_match_expr(mut expr_match: ExprMatch, watch_path: &TokenStream2) -> Expr {
    let attrs = expr_match.attrs.clone();
    let scrutinee = *expr_match.expr;
    let scrutinee_ident = Ident::new("__waterui_match_value", Span::call_site());
    let signal_ident = Ident::new("__waterui_match_signal", Span::call_site());
    let arm_count = expr_match.arms.len();

    for (index, arm) in expr_match.arms.iter_mut().enumerate() {
        let original = (*arm.body).clone();
        arm.body = Box::new(wrap_match_arm_body(original, watch_path, index, arm_count));
    }

    let arms = expr_match.arms;
    syn::parse2(quote! {{
        let #signal_ident = #scrutinee;
        #watch_path(#signal_ident.clone(), move |#scrutinee_ident| {
            #(#attrs)*
            match #scrutinee_ident {
                #(#arms)*
            }
        })
    }})
    .expect("view_builder lowered match expression must stay valid Rust")
}

fn wrap_match_arm_body(
    expr: Expr,
    watch_path: &TokenStream2,
    index: usize,
    arm_count: usize,
) -> Expr {
    let mut wrapped = lower_result_expr(expr, watch_path);

    if index + 1 != arm_count {
        wrapped = wrap_plain_expr(wrapped, FinalWrap::Ok);
    }

    for _ in 0..index {
        wrapped = wrap_plain_expr(wrapped, FinalWrap::Err);
    }

    wrapped
}

fn wrap_lowered_expr(expr: Expr, watch_path: &TokenStream2, wrapper: FinalWrap) -> Expr {
    wrap_plain_expr(lower_result_expr(expr, watch_path), wrapper)
}

fn wrap_plain_expr(expr: Expr, wrapper: FinalWrap) -> Expr {
    match wrapper {
        FinalWrap::Some => parse_quote!(::core::option::Option::Some(#expr)),
        FinalWrap::Ok => parse_quote!(::core::result::Result::Ok(#expr)),
        FinalWrap::Err => parse_quote!(::core::result::Result::Err(#expr)),
    }
}

fn lower_returns_in_expr(expr: &mut Expr, watch_path: &TokenStream2) {
    ReturnLowerer {
        watch_path: watch_path.clone(),
    }
    .visit_expr_mut(expr);
}

struct ReturnLowerer {
    watch_path: TokenStream2,
}

impl VisitMut for ReturnLowerer {
    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Closure(_) | Expr::Async(_) => {}
            Expr::Return(ExprReturn {
                expr: Some(inner), ..
            }) => {
                let original = core::mem::replace(inner.as_mut(), parse_quote!(()));
                *inner.as_mut() = lower_result_expr(original, &self.watch_path);
            }
            _ => visit_mut::visit_expr_mut(self, expr),
        }
    }
}
