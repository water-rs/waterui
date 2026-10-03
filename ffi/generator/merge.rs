//! Order-stable union merge of expanded Rust sources.
//!
//! Each expansion is the `waterui-ffi` crate resolved for one target triple:
//! `#[cfg]` is already evaluated, so items that exist only on another triple
//! are absent from that file. The union of all expansions is the full surface
//! the checked-in header must describe. The merge keeps the first expansion's
//! items verbatim and in order — the list starts with `aarch64-apple-darwin`,
//! so a union with no new exports is byte-identical to a macOS expansion — and
//! appends items found only in later expansions after them. Duplicate leaves
//! keep the first copy: two expansions can legitimately define the same
//! `extern "C"` name with different signatures under different `cfg`s, and C
//! has no overloads, so the header can carry only one declaration.

use quote::ToTokens;
use syn::{Field, Fields, ForeignItem, ImplItem, Item, TraitItem, Variant};

/// Merges `src` into `dst`: each item whose key is absent is appended in
/// `src`'s order; each item whose key is present merges its children into the
/// existing item and keeps the existing one otherwise.
pub fn merge_items(dst: &mut Vec<Item>, src: Vec<Item>) {
    merge_list(dst, src, item_key);
}

fn merge_list<T: MergeChildren>(dst: &mut Vec<T>, src: Vec<T>, key: fn(&T) -> String) {
    for item in src {
        match dst
            .iter_mut()
            .position(|existing| key(existing) == key(&item))
        {
            Some(index) => dst[index].merge_children(item),
            None => dst.push(item),
        }
    }
}

/// An element whose children can be unioned with another element of the same
/// key; the default merges nothing and keeps the first copy.
trait MergeChildren: Sized {
    fn merge_children(&mut self, _other: Self) {}
}

impl MergeChildren for Item {
    fn merge_children(&mut self, other: Self) {
        match (self, other) {
            (Self::Mod(a), Self::Mod(b)) => {
                if let (Some((_, dst)), Some((_, src))) = (&mut a.content, b.content) {
                    merge_items(dst, src);
                }
            }
            (Self::Impl(a), Self::Impl(b)) => {
                merge_list(&mut a.items, b.items, impl_item_key);
            }
            (Self::Trait(a), Self::Trait(b)) => {
                merge_list(&mut a.items, b.items, trait_item_key);
            }
            (Self::ForeignMod(a), Self::ForeignMod(b)) => {
                merge_list(&mut a.items, b.items, foreign_item_key);
            }
            (Self::Enum(a), Self::Enum(b)) => {
                let mut dst = a.variants.iter().cloned().collect::<Vec<_>>();
                merge_list(&mut dst, b.variants.into_iter().collect(), variant_key);
                a.variants = dst.into_iter().collect();
            }
            (Self::Struct(a), Self::Struct(b)) => merge_fields(&mut a.fields, b.fields),
            (Self::Union(a), Self::Union(b)) => {
                let mut dst = a.fields.named.iter().cloned().collect::<Vec<_>>();
                merge_list(&mut dst, b.fields.named.into_iter().collect(), field_key);
                a.fields.named = dst.into_iter().collect();
            }
            _ => {}
        }
    }
}

impl MergeChildren for ImplItem {}
impl MergeChildren for TraitItem {}
impl MergeChildren for ForeignItem {}
impl MergeChildren for Field {}
impl MergeChildren for Variant {}

fn merge_fields(dst: &mut Fields, src: Fields) {
    let (Fields::Named(dst_fields), Fields::Named(src_fields)) = (dst, src) else {
        return;
    };
    let mut dst_named = dst_fields.named.iter().cloned().collect::<Vec<_>>();
    merge_list(
        &mut dst_named,
        src_fields.named.into_iter().collect(),
        field_key,
    );
    dst_fields.named = dst_named.into_iter().collect();
}

/// Whitespace-free rendering of a syntax node, so two items that differ only
/// in formatting share a key.
fn text(node: &impl ToTokens) -> String {
    node.to_token_stream().to_string()
}

fn item_key(item: &Item) -> String {
    match item {
        Item::Const(item) => format!("const:{}", item.ident),
        Item::Enum(item) => format!("enum:{}", item.ident),
        Item::ExternCrate(item) => format!("extern-crate:{}", item.ident),
        Item::Fn(item) => format!("fn:{}", item.sig.ident),
        Item::ForeignMod(item) => format!("foreign-mod:{}", text(&item.abi)),
        Item::Impl(item) => format!(
            "impl:{}:{}",
            item.trait_
                .as_ref()
                .map(|(_, path, _)| text(path))
                .unwrap_or_default(),
            text(&item.self_ty)
        ),
        Item::Macro(item) => item.ident.as_ref().map_or_else(
            || format!("macro:{}", text(&item.mac)),
            |ident| format!("macro:{ident}"),
        ),
        Item::Mod(item) => format!("mod:{}", item.ident),
        Item::Static(item) => format!("static:{}", item.ident),
        Item::Struct(item) => format!("struct:{}", item.ident),
        Item::Trait(item) => format!("trait:{}", item.ident),
        Item::TraitAlias(item) => format!("trait-alias:{}", item.ident),
        Item::Type(item) => format!("type:{}", item.ident),
        Item::Union(item) => format!("union:{}", item.ident),
        Item::Use(item) => format!("use:{}", text(&item.tree)),
        Item::Verbatim(tokens) => format!("verbatim:{}", text(tokens)),
        _ => format!("other:{}", text(item)),
    }
}

fn impl_item_key(item: &ImplItem) -> String {
    match item {
        ImplItem::Const(item) => format!("const:{}", item.ident),
        ImplItem::Fn(item) => format!("fn:{}", item.sig.ident),
        ImplItem::Type(item) => format!("type:{}", item.ident),
        ImplItem::Macro(item) => format!("macro:{}", text(&item.mac)),
        ImplItem::Verbatim(tokens) => format!("verbatim:{}", text(tokens)),
        _ => format!("other:{}", text(item)),
    }
}

fn trait_item_key(item: &TraitItem) -> String {
    match item {
        TraitItem::Const(item) => format!("const:{}", item.ident),
        TraitItem::Fn(item) => format!("fn:{}", item.sig.ident),
        TraitItem::Type(item) => format!("type:{}", item.ident),
        TraitItem::Macro(item) => format!("macro:{}", text(&item.mac)),
        TraitItem::Verbatim(tokens) => format!("verbatim:{}", text(tokens)),
        _ => format!("other:{}", text(item)),
    }
}

fn foreign_item_key(item: &ForeignItem) -> String {
    match item {
        ForeignItem::Fn(item) => format!("fn:{}", item.sig.ident),
        ForeignItem::Static(item) => format!("static:{}", item.ident),
        ForeignItem::Type(item) => format!("type:{}", item.ident),
        ForeignItem::Macro(item) => format!("macro:{}", text(&item.mac)),
        ForeignItem::Verbatim(tokens) => format!("verbatim:{}", text(tokens)),
        _ => format!("other:{}", text(item)),
    }
}

fn field_key(field: &Field) -> String {
    field.ident.as_ref().map_or_else(
        || format!("field:{}", text(&field.ty)),
        |ident| format!("field:{ident}"),
    )
}

fn variant_key(variant: &Variant) -> String {
    format!("variant:{}", variant.ident)
}
