//! The display list: the owned, serializable form of recorded content, and the
//! slot updates that patch it.

use std::ops::Range;
use std::sync::Arc;

use kurbo::{Affine, Rect, Stroke};

use crate::animation::AnimLanes;
use crate::glyph::GlyphRun;
use crate::paint::{ImageId, Paint, Sampling};
use crate::resource::ResourceId;
use crate::shape::ShapeData;
use crate::style::{Group, Shadow};

/// One recorded command.
///
/// Scopes are flat: a `Begin*` command records the index of its matching
/// [`Command::End`], so a consumer can skip or regenerate a whole scope.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Command {
    /// Fill a shape.
    Fill {
        /// The shape.
        shape: ShapeData,
        /// The paint.
        paint: Paint,
    },
    /// Stroke a shape.
    Stroke {
        /// The shape.
        shape: ShapeData,
        /// The stroke style.
        stroke: Stroke,
        /// The paint.
        paint: Paint,
    },
    /// Cast a shadow from a shape.
    Shadow {
        /// The shape.
        shape: ShapeData,
        /// The shadow.
        shadow: Shadow,
    },
    /// Draw a glyph run.
    Glyphs {
        /// The run.
        run: GlyphRun,
        /// The paint.
        paint: Paint,
    },
    /// Draw an image into a rectangle.
    Image {
        /// The image.
        image: ImageId,
        /// Destination rectangle.
        dst: Rect,
        /// Sampling.
        sampling: Sampling,
    },
    /// Draw a shared picture.
    Picture {
        /// The picture.
        picture: Picture,
        /// Where it is placed.
        transform: Affine,
    },
    /// Clip the scope to a shape.
    BeginClip {
        /// The clip shape.
        shape: ShapeData,
        /// Index of the matching `End`.
        end: u32,
    },
    /// Transform the scope.
    BeginTransform {
        /// The transform.
        transform: Affine,
        /// Index of the matching `End`.
        end: u32,
    },
    /// Isolate the scope as a group.
    BeginGroup {
        /// The group style.
        group: Group,
        /// Index of the matching `End`.
        end: u32,
    },
    /// Close the innermost scope.
    End,
}

/// Which value of a command a slot addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum OperandKind {
    /// The shape of a fill, stroke, shadow or clip.
    Shape,
    /// The paint of a fill, stroke or glyph run.
    Paint,
    /// The stroke style of a stroke.
    Stroke,
    /// The shadow of a shadow command.
    Shadow,
    /// The transform of a picture or a transform scope.
    Transform,
    /// The group style of a group scope.
    Group,
    /// The destination rectangle of an image.
    Rect,
    /// The run of a glyph run.
    Run,
}

/// A value that replaces one operand of a command.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Operand {
    /// A shape.
    Shape(ShapeData),
    /// A paint.
    Paint(Paint),
    /// A stroke style.
    Stroke(Stroke),
    /// A shadow.
    Shadow(Shadow),
    /// A transform.
    Transform(Affine),
    /// A group style.
    Group(Group),
    /// A rectangle.
    Rect(Rect),
    /// A glyph run.
    Run(GlyphRun),
}

impl AnimLanes for Operand {
    fn anim_lanes(&self, target: &Self) -> Option<Box<[f64]>> {
        match (self, target) {
            (Self::Shape(from), Self::Shape(to)) => from.anim_lanes(to),
            (Self::Paint(from), Self::Paint(to)) => from.anim_lanes(to),
            (Self::Stroke(from), Self::Stroke(to)) => from.anim_lanes(to),
            (Self::Shadow(from), Self::Shadow(to)) => from.anim_lanes(to),
            (Self::Rect(from), Self::Rect(to)) => from.anim_lanes(to),
            (Self::Transform(from), Self::Transform(to)) => from.anim_lanes(to),
            (Self::Group(from), Self::Group(to)) => from.anim_lanes(to),
            // A glyph run and a variant change have no lane decomposition:
            // the update snaps.
            _ => None,
        }
    }

    fn with_lanes(&self, lanes: &[f64]) -> Self {
        match self {
            Self::Shape(shape) => Self::Shape(shape.with_lanes(lanes)),
            Self::Paint(paint) => Self::Paint(paint.with_lanes(lanes)),
            Self::Stroke(stroke) => Self::Stroke(stroke.with_lanes(lanes)),
            Self::Shadow(shadow) => Self::Shadow(shadow.with_lanes(lanes)),
            Self::Rect(rect) => Self::Rect(rect.with_lanes(lanes)),
            Self::Transform(transform) => Self::Transform(transform.with_lanes(lanes)),
            Self::Group(group) => Self::Group(group.with_lanes(lanes)),
            // No lane decomposition, so a run never gets here.
            Self::Run(run) => Self::Run(run.clone()),
        }
    }
}

impl Operand {
    /// Which operand this value replaces.
    #[must_use]
    pub const fn kind(&self) -> OperandKind {
        match self {
            Self::Shape(_) => OperandKind::Shape,
            Self::Paint(_) => OperandKind::Paint,
            Self::Stroke(_) => OperandKind::Stroke,
            Self::Shadow(_) => OperandKind::Shadow,
            Self::Transform(_) => OperandKind::Transform,
            Self::Group(_) => OperandKind::Group,
            Self::Rect(_) => OperandKind::Rect,
            Self::Run(_) => OperandKind::Run,
        }
    }
}

/// The address of a value in a display list: one operand of one command.
///
/// A value recorded from a signal owns a slot; when the signal changes, the
/// slot's new value is sent as a [`SlotUpdate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Slot {
    /// Index of the command.
    pub command: u32,
    /// Which operand of the command.
    pub operand: OperandKind,
}

/// A new value for a slot.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SlotUpdate {
    /// Index of the command.
    pub command: u32,
    /// The new value. Its kind selects the operand.
    pub value: Operand,
}

impl SlotUpdate {
    /// The slot this update addresses.
    #[must_use]
    pub const fn slot(&self) -> Slot {
        Slot {
            command: self.command,
            operand: self.value.kind(),
        }
    }
}

/// Commands that must be regenerated after updates, as sorted, disjoint
/// ranges of command indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dirty {
    ranges: Vec<Range<u32>>,
}

impl Dirty {
    /// The dirty ranges, sorted and disjoint.
    #[must_use]
    pub fn ranges(&self) -> &[Range<u32>] {
        &self.ranges
    }

    /// Whether nothing needs regenerating.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Merges `other` into this set: afterwards the ranges cover both
    /// sets. Reuse the same normalized form as a fresh merge.
    pub fn union(&mut self, other: Self) {
        self.ranges.extend(other.ranges);
        *self = Self::from_unsorted(std::mem::take(&mut self.ranges));
    }

    /// Whether a command needs regenerating.
    #[must_use]
    pub fn contains(&self, command: u32) -> bool {
        self.ranges.iter().any(|range| range.contains(&command))
    }

    fn from_unsorted(mut ranges: Vec<Range<u32>>) -> Self {
        ranges.sort_unstable_by_key(|range| range.start);
        let mut merged: Vec<Range<u32>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        Self { ranges: merged }
    }
}

/// Recorded content: an ordered list of commands.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "DisplayListData")
)]
pub struct DisplayList {
    commands: Vec<Command>,
}

/// A display list's commands before their scopes are validated: the form it
/// deserializes from, so that captured scenes cannot bypass the invariants.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct DisplayListData {
    commands: Vec<Command>,
}

/// Why a display list's scope structure is malformed.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ScopeError {
    /// The list holds more commands than a `u32` index can address.
    #[error("a display list holds at most u32::MAX commands, got {len}")]
    TooLong {
        /// Number of commands.
        len: usize,
    },
    /// An `End` closes no open scope.
    #[error("command {index} ends a scope that was never opened")]
    UnmatchedEnd {
        /// Index of the `End`.
        index: usize,
    },
    /// A `Begin*` records the wrong index for its `End`.
    #[error("the scope opened at {begin} records its end at {recorded}, but it ends at {actual}")]
    WrongEnd {
        /// Index of the `Begin*`.
        begin: usize,
        /// The end index the command records.
        recorded: u32,
        /// Where its `End` actually is.
        actual: usize,
    },
    /// A scope is never closed.
    #[error("the scope opened at {begin} is never closed")]
    Unclosed {
        /// Index of the `Begin*`.
        begin: usize,
    },
}

#[cfg(feature = "serde")]
impl TryFrom<DisplayListData> for DisplayList {
    type Error = ScopeError;

    fn try_from(data: DisplayListData) -> Result<Self, Self::Error> {
        let commands = data.commands;
        if u32::try_from(commands.len()).is_err() {
            return Err(ScopeError::TooLong {
                len: commands.len(),
            });
        }
        let mut open: Vec<(usize, u32)> = Vec::new();
        for (index, command) in commands.iter().enumerate() {
            match command {
                Command::BeginClip { end, .. }
                | Command::BeginTransform { end, .. }
                | Command::BeginGroup { end, .. } => open.push((index, *end)),
                Command::End => {
                    let (begin, recorded) = open.pop().ok_or(ScopeError::UnmatchedEnd { index })?;
                    if recorded as usize != index {
                        return Err(ScopeError::WrongEnd {
                            begin,
                            recorded,
                            actual: index,
                        });
                    }
                }
                Command::Fill { .. }
                | Command::Stroke { .. }
                | Command::Shadow { .. }
                | Command::Glyphs { .. }
                | Command::Image { .. }
                | Command::Picture { .. } => {}
            }
        }
        match open.pop() {
            Some((begin, _)) => Err(ScopeError::Unclosed { begin }),
            None => Ok(Self { commands }),
        }
    }
}

impl DisplayList {
    /// An empty list with room for `capacity` commands.
    // The recorders build lists through this and `push`.
    #[must_use]
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self {
            commands: Vec::with_capacity(capacity),
        }
    }

    /// Releases spare capacity when less than half of it was used, so a
    /// small list recorded after a large one does not keep its reservation.
    pub(crate) fn trim_spare(&mut self) {
        if self.commands.capacity() > 2 * self.commands.len() {
            self.commands.shrink_to_fit();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.commands.clear();
    }

    /// The commands.
    #[must_use]
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// A read-only view of the list, as a render target lowers it: the
    /// commands in order and, for each command, the operand slots a
    /// [`SlotUpdate`] can address with the values currently recorded. See
    /// [`DisplayListView`].
    #[must_use]
    pub const fn view(&self) -> DisplayListView<'_> {
        DisplayListView { list: self }
    }

    /// Heap bytes held by the command buffer, including each command's own
    /// allocations as reported by `nested`.
    pub fn heap_bytes(&self, mut nested: impl FnMut(&Command) -> u64) -> u64 {
        let mut bytes = (self.commands.capacity() * size_of::<Command>()) as u64;
        for command in &self.commands {
            bytes += nested(command);
        }
        bytes
    }

    /// Whether any command, including those of nested pictures, samples
    /// `resource`: a glyph run's font, an image draw, or an image or shader
    /// paint of a fill, stroke or glyph run. Content never names a backdrop
    /// shader; layers sample those through the tree. A render target's
    /// resource-liveness bookkeeping.
    #[must_use]
    pub fn references(&self, resource: ResourceId) -> bool {
        self.commands.iter().any(|command| match command {
            Command::Fill { paint, .. } | Command::Stroke { paint, .. } => {
                paint.references(resource)
            }
            Command::Glyphs { run, paint } => {
                resource == ResourceId::Font(run.font) || paint.references(resource)
            }
            Command::Image { image, .. } => resource == ResourceId::Image(*image),
            Command::Picture { picture, .. } => picture.display_list().references(resource),
            Command::Shadow { .. }
            | Command::BeginClip { .. }
            | Command::BeginTransform { .. }
            | Command::BeginGroup { .. }
            | Command::End => false,
        })
    }

    /// Number of commands.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether the list has no commands.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Applies slot updates and returns the commands to regenerate. An update
    /// to a leaf command dirties that command; an update to a scope dirties the
    /// whole scope through its `End`.
    ///
    /// # Panics
    ///
    /// Panics when an update addresses a command that does not exist or an
    /// operand the command does not have. Updates come from the recorder that
    /// produced this list, so either is a bug.
    pub fn apply(&mut self, updates: impl IntoIterator<Item = SlotUpdate>) -> Dirty {
        let mut dirty = Vec::new();
        for SlotUpdate { command, value } in updates {
            let index = command as usize;
            let len = self.commands.len();
            let target = self
                .commands
                .get_mut(index)
                .unwrap_or_else(|| panic!("slot update for command {index} of {len}"));
            let end = match (target, value) {
                (
                    Command::Fill { shape, .. }
                    | Command::Stroke { shape, .. }
                    | Command::Shadow { shape, .. },
                    Operand::Shape(new),
                ) => {
                    *shape = new;
                    command
                }
                (
                    Command::Fill { paint, .. }
                    | Command::Stroke { paint, .. }
                    | Command::Glyphs { paint, .. },
                    Operand::Paint(new),
                ) => {
                    *paint = new;
                    command
                }
                (Command::Stroke { stroke, .. }, Operand::Stroke(new)) => {
                    *stroke = new;
                    command
                }
                (Command::Glyphs { run, .. }, Operand::Run(new)) => {
                    *run = new;
                    command
                }
                (Command::Shadow { shadow, .. }, Operand::Shadow(new)) => {
                    *shadow = new;
                    command
                }
                (Command::Image { dst, .. }, Operand::Rect(new)) => {
                    *dst = new;
                    command
                }
                (Command::Picture { transform, .. }, Operand::Transform(new)) => {
                    *transform = new;
                    command
                }
                (Command::BeginClip { shape, end }, Operand::Shape(new)) => {
                    *shape = new;
                    *end
                }
                (Command::BeginTransform { transform, end }, Operand::Transform(new)) => {
                    *transform = new;
                    *end
                }
                (Command::BeginGroup { group, end }, Operand::Group(new)) => {
                    *group = new;
                    *end
                }
                (target, value) => panic!(
                    "slot update {:?} does not match command {index}: {target:?}",
                    value.kind()
                ),
            };
            dirty.push(command..end + 1);
        }
        Dirty::from_unsorted(dirty)
    }

    /// The operand `kind` currently recorded on command `index` — the
    /// value an animated change starts from. `None` when the index is out
    /// of range or the command has no such operand.
    pub(crate) fn operand(&self, index: u32, kind: OperandKind) -> Option<Operand> {
        let target = self.commands.get(index as usize)?;
        Some(match (target, kind) {
            (
                Command::Fill { shape, .. }
                | Command::Stroke { shape, .. }
                | Command::Shadow { shape, .. }
                | Command::BeginClip { shape, .. },
                OperandKind::Shape,
            ) => Operand::Shape(shape.clone()),
            (
                Command::Fill { paint, .. }
                | Command::Stroke { paint, .. }
                | Command::Glyphs { paint, .. },
                OperandKind::Paint,
            ) => Operand::Paint(paint.clone()),
            (Command::Stroke { stroke, .. }, OperandKind::Stroke) => {
                Operand::Stroke(stroke.clone())
            }
            (Command::Glyphs { run, .. }, OperandKind::Run) => Operand::Run(run.clone()),
            (Command::Shadow { shadow, .. }, OperandKind::Shadow) => Operand::Shadow(*shadow),
            (Command::Image { dst, .. }, OperandKind::Rect) => Operand::Rect(*dst),
            (
                Command::Picture { transform, .. } | Command::BeginTransform { transform, .. },
                OperandKind::Transform,
            ) => Operand::Transform(*transform),
            (Command::BeginGroup { group, .. }, OperandKind::Group) => Operand::Group(*group),
            _ => return None,
        })
    }

    // The recorders append through this.
    pub(crate) fn push(&mut self, command: Command) -> u32 {
        let index = u32::try_from(self.commands.len())
            .expect("a display list holds at most u32::MAX commands");
        self.commands.push(command);
        index
    }

    /// Closes the scope opened by the `Begin*` command at `begin`.
    pub(crate) fn end(&mut self, begin: u32) {
        let end = self.push(Command::End);
        match &mut self.commands[begin as usize] {
            Command::BeginClip { end: slot, .. }
            | Command::BeginTransform { end: slot, .. }
            | Command::BeginGroup { end: slot, .. } => *slot = end,
            other => unreachable!("command {begin} opens no scope: {other:?}"),
        }
    }
}

/// A read-only view of a [`DisplayList`], as another render target lowers
/// it: the commands in order and the live operands each [`Slot`]
/// addresses.
///
/// The view borrows the list — no recorded storage is cloned. A target
/// installs the list by lowering [`commands`](Self::commands); a later
/// [`SlotUpdate`] applies through [`DisplayList::apply`], and a view taken
/// afterwards reads the new value.
#[derive(Debug)]
pub struct DisplayListView<'a> {
    list: &'a DisplayList,
}

impl<'a> DisplayListView<'a> {
    /// The commands, in order.
    #[must_use]
    pub fn commands(&self) -> &'a [Command] {
        self.list.commands()
    }

    /// Number of commands.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.list.len()
    }

    /// Whether the list has no commands.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The live operands of command `index`: every slot a [`SlotUpdate`]
    /// can address on it, in the command's operand order, each paired with
    /// the value currently recorded. Empty when `index` is out of range.
    #[must_use]
    pub fn operands(&self, index: u32) -> Operands<'a> {
        Operands {
            command: self.list.commands.get(index as usize),
            index,
            next: 0,
        }
    }

    /// The value `slot` currently addresses; `None` when the slot's
    /// command does not exist or has no such operand.
    #[must_use]
    pub fn get(&self, slot: Slot) -> Option<OperandRef<'a>> {
        self.operands(slot.command)
            .find_map(|(s, value)| (s == slot).then_some(value))
    }
}

/// A command's operand, borrowed: the value one [`Slot`] currently
/// addresses.
///
/// The borrowed form of [`Operand`], read through a [`DisplayListView`]:
/// no recorded storage is cloned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OperandRef<'a> {
    /// A shape.
    Shape(&'a ShapeData),
    /// A paint.
    Paint(&'a Paint),
    /// A stroke style.
    Stroke(&'a Stroke),
    /// A shadow.
    Shadow(&'a Shadow),
    /// A transform.
    Transform(&'a Affine),
    /// A group style.
    Group(&'a Group),
    /// A rectangle.
    Rect(&'a Rect),
    /// A glyph run.
    Run(&'a GlyphRun),
}

impl OperandRef<'_> {
    /// Which operand this value is.
    #[must_use]
    pub const fn kind(&self) -> OperandKind {
        match self {
            Self::Shape(_) => OperandKind::Shape,
            Self::Paint(_) => OperandKind::Paint,
            Self::Stroke(_) => OperandKind::Stroke,
            Self::Shadow(_) => OperandKind::Shadow,
            Self::Transform(_) => OperandKind::Transform,
            Self::Group(_) => OperandKind::Group,
            Self::Rect(_) => OperandKind::Rect,
            Self::Run(_) => OperandKind::Run,
        }
    }
}

impl From<OperandRef<'_>> for Operand {
    fn from(value: OperandRef<'_>) -> Self {
        match value {
            OperandRef::Shape(shape) => Self::Shape(shape.clone()),
            OperandRef::Paint(paint) => Self::Paint(paint.clone()),
            OperandRef::Stroke(stroke) => Self::Stroke(stroke.clone()),
            OperandRef::Shadow(shadow) => Self::Shadow(*shadow),
            OperandRef::Transform(transform) => Self::Transform(*transform),
            OperandRef::Group(group) => Self::Group(*group),
            OperandRef::Rect(rect) => Self::Rect(*rect),
            OperandRef::Run(run) => Self::Run(run.clone()),
        }
    }
}

/// The operand count a command exposes, in [`Operands`] order.
const fn operand_count(command: &Command) -> u8 {
    match command {
        Command::Fill { .. } | Command::Shadow { .. } | Command::Glyphs { .. } => 2,
        Command::Stroke { .. } => 3,
        Command::Image { .. }
        | Command::Picture { .. }
        | Command::BeginClip { .. }
        | Command::BeginTransform { .. }
        | Command::BeginGroup { .. } => 1,
        Command::End => 0,
    }
}

/// The live operands of one command.
///
/// Yielded by [`DisplayListView::operands`]: every [`Slot`] the command
/// exposes — the operand kinds [`DisplayList::apply`] accepts for it — each
/// with the value currently recorded.
#[derive(Debug)]
pub struct Operands<'a> {
    command: Option<&'a Command>,
    index: u32,
    next: u8,
}

impl<'a> Iterator for Operands<'a> {
    type Item = (Slot, OperandRef<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let (kind, value) = match (self.command?, self.next) {
            (
                Command::Fill { shape, .. }
                | Command::Stroke { shape, .. }
                | Command::Shadow { shape, .. }
                | Command::BeginClip { shape, .. },
                0,
            ) => (OperandKind::Shape, OperandRef::Shape(shape)),
            (Command::Fill { paint, .. } | Command::Glyphs { paint, .. }, 1)
            | (Command::Stroke { paint, .. }, 2) => (OperandKind::Paint, OperandRef::Paint(paint)),
            (Command::Stroke { stroke, .. }, 1) => {
                (OperandKind::Stroke, OperandRef::Stroke(stroke))
            }
            (Command::Shadow { shadow, .. }, 1) => {
                (OperandKind::Shadow, OperandRef::Shadow(shadow))
            }
            (Command::Glyphs { run, .. }, 0) => (OperandKind::Run, OperandRef::Run(run)),
            (Command::Image { dst, .. }, 0) => (OperandKind::Rect, OperandRef::Rect(dst)),
            (Command::Picture { transform, .. } | Command::BeginTransform { transform, .. }, 0) => {
                (OperandKind::Transform, OperandRef::Transform(transform))
            }
            (Command::BeginGroup { group, .. }, 0) => {
                (OperandKind::Group, OperandRef::Group(group))
            }
            _ => return None,
        };
        self.next += 1;
        Some((
            Slot {
                command: self.index,
                operand: kind,
            },
            value,
        ))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.command.map_or(0, |command| {
            usize::from(operand_count(command)) - usize::from(self.next)
        });
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for Operands<'_> {}

/// Immutable recorded content, shared by reference: cloning is cheap, and a
/// picture can be sent to and shared between threads.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Picture(Arc<DisplayList>);

impl Picture {
    /// A picture over a finished list.
    #[must_use]
    pub fn from_list(list: DisplayList) -> Self {
        Self(Arc::new(list))
    }

    pub(crate) fn take_unique_list(&mut self) -> Option<DisplayList> {
        Arc::get_mut(&mut self.0).map(std::mem::take)
    }

    pub(crate) fn put_unique_list(&mut self, list: DisplayList) {
        *Arc::get_mut(&mut self.0).expect("picture must be unique") = list;
    }

    /// Clears the picture for storage reuse, returning `true` only when
    /// this is the last reference: a shared picture is left intact and
    /// its storage stays with the other references.
    pub fn try_recycle(&mut self) -> bool {
        let Some(list) = Arc::get_mut(&mut self.0) else {
            return false;
        };
        list.clear();
        true
    }

    /// The recorded commands.
    #[must_use]
    pub fn display_list(&self) -> &DisplayList {
        &self.0
    }

    /// The list for applying updates, cloned first while still shared.
    pub(crate) fn list_mut(&mut self) -> &mut DisplayList {
        Arc::make_mut(&mut self.0)
    }

    /// Applies slot updates and returns the commands to regenerate, cloning
    /// the shared list first if another reference still holds it.
    pub fn apply(&mut self, updates: impl IntoIterator<Item = SlotUpdate>) -> Dirty {
        self.list_mut().apply(updates)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "serde")]
    use super::ScopeError;
    #[cfg(feature = "serde")]
    use serde_json::json;

    use super::{
        Command, Dirty, DisplayList, Operand, OperandKind, OperandRef, Picture, Slot, SlotUpdate,
    };
    use crate::glyph::{FontId, Glyph, GlyphRun, GlyphStyle};
    use crate::paint::Paint;

    fn run(id: u32) -> GlyphRun {
        GlyphRun {
            font: FontId::new(0),
            size: 12.0,
            coords: Vec::new().into(),
            glyphs: vec![Glyph {
                id,
                x: 0.0,
                y: 0.0,
                transform: None,
            }]
            .into(),
            style: GlyphStyle::Fill,
        }
    }

    #[test]
    fn a_run_update_dirties_only_its_command() {
        let mut list = DisplayList::default();
        list.push(Command::Fill {
            shape: crate::shape::ShapeData::of(&kurbo::Rect::new(0.0, 0.0, 1.0, 1.0)),
            paint: Paint::from(crate::color::WorkingColor::WHITE),
        });
        list.push(Command::Glyphs {
            run: run(1),
            paint: Paint::from(crate::color::WorkingColor::WHITE),
        });
        list.push(Command::Fill {
            shape: crate::shape::ShapeData::of(&kurbo::Rect::new(0.0, 0.0, 2.0, 2.0)),
            paint: Paint::from(crate::color::WorkingColor::WHITE),
        });
        let dirty = list.apply([SlotUpdate {
            command: 1,
            value: Operand::Run(run(2)),
        }]);
        assert_eq!(dirty.ranges(), std::iter::once(1..2).collect::<Vec<_>>());
        let Command::Glyphs { run: got, .. } = &list.commands()[1] else {
            panic!("not a glyph run");
        };
        assert_eq!(got.glyphs[0].id, 2);
    }

    fn viewed_list() -> DisplayList {
        use crate::shape::ShapeData;
        use crate::style::Group;
        use kurbo::{Affine, Rect};

        let mut list = DisplayList::default();
        let begin = list.push(Command::BeginTransform {
            transform: Affine::translate((2., 3.)),
            end: 0,
        });
        list.push(Command::Fill {
            shape: ShapeData::Rect(Rect::new(0., 0., 1., 1.)),
            paint: Paint::Solid(crate::color::WorkingColor::WHITE),
        });
        list.push(Command::Glyphs {
            run: run(1),
            paint: Paint::Solid(crate::color::WorkingColor::BLACK),
        });
        list.push(Command::BeginGroup {
            group: Group::new().opacity(0.5),
            end: 0,
        });
        let clip = list.push(Command::BeginClip {
            shape: ShapeData::Rect(Rect::new(0., 0., 4., 4.)),
            end: 0,
        });
        list.end(clip);
        list.end(begin);
        list
    }

    #[test]
    fn the_view_lists_each_commands_live_operands_with_current_values() {
        use crate::shape::ShapeData;
        use kurbo::{Affine, Rect};

        let list = viewed_list();
        let view = list.view();
        assert_eq!(view.commands().len(), 7);
        assert_eq!(view.len(), 7);
        assert!(!view.is_empty());

        // A fill's live operands: its shape and its paint.
        let operands: Vec<_> = view.operands(1).collect();
        assert_eq!(operands.len(), 2);
        assert_eq!(
            operands[0].0,
            Slot {
                command: 1,
                operand: OperandKind::Shape,
            }
        );
        assert_eq!(
            operands[0].1,
            OperandRef::Shape(&ShapeData::Rect(Rect::new(0., 0., 1., 1.)))
        );
        assert_eq!(operands[0].1.kind(), operands[0].0.operand);
        assert_eq!(
            operands[1].0,
            Slot {
                command: 1,
                operand: OperandKind::Paint,
            }
        );

        // A transform scope's only live operand is its transform.
        let operands: Vec<_> = view.operands(0).collect();
        assert_eq!(
            operands,
            [(
                Slot {
                    command: 0,
                    operand: OperandKind::Transform,
                },
                OperandRef::Transform(&Affine::translate((2., 3.))),
            )]
        );

        // `End` exposes no operand, and neither does a missing command.
        assert_eq!(view.operands(5).count(), 0);
        assert_eq!(view.operands(10).count(), 0);
    }

    #[test]
    fn the_view_resolves_slots_and_reads_applied_updates() {
        use crate::style::Group;

        let mut list = viewed_list();
        // `get` resolves a slot to the recorded value, and misses a slot
        // the command does not expose.
        assert_eq!(
            list.view().get(Slot {
                command: 3,
                operand: OperandKind::Group,
            }),
            Some(OperandRef::Group(&Group::new().opacity(0.5)))
        );
        assert!(
            list.view()
                .get(Slot {
                    command: 3,
                    operand: OperandKind::Paint,
                })
                .is_none()
        );

        // An applied update reads through a later view.
        list.apply([SlotUpdate {
            command: 1,
            value: Operand::Paint(Paint::Solid(crate::color::WorkingColor::BLACK)),
        }]);
        assert_eq!(
            list.view().get(Slot {
                command: 1,
                operand: OperandKind::Paint,
            }),
            Some(OperandRef::Paint(&Paint::Solid(
                crate::color::WorkingColor::BLACK
            )))
        );
    }

    #[test]
    fn union_merges_two_dirty_sets() {
        let mut a = Dirty::from_unsorted(vec![0..2, 5..7]);
        let b = Dirty::from_unsorted(vec![2..5, 9..10]);
        a.union(b);
        assert_eq!(a.ranges(), &[0..7, 9..10]);
    }

    #[cfg(feature = "serde")]
    fn scopes(commands: &serde_json::Value) -> Result<DisplayList, String> {
        serde_json::from_value(json!({ "commands": commands })).map_err(|error| error.to_string())
    }

    #[test]
    #[cfg(feature = "serde")]
    fn deserialization_rejects_a_scope_whose_recorded_end_is_wrong() {
        let group =
            json!({ "opacity": 1.0, "blend": "Normal", "blend_space": "Linear", "filter": null });
        let error = scopes(&json!([
            { "BeginGroup": { "group": group, "end": 0 } },
            "End"
        ]))
        .expect_err("the recorded end points at the Begin itself");
        assert!(
            error.contains(
                &ScopeError::WrongEnd {
                    begin: 0,
                    recorded: 0,
                    actual: 1
                }
                .to_string()
            ),
            "{error}"
        );
    }

    #[test]
    #[cfg(feature = "serde")]
    fn deserialization_rejects_unbalanced_scopes() {
        let unmatched = scopes(&json!(["End"])).expect_err("an End with no scope");
        assert!(
            unmatched.contains(&ScopeError::UnmatchedEnd { index: 0 }.to_string()),
            "{unmatched}"
        );
    }

    #[test]
    fn clearing_a_unique_picture_keeps_its_command_buffer() {
        let mut list = DisplayList::with_capacity(2);
        list.push(Command::End);
        let mut picture = Picture::from_list(list);
        let shared = picture.clone();
        let pointer = picture.display_list().commands().as_ptr();

        assert!(!picture.try_recycle());
        assert_eq!(shared.display_list().len(), 1);
        drop(shared);
        assert!(picture.try_recycle());
        assert!(picture.display_list().is_empty());
        assert_eq!(picture.display_list().commands().as_ptr(), pointer);
    }
}
