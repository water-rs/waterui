//! Regions: parts of an installed live recording that record again, leave
//! or move in place, while the rest of the content — its slots, their
//! subscriptions and their running animations — stays as it is.
//!
//! A region is a contiguous, scope-balanced run of commands, opened with
//! [`Recorder::region`]. Regions nest, and the content keeps their tree as
//! segments: each level is a sequence of plain command runs and child
//! regions, so a region's place in the list is computed from the tree,
//! never stored, and an edit only has to adjust the lengths on its path to
//! the root.
//!
//! An edit moves every command after it, so the content's slots are
//! addressed by key instead of index: a key is fixed when a command is
//! recorded, and the content translates it to the command's current index.
//! Until the first edit every key is its index.

use std::any::Any;
use std::fmt;
use std::rc::{Rc, Weak};

use crate::display_list::{DisplayList, Splice};
use crate::record::{LiveState, Recorder};
use crate::size::LayoutSize;

/// One element of a level of the region tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Segment {
    /// Commands recorded directly at this level.
    Commands(u32),
    /// A child region, by slot.
    Region(u32),
}

/// The level a region was recorded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Parent {
    /// The content's top level.
    Root,
    /// Another region, by slot.
    Region(u32),
}

/// A recorded region.
struct Node {
    parent: Parent,
    segments: Vec<Segment>,
    /// Commands the region spans, its child regions' included.
    len: u32,
    /// The subscriptions of its commands and the values recorded with
    /// it ([`Recorder::keep`]), released when it is recorded again or
    /// removed.
    kept: Vec<Box<dyn Any>>,
}

/// Commands whose keys are consecutive, as they lie in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    key: u32,
    index: u32,
    len: u32,
}

/// Where an edit moves the commands of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shift {
    /// `remove` commands at `at` replaced by `insert` new ones.
    Replace { at: u32, remove: u32, insert: u32 },
    /// The `len` commands at `from` moved to start at `to` — an index in
    /// the list without them, which is also their index afterwards.
    Move { from: u32, len: u32, to: u32 },
}

impl Shift {
    /// Where the command at `index` before the edit is after it; `None`
    /// for a removed command.
    pub(crate) const fn index(self, index: u32) -> Option<u32> {
        match self {
            Self::Replace { at, remove, insert } => {
                if index < at {
                    Some(index)
                } else if index < at + remove {
                    None
                } else {
                    Some(index - remove + insert)
                }
            }
            Self::Move { from, len, to } => {
                if index >= from && index < from + len {
                    return Some(to + (index - from));
                }
                let without = if index >= from + len {
                    index - len
                } else {
                    index
                };
                Some(if without >= to {
                    without + len
                } else {
                    without
                })
            }
        }
    }
}

/// An edit queued for the content to apply to its list.
#[derive(Debug)]
pub(crate) struct PendingEdit {
    pub(crate) shift: Shift,
    /// The commands a `Replace` inserts; empty for a `Move`.
    pub(crate) insert: DisplayList,
}

/// The region tree of one live content, its key translation, and the edits
/// queued against its list.
pub(crate) struct Table {
    /// The layout size the content was recorded at: every region records
    /// against it too.
    pub(crate) size: LayoutSize,
    nodes: Vec<Option<Node>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    root: Vec<Segment>,
    /// Commands in the list once every queued edit applies.
    len: u32,
    next_key: u32,
    /// Sorted by key. `None` while every key is its index.
    keys: Option<Vec<Run>>,
    /// Edits not yet applied to the content's list, in order.
    pub(crate) edits: Vec<PendingEdit>,
    /// Edits applied to the content's list and not yet sent, as the
    /// splices that send them.
    pub(crate) staged: Vec<Splice>,
    /// What the edit in progress replaced or removed kept, released once
    /// the edit is queued and the table's borrow has ended.
    pub(crate) retired: Vec<Box<dyn Any>>,
}

impl fmt::Debug for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table")
            .field("regions", &self.nodes.iter().flatten().count())
            .field("len", &self.len)
            .field("edits", &self.edits.len())
            .finish_non_exhaustive()
    }
}

impl Table {
    pub(crate) const fn new(size: LayoutSize) -> Self {
        Self {
            size,
            nodes: Vec::new(),
            generations: Vec::new(),
            free: Vec::new(),
            root: Vec::new(),
            len: 0,
            next_key: 0,
            keys: None,
            edits: Vec::new(),
            staged: Vec::new(),
            retired: Vec::new(),
        }
    }

    /// A slot for a region being recorded in `parent`, and its generation.
    pub(crate) fn allocate(&mut self, parent: Parent) -> (u32, u32) {
        let node = Node {
            parent,
            segments: Vec::new(),
            len: 0,
            kept: Vec::new(),
        };
        if let Some(slot) = self.free.pop() {
            self.nodes[slot as usize] = Some(node);
            (slot, self.generations[slot as usize])
        } else {
            let slot = u32::try_from(self.nodes.len()).expect("fewer than u32::MAX regions");
            self.nodes.push(Some(node));
            self.generations.push(0);
            (slot, 0)
        }
    }

    /// Stores what recording the region in `slot` produced.
    pub(crate) fn fill(
        &mut self,
        slot: u32,
        segments: Vec<Segment>,
        len: u32,
        kept: Vec<Box<dyn Any>>,
    ) {
        let node = self.node_mut(slot);
        node.segments = segments;
        node.len = len;
        node.kept = kept;
    }

    /// Stores the content's top level once its first recording finishes.
    pub(crate) fn finish_root(&mut self, segments: Vec<Segment>, len: u32) {
        self.root = segments;
        self.len = len;
        self.next_key = len;
    }

    /// Whether the region a handle names is still recorded.
    pub(crate) fn holds(&self, slot: u32, generation: u32) -> bool {
        self.nodes.get(slot as usize).is_some_and(Option::is_some)
            && self.generations[slot as usize] == generation
    }

    /// The first key of a region recording that starts now.
    pub(crate) const fn key_base(&self) -> u32 {
        self.next_key
    }

    /// Translates a command's key to its index once the queued edits
    /// apply; `None` for a command an edit removed.
    pub(crate) fn index_of(&self, key: u32) -> Option<u32> {
        let Some(runs) = &self.keys else {
            return Some(key);
        };
        let at = runs.partition_point(|run| run.key + run.len <= key);
        runs.get(at)
            .filter(|run| run.key <= key)
            .map(|run| run.index + (key - run.key))
    }

    fn node(&self, slot: u32) -> &Node {
        self.nodes[slot as usize]
            .as_ref()
            .expect("a live region's slot holds its node")
    }

    fn node_mut(&mut self, slot: u32) -> &mut Node {
        self.nodes[slot as usize]
            .as_mut()
            .expect("a live region's slot holds its node")
    }

    fn segments(&self, parent: Parent) -> &[Segment] {
        match parent {
            Parent::Root => &self.root,
            Parent::Region(slot) => &self.node(slot).segments,
        }
    }

    fn segments_mut(&mut self, parent: Parent) -> &mut Vec<Segment> {
        match parent {
            Parent::Root => &mut self.root,
            Parent::Region(slot) => &mut self.node_mut(slot).segments,
        }
    }

    fn segment_len(&self, segment: Segment) -> u32 {
        match segment {
            Segment::Commands(len) => len,
            Segment::Region(slot) => self.node(slot).len,
        }
    }

    /// The index of the region's first command.
    pub(crate) fn start(&self, slot: u32) -> u32 {
        let parent = self.node(slot).parent;
        let mut offset = 0;
        for &segment in self.segments(parent) {
            if segment == Segment::Region(slot) {
                break;
            }
            offset += self.segment_len(segment);
        }
        offset
            + match parent {
                Parent::Root => 0,
                Parent::Region(parent) => self.start(parent),
            }
    }

    /// Adds `delta` to the length of every region from `parent` up.
    fn grow(&mut self, mut parent: Parent, delta: i64) {
        while let Parent::Region(slot) = parent {
            let node = self.node_mut(slot);
            node.len =
                u32::try_from(i64::from(node.len) + delta).expect("a region's length fits u32");
            parent = node.parent;
        }
        self.len = u32::try_from(i64::from(self.len) + delta).expect("a list's length fits u32");
    }

    /// Removes the region in `slot` and every region inside it, keeping
    /// what they kept for release after the edit reaches the target.
    fn drop_subtree(&mut self, slot: u32) {
        let node = self.nodes[slot as usize]
            .take()
            .expect("a live region's slot holds its node");
        self.generations[slot as usize] += 1;
        self.free.push(slot);
        self.retired.extend(node.kept);
        for segment in node.segments {
            if let Segment::Region(child) = segment {
                self.drop_subtree(child);
            }
        }
    }

    /// The key runs, made explicit before the first edit moves anything.
    fn runs(&mut self) -> &mut Vec<Run> {
        let len = self.len;
        self.keys.get_or_insert_with(|| {
            if len == 0 {
                Vec::new()
            } else {
                vec![Run {
                    key: 0,
                    index: 0,
                    len,
                }]
            }
        })
    }

    /// Splits the runs so that none straddles index `at`.
    fn split_runs(runs: &mut Vec<Run>, at: u32) {
        let mut tail = Vec::new();
        for run in runs.iter_mut() {
            if run.index < at && at < run.index + run.len {
                let head = at - run.index;
                tail.push(Run {
                    key: run.key + head,
                    index: at,
                    len: run.len - head,
                });
                run.len = head;
            }
        }
        runs.extend(tail);
    }

    /// Takes out the runs of the `len` commands at `at`, indexed from
    /// their own start, closing the gap they leave.
    fn extract_runs(&mut self, at: u32, len: u32) -> Vec<Run> {
        let runs = self.runs();
        Self::split_runs(runs, at);
        Self::split_runs(runs, at + len);
        let mut taken = Vec::new();
        runs.retain_mut(|run| {
            if run.index >= at && run.index < at + len {
                taken.push(Run {
                    index: run.index - at,
                    ..*run
                });
                false
            } else {
                if run.index >= at + len {
                    run.index -= len;
                }
                true
            }
        });
        taken
    }

    /// Opens a gap of `len` commands at `at` and places `taken` — runs
    /// indexed from the gap's start — in it.
    fn insert_runs(&mut self, at: u32, len: u32, taken: Vec<Run>) {
        let runs = self.runs();
        Self::split_runs(runs, at);
        for run in runs.iter_mut() {
            if run.index >= at {
                run.index += len;
            }
        }
        runs.extend(taken.into_iter().map(|run| Run {
            index: run.index + at,
            ..run
        }));
        runs.sort_unstable_by_key(|run| run.key);
    }

    /// The segment index a child region placed before the parent's
    /// `index`-th child region goes at — the end when there is none.
    fn child_position(&self, parent: Parent, index: usize) -> usize {
        let segments = self.segments(parent);
        segments
            .iter()
            .enumerate()
            .filter(|(_, segment)| matches!(segment, Segment::Region(_)))
            .nth(index)
            .map_or(segments.len(), |(position, _)| position)
    }

    /// The fresh recording of a region whose keys start at `key_base`:
    /// queues the edit and returns how it moves the list.
    fn queue_insert(&mut self, at: u32, remove: u32, insert: DisplayList, key_base: u32) -> Shift {
        let inserted = u32::try_from(insert.len()).expect("a region's length fits u32");
        let _ = self.extract_runs(at, remove);
        let fresh = if inserted == 0 {
            Vec::new()
        } else {
            vec![Run {
                key: key_base,
                index: 0,
                len: inserted,
            }]
        };
        self.insert_runs(at, inserted, fresh);
        self.next_key = key_base + inserted;
        let shift = Shift::Replace {
            at,
            remove,
            insert: inserted,
        };
        self.edits.push(PendingEdit { shift, insert });
        shift
    }

    /// Replaces the region in `slot` with a new recording of it.
    pub(crate) fn replace(&mut self, slot: u32, recorded: Recorded) -> Shift {
        let at = self.start(slot);
        let Recorded {
            list,
            segments,
            kept,
            key_base,
        } = recorded;
        let len = u32::try_from(list.len()).expect("a region's length fits u32");
        let node = self.node_mut(slot);
        let old_len = node.len;
        let old_segments = std::mem::replace(&mut node.segments, segments);
        let old_kept = std::mem::replace(&mut node.kept, kept);
        node.len = len;
        let parent = node.parent;
        self.retired.extend(old_kept);
        for segment in old_segments {
            if let Segment::Region(child) = segment {
                self.drop_subtree(child);
            }
        }
        self.grow(parent, i64::from(len) - i64::from(old_len));
        self.queue_insert(at, old_len, list, key_base)
    }

    /// Places the freshly recorded region `slot` before `parent`'s
    /// `index`-th child region.
    pub(crate) fn insert(
        &mut self,
        parent: Parent,
        index: usize,
        slot: u32,
        recorded: Recorded,
    ) -> Shift {
        let Recorded {
            list,
            segments,
            kept,
            key_base,
        } = recorded;
        let len = u32::try_from(list.len()).expect("a region's length fits u32");
        self.fill(slot, segments, len, kept);
        let position = self.child_position(parent, index);
        self.segments_mut(parent)
            .insert(position, Segment::Region(slot));
        let at = self.start(slot);
        self.grow(parent, i64::from(len));
        self.queue_insert(at, 0, list, key_base)
    }

    /// Removes the region in `slot` and its commands.
    pub(crate) fn remove(&mut self, slot: u32) -> Shift {
        let at = self.start(slot);
        let (parent, len) = self.place(slot);
        self.segments_mut(parent)
            .retain(|segment| *segment != Segment::Region(slot));
        self.drop_subtree(slot);
        self.grow(parent, -i64::from(len));
        let _ = self.extract_runs(at, len);
        let shift = Shift::Replace {
            at,
            remove: len,
            insert: 0,
        };
        self.edits.push(PendingEdit {
            shift,
            insert: DisplayList::default(),
        });
        shift
    }

    /// Moves the region in `slot` before its parent's `index`-th other
    /// child region — to the end when there is none.
    pub(crate) fn move_to(&mut self, slot: u32, index: usize) -> Option<Shift> {
        let from = self.start(slot);
        let (parent, len) = self.place(slot);
        self.segments_mut(parent)
            .retain(|segment| *segment != Segment::Region(slot));
        let position = self.child_position(parent, index);
        self.segments_mut(parent)
            .insert(position, Segment::Region(slot));
        let to = self.start(slot);
        if to == from || len == 0 {
            return None;
        }
        let taken = self.extract_runs(from, len);
        self.insert_runs(to, len, taken);
        let shift = Shift::Move { from, len, to };
        self.edits.push(PendingEdit {
            shift,
            insert: DisplayList::default(),
        });
        Some(shift)
    }

    /// The level the region in `slot` lives in, and its length.
    fn place(&self, slot: u32) -> (Parent, u32) {
        let node = self.node(slot);
        (node.parent, node.len)
    }
}

/// What a region recording produced.
pub(crate) struct Recorded {
    pub(crate) list: DisplayList,
    pub(crate) segments: Vec<Segment>,
    pub(crate) kept: Vec<Box<dyn Any>>,
    pub(crate) key_base: u32,
}

/// A region of an installed live recording: a part that records again,
/// leaves or moves in place.
///
/// [`Recorder::region`] opens one and returns its handle. Every operation
/// edits the installed content directly: the target receives the commands
/// that changed as an [`Edit`](crate::Edit), and the content's other slots
/// — their subscriptions, and animations running on them — stay as they
/// are, whatever indices the edit moves them to.
///
/// The handle does not keep the region: a region lives until it is
/// recorded over, removed, or its content retires. Once it is gone the
/// handle's operations do nothing — a host that retires a content records
/// it again whole, with fresh handles.
pub struct Region {
    live: Weak<LiveState>,
    slot: u32,
    generation: u32,
}

impl fmt::Debug for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Region")
            .field("slot", &self.slot)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Region {
    pub(crate) const fn new(live: Weak<LiveState>, slot: u32, generation: u32) -> Self {
        Self {
            live,
            slot,
            generation,
        }
    }

    /// Whether the region is still recorded: its content has not retired
    /// or dropped, and the region has not been recorded over or removed.
    #[must_use]
    pub fn is_recorded(&self) -> bool {
        self.live().is_some()
    }

    /// The content and this region's slot, while the region is recorded.
    fn live(&self) -> Option<Rc<LiveState>> {
        let live = self.live.upgrade()?;
        live.holds(self.slot, self.generation).then_some(live)
    }

    /// Records the region again: its commands are replaced by what `body`
    /// records, and everything it recorded before — subscriptions, nested
    /// regions, kept values — is released as the edit is queued.
    pub fn record(&self, body: impl FnOnce(&mut Recorder)) {
        let Some(live) = self.live() else {
            return;
        };
        let recorded = Recorder::record_region(&live, self.slot, body);
        live.edit(|table| Some(table.replace(self.slot, recorded)));
    }

    /// Records `body` as a new region inside this one, placed before its
    /// `index`-th child region, or after the last when `index` is the
    /// number of children.
    ///
    /// # Panics
    ///
    /// When the region is gone.
    #[must_use = "a region is removed only through its handle"]
    pub fn insert(&self, index: usize, body: impl FnOnce(&mut Recorder)) -> Self {
        let live = self
            .live()
            .expect("a region inserts children only while it is recorded");
        let (slot, generation) = live.allocate(Parent::Region(self.slot));
        let recorded = Recorder::record_region(&live, slot, body);
        live.edit(|table| Some(table.insert(Parent::Region(self.slot), index, slot, recorded)));
        Self::new(Rc::downgrade(&live), slot, generation)
    }

    /// Removes the region and its commands.
    pub fn remove(&self) {
        if let Some(live) = self.live() {
            live.edit(|table| Some(table.remove(self.slot)));
        }
    }

    /// Moves the region, commands, slots and running animations included,
    /// before its parent's `index`-th other child region — after the last
    /// when `index` is their number. Nothing records again.
    pub fn move_to(&self, index: usize) {
        if let Some(live) = self.live() {
            live.edit(|table| table.move_to(self.slot, index));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use kurbo::{Affine, Rect};
    use nami::{SignalExt as _, binding};

    use super::Shift;
    use crate::color::{Color, Srgb};
    use crate::display_list::{Command, Picture};
    use crate::paint::Paint;
    use crate::record::{Content, ContentChange, Draw as _};
    use crate::shape::ShapeData;
    use crate::size::LayoutSize;
    use crate::{Animation, Curve, Instant, WorkingColor};

    fn grey() -> Color<Srgb> {
        Color::new([0.5, 0.5, 0.5, 1.])
    }

    fn square(x: f64) -> Rect {
        Rect::new(x, 0., x + 1., 1.)
    }

    /// The target's copy of `content`: its first change, then every later
    /// one applied the way a target applies them.
    struct Remote(Picture);

    impl Remote {
        fn install(content: &mut Content) -> Self {
            let Some(ContentChange::Replace(picture)) = content.take_change() else {
                panic!("the first change sends the whole list");
            };
            Self(picture)
        }

        /// Applies the next change; returns it.
        fn sync(&mut self, content: &mut Content) -> Option<ContentChange> {
            let change = content.take_change();
            match change.clone() {
                Some(ContentChange::Update(updates)) => {
                    let _ = self.0.apply(updates);
                }
                Some(ContentChange::Edit(edit)) => self.0.edit(*edit),
                Some(ContentChange::Replace(_)) => panic!("an installed content never resends"),
                None => {}
            }
            assert_eq!(
                self.0.display_list(),
                content.snapshot(),
                "the target's list follows the content's"
            );
            change
        }

        fn command(&self, index: usize) -> &Command {
            &self.0.display_list().commands()[index]
        }
    }

    #[test]
    fn recording_a_region_again_keeps_the_animation_running_after_it() {
        let colour = binding(WorkingColor::new([1., 0., 0., 1.]));
        let mut region = None;
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.fill(square(0.), grey());
            region = Some(c.region(|c| c.fill(square(1.), grey())));
            c.fill(
                square(2.),
                colour
                    .clone()
                    .with(Animation::from(Curve::linear(Duration::from_millis(400)))),
            );
        });
        let region = region.expect("the region was recorded");
        let mut remote = Remote::install(&mut content);

        let start = Instant::now();
        colour.set(WorkingColor::new([0., 1., 0., 1.]));
        assert!(content.sample(start).is_animating());
        let _ = remote.sync(&mut content);

        // The region grows from one command to four, moving the
        // animated fill from index 2 to 5.
        region.record(|c| {
            c.fill(square(1.), grey());
            c.transform(Affine::translate((1., 0.)), |c| c.fill(square(1.), grey()));
        });
        assert!(
            content
                .sample(start + Duration::from_millis(200))
                .is_animating(),
            "the edit leaves the track running"
        );
        let Some(ContentChange::Edit(edit)) = remote.sync(&mut content) else {
            panic!("a region recorded again sends an edit");
        };
        assert_eq!(edit.splices.len(), 1);
        assert_eq!((edit.splices[0].at, edit.splices[0].remove), (1, 1));
        assert_eq!(edit.splices[0].insert.len(), 4);
        let Command::Fill {
            paint: Paint::Solid(mid),
            ..
        } = remote.command(5)
        else {
            panic!("the animated fill moved to index 5");
        };
        assert!((mid.components[1] - 0.5).abs() < 0.01, "mid {mid:?}");

        // A later change of the same signal reaches the moved command.
        // The retarget's first sample holds its position; it settles after.
        colour.set(WorkingColor::new([0., 0., 1., 1.]));
        let _ = content.sample(start + Duration::from_secs(2));
        let _ = remote.sync(&mut content);
        let _ = content.sample(start + Duration::from_secs(4));
        let _ = remote.sync(&mut content);
        assert!(matches!(
            remote.command(5),
            Command::Fill { paint: Paint::Solid(blue), .. } if blue.components[2] > 0.99
        ));
    }

    #[test]
    fn inserted_moved_and_removed_regions_keep_their_slots() {
        let a = binding(0.0_f64);
        let b = binding(10.0_f64);
        let mut items = Vec::new();
        let mut container = None;
        let mut content = Content::record(&LayoutSize::new(), |c| {
            c.clip(Rect::new(0., 0., 100., 100.), |c| {
                container = Some(c.region(|c| {
                    items.push(c.region(|c| c.fill(a.clone().map(square), grey())));
                    items.push(c.region(|c| {
                        c.transform(Affine::IDENTITY, |c| c.fill(b.clone().map(square), grey()));
                    }));
                }));
            });
            c.fill(square(50.), grey());
        });
        let container = container.expect("the container was recorded");
        let mut remote = Remote::install(&mut content);

        // Move `b` before `a`: no command records again.
        items[1].move_to(0);
        let _ = remote.sync(&mut content);
        assert!(matches!(
            remote.command(1),
            Command::BeginTransform { end: 3, .. }
        ));
        a.set(5.);
        let _ = remote.sync(&mut content);
        assert_eq!(
            remote.0.display_list().view().get(crate::Slot {
                command: 4,
                operand: crate::OperandKind::Shape,
            }),
            Some(crate::OperandRef::Shape(&ShapeData::Rect(square(5.)))),
            "`a`'s fill moved behind `b` and still follows its signal"
        );

        // Insert a new item between them, then remove `b`.
        let c_item = container.insert(1, |c| c.fill(square(30.), grey()));
        let _ = remote.sync(&mut content);
        items[1].remove();
        let _ = remote.sync(&mut content);
        b.set(20.);
        assert_eq!(
            content.take_change(),
            None,
            "a removed region's slots are gone"
        );
        a.set(7.);
        let _ = remote.sync(&mut content);
        c_item.record(|c| c.fill(square(31.), grey()));
        let _ = remote.sync(&mut content);
        let expected = Content::record(&LayoutSize::new(), |c| {
            c.clip(Rect::new(0., 0., 100., 100.), |c| {
                c.fill(square(31.), grey());
                c.fill(square(7.), grey());
            });
            c.fill(square(50.), grey());
        })
        .into_picture();
        assert_eq!(remote.0.display_list(), expected.display_list());
    }

    #[test]
    fn a_move_places_the_block_and_closes_its_gap() {
        // [a b c d e]: move "b c" (1..3) to after "d" — index 2 in [a d e].
        let shift = Shift::Move {
            from: 1,
            len: 2,
            to: 2,
        };
        let moved: Vec<_> = (0..5).map(|i| shift.index(i).unwrap()).collect();
        // a d b c e
        assert_eq!(moved, [0, 2, 3, 1, 4]);
    }

    #[test]
    fn a_replace_drops_the_removed_commands_and_shifts_the_rest() {
        let shift = Shift::Replace {
            at: 1,
            remove: 2,
            insert: 3,
        };
        let moved: Vec<_> = (0..5).map(|i| shift.index(i)).collect();
        assert_eq!(moved, [Some(0), None, None, Some(4), Some(5)]);
    }
}
