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
use waterui_core::layout::{Affine2, Rect, Size};
use waterui_core::{Environment, contract_violation};
use waterui_text::styled::StyledStr;

use crate::canvas::cell::SignalCell;
use crate::canvas::drawing::Drawing;
use crate::canvas::geometry::Shape;
use crate::canvas::operand::Operand;
use crate::canvas::paint::{IntoPaint, Layer, Paint, Sampling, Shadow, StrokeStyle};
use crate::canvas::resource::{GlyphRun, ImageSource};
use crate::canvas::text::TextPlacement;
use crate::color::{Color, WorkingColor};
use crate::draw::{Recorder, Region, ShapeData};
use crate::scene_view::SceneInvalidator;

/// What every context of one mounted canvas shares.
pub struct Shared {
    pub(crate) env: Environment,
    invalidator: RefCell<Option<SceneInvalidator>>,
    pub(crate) size: SignalCell<Size>,
    pub(crate) clock: SignalCell<Duration>,
    /// The clocks [`Cx::time_until`] handed out, each with the time it holds
    /// at.
    bounded_clocks: RefCell<Vec<(Duration, SignalCell<Duration>)>>,
    /// The installed regions whose structure changed, to record again at
    /// the host's next update, in the order they changed.
    stale: RefCell<Vec<Stale>>,
}

/// An installed region whose structure changed.
pub enum Stale {
    /// A part to record again.
    Part(Weak<Part>),
    /// A collection whose items were inserted, removed or moved.
    Each(Weak<RefCell<dyn Collection>>),
}

impl Shared {
    /// The shared state of a canvas mounted in `env`, reading its box from
    /// `size`: the cell the [`Canvas`](crate::canvas::Canvas) created.
    pub(crate) fn new(env: Environment, size: SignalCell<Size>) -> Rc<Self> {
        Rc::new(Self {
            env,
            invalidator: RefCell::new(None),
            size,
            clock: SignalCell::new(Duration::ZERO),
            bounded_clocks: RefCell::new(Vec::new()),
            stale: RefCell::new(Vec::new()),
        })
    }

    pub(crate) fn set_invalidator(&self, invalidator: Option<SceneInvalidator>) {
        *self.invalidator.borrow_mut() = invalidator;
    }

    /// Asks the host for a pass: an attribute or the structure changed.
    pub(crate) fn invalidate(&self) {
        let invalidator = self.invalidator.borrow().clone();
        if let Some(invalidator) = invalidator {
            invalidator();
        }
    }

    /// A new clock that holds at `end`.
    fn bounded_clock(&self, end: Duration) -> SignalCell<Duration> {
        let clock = SignalCell::new(Duration::ZERO);
        self.bounded_clocks.borrow_mut().push((end, clock.clone()));
        clock
    }

    /// Advances every observed clock to `elapsed` since the canvas mounted,
    /// a bounded one no further than its end. A bounded clock no recording
    /// holds any more is forgotten.
    pub(crate) fn advance_clocks(&self, elapsed: Duration) {
        if self.clock.observed() && self.clock.snapshot() != elapsed {
            self.clock.set(&Context::from(elapsed));
        }
        self.bounded_clocks
            .borrow_mut()
            .retain(|(_, clock)| clock.shared());
        for (end, clock) in &*self.bounded_clocks.borrow() {
            let now = elapsed.min(*end);
            if clock.observed() && clock.snapshot() != now {
                clock.set(&Context::from(now));
            }
        }
    }

    /// Whether a clock something observes still has frames to advance on:
    /// the unbounded clock, or a bounded one short of its end.
    pub(crate) fn clocks_running(&self) -> bool {
        self.clock.observed()
            || self
                .bounded_clocks
                .borrow()
                .iter()
                .any(|(end, clock)| clock.observed() && clock.snapshot() < *end)
    }

    /// Queues `stale` to record again in place at the host's next update.
    pub(crate) fn mark(&self, stale: Stale) {
        self.stale.borrow_mut().push(stale);
        self.invalidate();
    }

    /// The regions queued since the last call.
    pub(crate) fn take_stale(&self) -> Vec<Stale> {
        core::mem::take(&mut *self.stale.borrow_mut())
    }
}

/// A part of the canvas recorded as a region of its own: a
/// [`Cx::scope`] region, an item of a [`Cx::each`], or a single command
/// whose resource can change. It records again in place, without the rest
/// of the canvas.
pub struct Part {
    pub(crate) recording: RefCell<Recording>,
    /// Its region in the installed recording, once installed.
    pub(crate) handle: RefCell<Option<Region>>,
}

impl Part {
    pub(crate) fn new(recording: Recording) -> Rc<Self> {
        Rc::new(Self {
            recording: RefCell::new(recording),
            handle: RefCell::new(None),
        })
    }

    /// Records the part as a region of `recorder`, through `record`.
    pub(crate) fn install(self: &Rc<Self>, recorder: &mut Recorder, record: &mut RecordPart<'_>) {
        let handle = recorder.region(|recorder| record(self, recorder));
        *self.handle.borrow_mut() = Some(handle);
    }

    /// Records the installed part again in place, through `record`.
    pub(crate) fn rerecord(self: &Rc<Self>, record: &mut RecordPart<'_>) {
        let handle = self.handle.borrow_mut().take();
        if let Some(handle) = handle {
            handle.record(|recorder| record(self, recorder));
            *self.handle.borrow_mut() = Some(handle);
        }
    }
}

/// Records a part's commands into the recorder of its region.
pub type RecordPart<'a> = dyn FnMut(&Rc<Part>, &mut Recorder) + 'a;

/// A [`Cx::each`] as the scene installs it.
pub trait Collection {
    /// Records the items, each a region of its own, inside one region.
    fn install(&mut self, recorder: &mut Recorder, record: &mut RecordPart<'_>);
    /// Brings the installed regions in line with the items: removed items
    /// leave, new ones record, moved ones move — and nothing else records.
    fn apply(&mut self, record: &mut RecordPart<'_>);
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
    Text {
        text: Computed<StyledStr>,
        placement: Computed<TextPlacement>,
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
    /// A part recorded as a region of its own.
    Part(Rc<Part>),
    /// A keyed collection of regions ([`Cx::each`]).
    Each(Rc<RefCell<dyn Collection>>),
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

    /// The time since the canvas was mounted, held at `end` once it gets
    /// there.
    ///
    /// It is a clock like [`time`](Self::time), advancing once per presented
    /// frame while something observes it. Once it reaches `end` it no longer
    /// counts as observing the clock, so a canvas whose only clock readers
    /// have all reached their end presents no more frames.
    #[must_use]
    pub fn time_until(&self, end: Duration) -> Computed<Duration> {
        Computed::new(self.shared.bounded_clock(end))
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
        // Its own part: a new image is a new registration, which records
        // this one command again.
        self.isolate(Node::Image {
            image: image.into().into_computed(),
            dst: rect.into().into_computed(),
            sampling,
        });
    }

    /// Records `node` as a part of its own, so a change of the resource it
    /// draws records only it again.
    fn isolate(&mut self, node: Node) {
        let part = Part::new(Recording {
            nodes: alloc::vec![node],
            kept: Vec::new(),
        });
        self.recording.nodes.push(Node::Part(part));
    }

    /// Draws `run`'s glyphs, exactly as positioned, with `paint`.
    pub fn glyphs<P: IntoPaint>(
        &mut self,
        run: impl Into<Operand<GlyphRun>>,
        paint: impl Into<Operand<P>>,
    ) {
        // Its own part: a run in a font not yet registered records this
        // one command again.
        let node = Node::Glyphs {
            run: run.into().into_computed(),
            paint: self.paint(paint),
        };
        self.isolate(node);
    }

    /// Draws styled `text` anchored by `placement`.
    ///
    /// The target lays the text out with its own text stack, so it matches
    /// every other `Text` on the screen, and resolves its colours, locale
    /// and fonts in the canvas's environment. The canvas never measures: the
    /// [`placement`](TextPlacement) names the anchor, the alignment of the
    /// lines and the edge of the box the anchor sits at, and the width
    /// lines break at. Text whose box layout must know stays a `Text` view
    /// over the canvas.
    ///
    /// A signal of text lays it out again and swaps the layout, and so does
    /// a signal of placements whose alignment or width changed; one whose
    /// origin alone changed moves the laid-out text, animated when the
    /// change carries an animation.
    pub fn text(
        &mut self,
        text: impl Into<Operand<StyledStr>>,
        placement: impl Into<Operand<TextPlacement>>,
    ) {
        // Its own part: a new text is a new layout, which records this one
        // command again.
        self.isolate(Node::Text {
            text: text.into().into_computed(),
            placement: placement.into().into_computed(),
        });
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
        let part = {
            let mut cx = Self::new(Rc::clone(&self.shared));
            signal.with_ref(|value| body(&mut cx, value));
            Part::new(cx.recording)
        };
        let shared = Rc::downgrade(&self.shared);
        let weak = Rc::downgrade(&part);
        let guard: BoxWatcherGuard = signal.watch(move |context| {
            let (Some(shared), Some(part)) = (shared.upgrade(), weak.upgrade()) else {
                return;
            };
            let mut cx = Self::new(Rc::clone(&shared));
            body(&mut cx, context.value());
            // The replaced recording drops after the borrow ends: what it
            // kept may observe the canvas.
            let replaced = core::mem::replace(&mut *part.recording.borrow_mut(), cx.recording);
            drop(replaced);
            shared.mark(Stale::Part(Rc::downgrade(&part)));
        });
        self.recording.kept.push(Box::new(guard));
        self.recording.nodes.push(Node::Part(part));
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
    /// Two items sharing a key is a contract violation: the later item is
    /// not recorded.
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
            container: None,
            installed: Vec::new(),
            removed: Vec::new(),
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
                    let list: Rc<RefCell<dyn Collection>> = list;
                    shared.mark(Stale::Each(Rc::downgrade(&list)));
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
/// the part its body recorded.
struct Entry<T: 'static, K> {
    key: K,
    item: SignalCell<T>,
    part: Rc<Part>,
}

struct Keyed<T: 'static, K> {
    entries: Vec<Entry<T, K>>,
    /// The region the items are installed in, once installed.
    container: Option<Region>,
    /// The keys of the installed item regions, in their installed order.
    installed: Vec<K>,
    /// The installed regions of items that left, to remove.
    removed: Vec<Region>,
}

impl<T: Clone + PartialEq + 'static, K: Eq + Hash + Clone + fmt::Debug + 'static> Collection
    for Keyed<T, K>
{
    fn install(&mut self, recorder: &mut Recorder, record: &mut RecordPart<'_>) {
        // Regions installed before belong to a recording that is gone.
        self.removed.clear();
        let entries = &self.entries;
        let container = recorder.region(|recorder| {
            for entry in entries {
                entry.part.install(recorder, record);
            }
        });
        self.container = Some(container);
        self.installed = self.entries.iter().map(|entry| entry.key.clone()).collect();
    }

    fn apply(&mut self, record: &mut RecordPart<'_>) {
        let Some(container) = &self.container else {
            return;
        };
        for region in self.removed.drain(..) {
            region.remove();
        }
        let present: std::collections::HashSet<&K> =
            self.entries.iter().map(|entry| &entry.key).collect();
        self.installed.retain(|key| present.contains(key));
        for (index, entry) in self.entries.iter().enumerate() {
            let handle = entry.part.handle.borrow();
            match handle.as_ref() {
                None => {
                    drop(handle);
                    let region = container.insert(index, |recorder| record(&entry.part, recorder));
                    *entry.part.handle.borrow_mut() = Some(region);
                    self.installed.insert(index, entry.key.clone());
                }
                Some(region) if self.installed[index] != entry.key => {
                    let from = self
                        .installed
                        .iter()
                        .position(|key| *key == entry.key)
                        .expect("`installed` lists the key of every entry that has a region");
                    region.move_to(index);
                    let key = self.installed.remove(from);
                    self.installed.insert(index, key);
                }
                Some(_) => {}
            }
        }
    }
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
            if entries.iter().any(|entry: &Entry<T, K>| entry.key == key) {
                contract_violation!(
                    "Cx::each: two items share the key {key:?}; the later item is not recorded"
                );
                continue;
            }
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
                    part: Part::new(cx.recording),
                });
            }
        }
        // The regions of items that left are removed at the next update.
        for entry in old.values() {
            if let Some(region) = entry.part.handle.borrow_mut().take() {
                self.removed.push(region);
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
