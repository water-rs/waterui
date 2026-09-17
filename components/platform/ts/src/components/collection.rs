//! `<For>`: a dynamic set of views, reconciled by identity.
//!
//! A child position whose *membership* changes is a collection, never a
//! watched subtree: a `ForEach` over a reactive collection patches the items
//! that moved, arrived or left, while replacing the subtree would tear down
//! every item's state on every change. That is why this module exists at all,
//! and why `<For>` is not built on the reactive child slot.
//!
//! Identity is the whole mechanism, so it has to survive the boundary. A
//! JavaScript object's referential identity does not: an object crosses as
//! data, and Rust sees a copy. `by` is therefore required for anything but a
//! primitive item, and its absence is an error naming the fix rather than a
//! reconciliation that quietly treats two equal-looking rows as one.

use core::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use nami::collection::SignalCollection;
use nami::{Binding, Computed, Signal, SignalExt as _};
use suiteki::Str;
use waterui_core::id::Identifiable;
use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};
use waterui_core::{AnyView, Environment, Metadata, Retain, View};
use waterui_layout::stack::{HStack, VStack, ZStack};
use waterui_ts_engine::{JsError, JsFunction, JsValue};

use crate::bridge::{Bridge, WeakBridge};
use crate::components::control_flow::Branch;

/// The identity of one item, as `by` or the item itself answered it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKey {
    /// A boolean key.
    Bool(bool),
    /// An integer key, from a `bigint` or a whole `number`.
    Integer(i64),
    /// A fractional key, by its bit pattern, so two keys are the same key
    /// exactly when JavaScript says the numbers are.
    Number(u64),
    /// A string key.
    Text(Str),
}

impl Identifiable for ItemKey {
    type Id = Self;

    fn id(&self) -> Self::Id {
        self.clone()
    }
}

impl ItemKey {
    /// The key a value carries, or an error explaining what has to be given
    /// instead.
    fn of(value: &JsValue, keyed: bool) -> Result<Self, JsError> {
        match value {
            JsValue::Bool(value) => Ok(Self::Bool(*value)),
            JsValue::BigInt(_) => value
                .as_i64()
                .map(Self::Integer)
                .ok_or_else(|| JsError::conversion("a key past the range of a 64-bit integer")),
            JsValue::Number(number) => Ok(if number.fract() == 0.0 && number.is_finite() {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "the value has no fractional part and is finite, so the cast is exact"
                )]
                Self::Integer(*number as i64)
            } else {
                Self::Number(number.to_bits())
            }),
            JsValue::String(text) => Ok(Self::Text(Str::from(text.clone()))),
            other if keyed => Err(JsError::conversion(format!(
                "<For by={{…}}> answered {}, which cannot be a key: return a string, a number or \
                 a boolean",
                crate::error::kind_of(other)
            ))),
            other => Err(JsError::conversion(format!(
                "<For each={{…}}> was given items of {}, whose identity cannot cross into the \
                 native side: an object crosses as data, so two references Rust sees are two \
                 values. Give <For> a `by` that answers a stable key",
                crate::error::kind_of(other)
            ))),
        }
    }
}

/// One item: the value the render callback receives, its live index, and the
/// branch it built.
struct Item {
    /// The item as JavaScript last sent it.
    value: JsValue,
    /// The item's position, which a move updates in place.
    index: Binding<u32>,
    /// The accessor the render callback was handed for that position.
    accessor: JsValue,
    /// The branch this item is currently presented through, disposed when the
    /// item leaves or is realized again.
    branch: Option<Branch>,
}

/// Everything one `<For>` owns.
struct EachState {
    bridge: WeakBridge,
    render: JsFunction,
    by: Option<JsFunction>,
    items: RefCell<BTreeMap<ItemKey, Item>>,
}

impl EachState {
    /// The key of one item, through `by` when there is one.
    fn key(&self, bridge: &Bridge, value: &JsValue) -> Result<ItemKey, JsError> {
        match &self.by {
            Some(by) => ItemKey::of(&bridge.call(by, core::slice::from_ref(value))?, true),
            None => ItemKey::of(value, false),
        }
    }

    /// Takes one snapshot of the list: new keys gain an item, retained keys
    /// keep theirs and learn their new index, and departed keys lose theirs —
    /// which disposes the branch they were presented through.
    fn reconcile(&self, bridge: &Bridge, snapshot: &[JsValue]) -> Result<Vec<ItemKey>, JsError> {
        let mut keys = Vec::with_capacity(snapshot.len());
        for (position, value) in snapshot.iter().enumerate() {
            let key = self.key(bridge, value)?;
            let index = u32::try_from(position).map_err(|_| {
                JsError::conversion("a <For> list longer than a 32-bit index can address")
            })?;
            if let Some(item) = self.items.borrow_mut().get_mut(&key) {
                item.value = value.clone();
                item.index.set(index);
                keys.push(key);
                continue;
            }
            let binding = Binding::container(index);
            let accessor = bridge.export_computed(&binding.computed())?;
            self.items.borrow_mut().insert(
                key.clone(),
                Item {
                    value: value.clone(),
                    index: binding,
                    accessor,
                    branch: None,
                },
            );
            keys.push(key);
        }
        self.items.borrow_mut().retain(|key, _| keys.contains(key));
        Ok(keys)
    }

    /// Renders one item, replacing whatever branch it had.
    fn realize(&self, key: &ItemKey) -> Result<AnyView, JsError> {
        let bridge = self.bridge.upgrade().ok_or_else(|| {
            JsError::new(
                "Error",
                "a <For> item was realized after the TypeScript runtime was dropped",
            )
        })?;
        // The borrow is released before JavaScript runs: a render callback
        // reaches back into the host, and a live borrow would meet it there.
        let arguments = {
            let items = self.items.borrow();
            let item = items.get(key).ok_or_else(|| {
                JsError::new("Error", "a <For> item was realized after it left the list")
            })?;
            [item.value.clone(), item.accessor.clone()]
        };
        let (view, branch) = Branch::render(&bridge, &self.render, &arguments)?;
        if let Some(item) = self.items.borrow_mut().get_mut(key) {
            item.branch = Some(branch);
        }
        Ok(view)
    }
}

/// One item's view, rendered when the collection realizes that position.
struct ItemView {
    state: Rc<EachState>,
    key: ItemKey,
}

impl View for ItemView {
    fn body(self, _env: &Environment) -> impl View {
        self.state
            .realize(&self.key)
            .unwrap_or_else(|error| panic!("a <For> item could not be rendered: {error}"))
    }
}

/// The collection `<For>` evaluates to.
///
/// Standing on its own it is a vertical stack of its items, the layout
/// `vstack(ForEach…)` gives them. Written as a stack's only child it becomes
/// that stack's content instead, so `<HStack><For …/></HStack>` lays the same
/// items out in a row — which is the Rust model exactly: a stack is either
/// fixed or lazy, and a `<For>` beside other children is its own stack, as
/// `vstack((text, vstack(for_each)))` would be.
pub struct Collection {
    state: Rc<EachState>,
    keys: Binding<Vec<ItemKey>>,
    /// The watch that keeps `keys` in step with the list JavaScript holds.
    guard: Rc<dyn core::any::Any>,
}

impl Collection {
    /// Keeps the reconciliation watch alive for as long as the view is.
    fn retain(&self) -> Retain {
        Retain::new(Rc::clone(&self.guard))
    }

    /// The reactive key list a stack watches.
    fn keys(&self) -> SignalCollection<Binding<Vec<ItemKey>>> {
        SignalCollection::new(self.keys.clone())
    }

    /// The generator a stack's `for_each` takes, over the same state.
    fn items(&self) -> Box<dyn Fn(ItemKey) -> ItemView> {
        let state = Rc::clone(&self.state);
        Box::new(move |key| ItemView {
            state: Rc::clone(&state),
            key,
        })
    }
}

impl View for Collection {
    /// On its own, a collection is the vertical stack of its items.
    fn body(self, _env: &Environment) -> impl View {
        into_vstack(&self, HorizontalAlignment::Center, None)
    }
}

/// A vertical stack over a collection.
pub fn into_vstack(
    collection: &Collection,
    alignment: HorizontalAlignment,
    spacing: Option<Computed<f32>>,
) -> AnyView {
    let mut stack = VStack::for_each(collection.keys(), collection.items()).alignment(alignment);
    if let Some(spacing) = spacing {
        stack = stack.spacing(spacing);
    }
    AnyView::new(Metadata::new(stack, collection.retain()))
}

/// Builds the collection `<For each={…}>` evaluates to.
///
/// # Errors
///
/// Returns [`JsError`] when the list cannot be materialized, when an item has
/// no key, or when an index accessor cannot be exported.
pub fn each(
    bridge: &Bridge,
    items: &JsValue,
    render: &JsFunction,
    by: Option<&JsFunction>,
) -> Result<AnyView, JsError> {
    let source: Computed<Vec<JsValue>> = bridge.materialize_computed(items)?;
    let state = Rc::new(EachState {
        bridge: bridge.downgrade(),
        render: render.clone(),
        by: by.cloned(),
        items: RefCell::new(BTreeMap::new()),
    });

    let keys = Binding::container(state.reconcile(bridge, &source.get())?);
    let guard = source.watch({
        let state = Rc::clone(&state);
        let bridge = bridge.downgrade();
        let keys = keys.clone();
        move |context| {
            let Some(bridge) = bridge.upgrade() else {
                return;
            };
            let next = state
                .reconcile(&bridge, &context.into_value())
                .unwrap_or_else(|error| panic!("a <For> list could not be reconciled: {error}"));
            keys.set(next);
        }
    });

    // The list signal is retained beside its guard: a watch registration does
    // not keep what it watches alive, and this signal owns the JavaScript
    // subscription that announces membership changes.
    Ok(AnyView::new(Collection {
        state,
        keys,
        guard: Rc::new((guard, source)),
    }))
}

/// The stack axis a spliced collection belongs to, for the stacks that take
/// one as their only child.
pub enum Spliced {
    /// The stack's content is this collection.
    Lazy(Collection),
    /// The stack's content is a fixed list of views.
    Fixed(Vec<AnyView>),
}

/// Whether a stack's children are one collection, which becomes the stack's
/// own lazy content, or a fixed list.
pub fn splice(children: Vec<AnyView>) -> Spliced {
    let mut children = children;
    if children.len() == 1 {
        match children.remove(0).downcast::<Collection>() {
            Ok(collection) => return Spliced::Lazy(*collection),
            Err(view) => children.push(view),
        }
    }
    Spliced::Fixed(children)
}

/// A horizontal stack over a collection.
pub fn into_hstack(
    collection: &Collection,
    alignment: VerticalAlignment,
    spacing: Option<Computed<f32>>,
) -> AnyView {
    let mut stack = HStack::for_each(collection.keys(), collection.items()).alignment(alignment);
    if let Some(spacing) = spacing {
        stack = stack.spacing(spacing);
    }
    AnyView::new(Metadata::new(stack, collection.retain()))
}

/// A depth stack over a collection.
pub fn into_zstack(collection: &Collection, alignment: Alignment) -> AnyView {
    let stack = ZStack::for_each(collection.keys(), collection.items()).alignment(alignment);
    AnyView::new(Metadata::new(stack, collection.retain()))
}
