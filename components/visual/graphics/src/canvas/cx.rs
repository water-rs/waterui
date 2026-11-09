//! The drawing context a canvas's closure records into, and the retained
//! recording it produces.

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use alloc::vec::Vec;
use core::any::Any;
use core::cell::RefCell;
use core::fmt;
use core::hash::Hash;
use core::time::Duration;
use std::collections::HashMap;

use nami::watcher::{BoxWatcherGuard, Context};
use nami::{Computed, Signal, SignalExt as _};
use waterui_core::Environment;
use waterui_core::layout::{Affine2, Rect, Size};

use crate::canvas::cell::SignalCell;
use crate::canvas::drawing::Drawing;
use crate::canvas::geometry::Shape;
use crate::canvas::operand::Operand;
use crate::canvas::paint::{IntoPaint, Layer, Paint, Sampling, Shadow, StrokeStyle};
use crate::canvas::resource::{GlyphRun, ImageSource};
use crate::color::{Color, WorkingColor};
use crate::draw::ShapeData;
use crate::scene_view::SceneInvalidator;

/// What every context of one mounted canvas shares.
pub struct Shared {
    pub(crate) env: Environment,
    invalidator: RefCell<Option<SceneInvalidator>>,
    pub(crate) size: SignalCell<Size>,
    pub(crate) clock: SignalCell<Duration>,
}

impl Shared {
    pub(crate) fn new(env: Environment) -> Rc<Self> {
        Rc::new(Self {
            env,
            invalidator: RefCell::new(None),
            size: SignalCell::new(Size::zero()),
            clock: SignalCell::new(Duration::ZERO),
        })
    }

    pub(crate) fn set_invalidator(&self, invalidator: Option<SceneInvalidator>) {
        *self.invalidator.borrow_mut() = invalidator;
    }

    /// Asks the host to record the canvas again: its structure changed.
    pub(crate) fn invalidate(&self) {
        let invalidator = self.invalidator.borrow().clone();
        if let Some(invalidator) = invalidator {
            invalidator();
        }
    }
}

/// One retained command, holding its operands as signals.
pub enum Node {
    Fill {
        shape: Computed<ShapeData>,
        paint: Computed<Paint>,
    },
    Stroke {
        shape: Computed<ShapeData>,
        style: Computed<StrokeStyle>,
        paint: Computed<Paint>,
    },
    Shadow {
        shape: Computed<ShapeData>,
        shadow: Computed<Shadow>,
    },
    Image {
        image: Computed<ImageSource>,
        dst: Computed<Rect>,
        sampling: Sampling,
    },
    Glyphs {
        run: Computed<GlyphRun>,
        paint: Computed<Paint>,
    },
    Draw {
        drawing: Drawing,
        transform: Computed<Affine2>,
    },
    Clip {
        shape: Computed<ShapeData>,
        body: Recording,
    },
    Transform {
        transform: Computed<Affine2>,
        body: Recording,
    },
    Layer {
        layer: Computed<Layer>,
        body: Recording,
    },
    /// A region a signal re-records ([`Cx::scope`]).
    Scope(Rc<RefCell<Recording>>),
    /// A keyed collection of regions ([`Cx::each`]).
    Each(Rc<RefCell<dyn Regions>>),
}

/// The ordered sub-recordings of a [`Cx::each`].
pub trait Regions {
    /// Calls `f` with each sub-recording, in item order.
    fn for_each(&self, f: &mut dyn FnMut(&Recording));
}

/// What one closure scope recorded: its commands, and the values it keeps
/// alive for as long as it is part of the canvas.
#[derive(Default)]
pub struct Recording {
    pub(crate) nodes: Vec<Node>,
    kept: Vec<Box<dyn Any>>,
}

impl fmt::Debug for Recording {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recording")
            .field("commands", &self.nodes.len())
            .field("kept", &self.kept.len())
            .finish()
    }
}

/// The drawing context a [`Canvas`](crate::canvas::Canvas) closure records
/// into.
///
/// Every operand takes a value or a signal; a signal stays bound, and its
/// change updates that operand of the recording in place. State is scoped by
/// closures — [`clip`](Self::clip), [`transform`](Self::transform),
/// [`layer`](Self::layer) — so there is nothing to save or restore.
///
/// The closure runs once. Structure that depends on a value re-records
/// locally: [`each`](Self::each) keeps one sub-recording per item key, and
/// [`scope`](Self::scope) re-records one region when its signal changes.
pub struct Cx {
    shared: Rc<Shared>,
    recording: Recording,
}

impl fmt::Debug for Cx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cx")
            .field("recording", &self.recording)
            .finish_non_exhaustive()
    }
}

/// Lowers a shape operand to the recorded shape data it draws.
fn shape_signal<S: Shape>(shape: impl Into<Operand<S>>) -> Computed<ShapeData> {
    shape
        .into()
        .into_computed()
        .map(|shape: S| shape.lower())
        .computed()
}

impl Cx {
    pub(crate) const fn new(shared: Rc<Shared>) -> Self {
        Self {
            shared,
            recording: Recording {
                nodes: Vec::new(),
                kept: Vec::new(),
            },
        }
    }

    pub(crate) fn finish(self) -> Recording {
        self.recording
    }

    /// Records `body` into a recording of its own.
    fn sub(&self, body: impl FnOnce(&mut Self)) -> Recording {
        let mut cx = Self::new(Rc::clone(&self.shared));
        body(&mut cx);
        cx.recording
    }

    fn paint<P: IntoPaint>(&self, paint: impl Into<Operand<P>>) -> Computed<Paint> {
        P::resolve(paint.into().into_computed(), &self.shared.env)
    }

    /// The canvas's box, in points: a signal that follows layout.
    #[must_use]
    pub fn size(&self) -> Computed<Size> {
        Computed::new(self.shared.size.clone())
    }

    /// The time since the canvas was mounted.
    ///
    /// It advances once per presented frame while something observes it — a
    /// live operand derived from it, or a [`scope`](Self::scope) over it —
    /// and stops when nothing does, so a canvas that does not animate
    /// presents no frames.
    #[must_use]
    pub fn time(&self) -> Computed<Duration> {
        Computed::new(self.shared.clock.clone())
    }

    /// `color` resolved in the canvas's environment: a theme colour as a
    /// signal of working-space colours, for building gradients and other
    /// paints from theme colours.
    #[must_use]
    pub fn resolve(&self, color: impl Into<Color>) -> Computed<WorkingColor> {
        color.into().resolve(&self.shared.env)
    }

    /// Ties `value` to this recording's lifetime: it is dropped when the
    /// recording is — when the canvas unmounts, or when the enclosing
    /// [`scope`](Self::scope) re-records or [`each`](Self::each) item is
    /// removed. A hardware source opened for the drawing closes then.
    pub fn keep(&mut self, value: impl Any) {
        self.recording.kept.push(Box::new(value));
    }

    /// Fills `shape` with `paint`.
    pub fn fill<S: Shape, P: IntoPaint>(
        &mut self,
        shape: impl Into<Operand<S>>,
        paint: impl Into<Operand<P>>,
    ) {
        let node = Node::Fill {
            shape: shape_signal(shape),
            paint: self.paint(paint),
        };
        self.recording.nodes.push(node);
    }

    /// Strokes `shape`'s outline in `style` with `paint`.
    pub fn stroke<S: Shape, P: IntoPaint>(
        &mut self,
        shape: impl Into<Operand<S>>,
        style: impl Into<Operand<StrokeStyle>>,
        paint: impl Into<Operand<P>>,
    ) {
        let node = Node::Stroke {
            shape: shape_signal(shape),
            style: style.into().into_computed(),
            paint: self.paint(paint),
        };
        self.recording.nodes.push(node);
    }

    /// Casts `shadow` from `shape`.
    pub fn shadow<S: Shape>(
        &mut self,
        shape: impl Into<Operand<S>>,
        shadow: impl Into<Operand<Shadow>>,
    ) {
        self.recording.nodes.push(Node::Shadow {
            shape: shape_signal(shape),
            shadow: shadow.into().into_computed(),
        });
    }

    /// Draws `image` scaled into `rect`.
    ///
    /// A signal of images swaps the picture; what the target made of each
    /// image is its [`status`](ImageSource::status).
    pub fn image(
        &mut self,
        image: impl Into<Operand<ImageSource>>,
        rect: impl Into<Operand<Rect>>,
        sampling: Sampling,
    ) {
        self.recording.nodes.push(Node::Image {
            image: image.into().into_computed(),
            dst: rect.into().into_computed(),
            sampling,
        });
    }

    /// Draws `run`'s glyphs, exactly as positioned, with `paint`.
    pub fn glyphs<P: IntoPaint>(
        &mut self,
        run: impl Into<Operand<GlyphRun>>,
        paint: impl Into<Operand<P>>,
    ) {
        let node = Node::Glyphs {
            run: run.into().into_computed(),
            paint: self.paint(paint),
        };
        self.recording.nodes.push(node);
    }

    /// Draws `drawing` under `transform`.
    pub fn draw(&mut self, drawing: &Drawing, transform: impl Into<Operand<Affine2>>) {
        self.recording.nodes.push(Node::Draw {
            drawing: drawing.clone(),
            transform: transform.into().into_computed(),
        });
    }

    /// Draws `body` clipped to `shape`.
    pub fn clip<S: Shape>(&mut self, shape: impl Into<Operand<S>>, body: impl FnOnce(&mut Self)) {
        let shape = shape_signal(shape);
        let body = self.sub(body);
        self.recording.nodes.push(Node::Clip { shape, body });
    }

    /// Draws `body` under `transform`.
    pub fn transform(
        &mut self,
        transform: impl Into<Operand<Affine2>>,
        body: impl FnOnce(&mut Self),
    ) {
        let transform = transform.into().into_computed();
        let body = self.sub(body);
        self.recording
            .nodes
            .push(Node::Transform { transform, body });
    }

    /// Draws `body` isolated in `layer`, which composites it as one image
    /// with the layer's opacity and blend mode.
    pub fn layer(&mut self, layer: impl Into<Operand<Layer>>, body: impl FnOnce(&mut Self)) {
        let layer = layer.into().into_computed();
        let body = self.sub(body);
        self.recording.nodes.push(Node::Layer { layer, body });
    }

    /// Records `body` for the current value of `signal`, and records it again
    /// — that region only — whenever the signal changes.
    ///
    /// The tool for content whose shape, not just its operands, depends on a
    /// value: a formula re-typeset from its source. Everything else in the
    /// canvas stays as recorded.
    pub fn scope<T: 'static>(
        &mut self,
        signal: impl Into<Operand<T>>,
        body: impl Fn(&mut Self, &T) + 'static,
    ) {
        let signal = signal.into().into_computed();
        let region = Rc::new(RefCell::new(Recording::default()));
        let body = Rc::new(body);
        let shared = Rc::downgrade(&self.shared);
        let record = {
            let region = Rc::downgrade(&region);
            let body = Rc::clone(&body);
            move |value: &T| {
                let (Some(shared), Some(region)) = (shared.upgrade(), region.upgrade()) else {
                    return;
                };
                let mut cx = Self::new(Rc::clone(&shared));
                body(&mut cx, value);
                // The replaced recording drops after the borrow ends: what it
                // kept may observe the canvas.
                let replaced = core::mem::replace(&mut *region.borrow_mut(), cx.recording);
                drop(replaced);
                shared.invalidate();
            }
        };
        {
            let mut cx = Self::new(Rc::clone(&self.shared));
            signal.with_ref(|value| body(&mut cx, value));
            *region.borrow_mut() = cx.recording;
        }
        let guard: BoxWatcherGuard = signal.watch(move |context| record(context.value()));
        self.recording.kept.push(Box::new(guard));
        self.recording.nodes.push(Node::Scope(region));
    }

    /// Records `body` once per item of `items`, keyed by `key`.
    ///
    /// When the collection changes, an item with a new key records, an item
    /// whose key is gone drops, and moved items reorder — without recording
    /// them again. An item whose value changes under the same key updates
    /// through the `Computed` its body was handed: the operands bound to it
    /// update in place, animated when the collection changed under an
    /// animation.
    ///
    /// # Panics
    ///
    /// When two items share a key.
    pub fn each<T, K>(
        &mut self,
        items: impl Into<Operand<Vec<T>>>,
        key: impl Fn(&T) -> K + 'static,
        body: impl Fn(&mut Self, Computed<T>) + 'static,
    ) where
        T: Clone + PartialEq + 'static,
        K: Eq + Hash + Clone + fmt::Debug + 'static,
    {
        let items = items.into().into_computed();
        let list = Rc::new(RefCell::new(Keyed::<T, K> {
            entries: Vec::new(),
        }));
        let key: Rc<dyn Fn(&T) -> K> = Rc::new(key);
        let body: Rc<ItemBody<T>> = Rc::new(body);
        list.borrow_mut().update(
            &Context::from(items.snapshot()),
            key.as_ref(),
            body.as_ref(),
            &self.shared,
        );
        let guard: BoxWatcherGuard = {
            let list = Rc::downgrade(&list);
            let shared: Weak<Shared> = Rc::downgrade(&self.shared);
            items.watch(move |change| {
                let (Some(list), Some(shared)) = (list.upgrade(), shared.upgrade()) else {
                    return;
                };
                let (structural, dropped) =
                    list.borrow_mut()
                        .update(&change, key.as_ref(), body.as_ref(), &shared);
                // Removed items drop outside the borrow: what they kept may
                // observe the canvas.
                drop(dropped);
                if structural {
                    shared.invalidate();
                }
            })
        };
        self.recording.kept.push(Box::new(guard));
        self.recording.nodes.push(Node::Each(list));
    }
}

/// What [`Cx::each`] records for one item.
type ItemBody<T> = dyn Fn(&mut Cx, Computed<T>);

/// One item of a [`Cx::each`]: its key, the signal its body observes, and
/// what the body recorded.
struct Entry<T: 'static, K> {
    key: K,
    item: SignalCell<T>,
    recording: Recording,
}

struct Keyed<T: 'static, K> {
    entries: Vec<Entry<T, K>>,
}

impl<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone + fmt::Debug + 'static> Keyed<T, K> {
    /// Brings the entries in line with `change`'s items. Returns whether
    /// the structure changed — an insert, a removal or a move — and the
    /// entries that left, for the caller to drop.
    fn update(
        &mut self,
        change: &Context<Vec<T>>,
        key: &dyn Fn(&T) -> K,
        body: &dyn Fn(&mut Cx, Computed<T>),
        shared: &Rc<Shared>,
    ) -> (bool, Vec<Entry<T, K>>) {
        let items = change.value();
        let before: Vec<K> = self.entries.iter().map(|entry| entry.key.clone()).collect();
        let mut old: HashMap<K, Entry<T, K>> = self
            .entries
            .drain(..)
            .map(|entry| (entry.key.clone(), entry))
            .collect();
        let mut entries = Vec::with_capacity(items.len());
        let mut updates = Vec::new();
        for item in items {
            let key = key(item);
            assert!(
                !entries.iter().any(|entry: &Entry<T, K>| entry.key == key),
                "Cx::each: two items share the key {key:?}"
            );
            if let Some(entry) = old.remove(&key) {
                if entry.item.snapshot() != *item {
                    updates.push((entry.item.clone(), item.clone()));
                }
                entries.push(entry);
            } else {
                let cell = SignalCell::new(item.clone());
                let mut cx = Cx::new(Rc::clone(shared));
                body(&mut cx, Computed::new(cell.clone()));
                entries.push(Entry {
                    key,
                    item: cell,
                    recording: cx.recording,
                });
            }
        }
        let structural = entries.len() != before.len()
            || entries
                .iter()
                .zip(&before)
                .any(|(entry, key)| entry.key != *key);
        self.entries = entries;
        // The changed items update after the entries are in place: their
        // watchers run against the new structure.
        let metadata = change.metadata().clone();
        for (cell, value) in updates {
            cell.set(&Context::new(value, metadata.clone()));
        }
        (structural, old.into_values().collect())
    }
}

impl<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone + fmt::Debug + 'static> Regions
    for Keyed<T, K>
{
    fn for_each(&self, f: &mut dyn FnMut(&Recording)) {
        for entry in &self.entries {
            f(&entry.recording);
        }
    }
}
