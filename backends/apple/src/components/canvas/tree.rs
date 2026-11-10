//! The layer tree a canvas recording lowers to, kept in step with the
//! recording as it changes.
//!
//! The tree mirrors the display list: one [`Slot`] per command, in command
//! order, and each scope opener's layer holds the layers of the commands
//! directly inside its scope. A run of commands Core Graphics draws
//! together ([`plan::drawn_run`]) is one layer, in its first command's
//! slot. A recording change touches only what it names:
//!
//! - a slot update rewrites the layer of the command it addresses
//!   ([`Built::update`] sets a layer property where one carries the
//!   operand, and the layer is built again only when the operand changes
//!   what kind of layer realizes the command), or draws again the run
//!   holding it;
//! - a splice replaces the layers of the commands it removes and inserts,
//!   and the sublayer array of the scope that holds them; sibling layers
//!   outside the splice keep their identity and contents.
//!
//! A change that moves where runs begin or end — a group's blend mode
//! switching between a layer and a run, or a splice inside or into a
//! level with a run — lowers the whole list again.

use core::ops::Range;

use cocoa_ui::objc2::rc::Retained;
use cocoa_ui::objc2_foundation::NSArray;
use cocoa_ui::objc2_quartz_core::CALayer;
use waterui_graphics::draw::{Command, DisplayList, Edit, SlotUpdate, Splice};

use super::layers::{self, Built, Ctx, container};
use super::plan::{self, Node};

/// One command's layer.
struct Entry {
    /// What the layer was built from, to tell what an update changes.
    node: Node,
    built: Built,
    /// The layers of a picture command's frozen display list, kept so the
    /// mesh paints among them stay alive and can be shown.
    picture: Option<Tree>,
}

impl Entry {
    /// Shows the mesh paints the GPU has finished, here and in the
    /// picture below.
    fn show_completed(&self) {
        self.built.show_completed();
        if let Some(picture) = &self.picture {
            picture.show_completed();
        }
    }

    #[cfg(all(test, target_os = "macos"))]
    fn settle(&self) {
        self.built.settle();
        if let Some(picture) = &self.picture {
            picture.settle();
        }
    }
}

/// What realizes one command.
enum Slot {
    /// The `End` of a scope.
    End,
    /// A command the run that starts before it draws.
    Covered,
    /// A command with a layer of its own.
    Layer(Box<Entry>),
    /// The first command of a run, `len` commands long, that Core
    /// Graphics draws into `layer`.
    Run {
        len: usize,
        layer: Retained<CALayer>,
    },
}

/// The layers of one display list.
pub struct Tree {
    root: Retained<CALayer>,
    list: DisplayList,
    slots: Vec<Slot>,
    /// The runs the slots realize, by command range.
    runs: Vec<Range<usize>>,
}

impl core::fmt::Debug for Tree {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tree")
            .field("commands", &self.list.len())
            .field("runs", &self.runs)
            .finish_non_exhaustive()
    }
}

fn lower(command: &Command, ctx: Ctx<'_>) -> Entry {
    let node = plan::describe(command).expect("only a scope's End has no layer");
    let built = Built::new(&node, ctx);
    let picture = if let Node::Picture { picture, .. } = &node {
        // A shared picture is frozen: its layers hang under the command's
        // layer and nothing updates them again.
        let tree = Tree::new(picture.display_list(), ctx);
        built.add_sublayer(tree.root());
        Some(tree)
    } else {
        None
    };
    Entry {
        node,
        built,
        picture,
    }
}

/// The layer of `slot`, if it has one of its own.
const fn slot_layer(slot: &Slot) -> Option<&Retained<CALayer>> {
    match slot {
        Slot::Layer(entry) => Some(&entry.built.layer),
        Slot::Run { layer, .. } => Some(layer),
        Slot::End | Slot::Covered => None,
    }
}

/// Lowers the commands directly inside `scope`, and every scope below
/// them, into `slots`, and attaches their layers to `parent`.
fn build_scope(
    commands: &[Command],
    slots: &mut [Slot],
    scope: Option<usize>,
    parent: &CALayer,
    ctx: Ctx<'_>,
) {
    let run = if plan::is_level(commands, scope) {
        plan::drawn_run(commands, scope)
    } else {
        None
    };
    if let Some(run) = &run {
        slots[run.start] = Slot::Run {
            len: run.len(),
            layer: layers::run_layer(commands, run.clone(), ctx),
        };
    }
    for index in plan::children(commands, scope) {
        if run.as_ref().is_some_and(|run| run.contains(&index)) {
            continue;
        }
        build_command(commands, slots, index, ctx);
    }
    attach(commands, slots, scope, parent);
}

/// Lowers the command at `index` and, for a scope opener, its scope.
fn build_command(commands: &[Command], slots: &mut [Slot], index: usize, ctx: Ctx<'_>) {
    let entry = lower(&commands[index], ctx);
    let layer = entry.built.layer.clone();
    slots[index] = Slot::Layer(Box::new(entry));
    if let Some(end) = plan::scope_end(&commands[index]) {
        slots[end] = Slot::End;
        build_scope(commands, slots, Some(index), &layer, ctx);
    }
}

/// Sets the sublayers of `parent`, the layer of `scope`, to the layers of
/// the commands directly inside it.
fn attach(commands: &[Command], slots: &[Slot], scope: Option<usize>, parent: &CALayer) {
    let layers: Vec<Retained<CALayer>> = plan::children(commands, scope)
        .into_iter()
        .filter_map(|index| slot_layer(&slots[index]).cloned())
        .collect();
    let array = NSArray::from_retained_slice(&layers);
    // SAFETY: the array holds live layers, none of which is an ancestor of
    // `parent`.
    unsafe { parent.setSublayers(Some(&array)) };
}

impl Tree {
    /// The layers of `list`.
    #[must_use]
    pub fn new(list: &DisplayList, ctx: Ctx<'_>) -> Self {
        let mut tree = Self {
            root: container(),
            list: list.clone(),
            slots: Vec::new(),
            runs: Vec::new(),
        };
        tree.build(ctx);
        tree
    }

    /// Lowers the whole list under the root.
    fn build(&mut self, ctx: Ctx<'_>) {
        let commands = self.list.commands();
        self.slots = commands.iter().map(|_| Slot::Covered).collect();
        build_scope(commands, &mut self.slots, None, &self.root, ctx);
        self.runs = plan::runs(commands);
    }

    /// The layer to place in the host.
    #[must_use]
    pub const fn root(&self) -> &Retained<CALayer> {
        &self.root
    }

    /// The display list the tree is in step with.
    #[must_use]
    pub const fn list(&self) -> &DisplayList {
        &self.list
    }

    /// The layer of the scope opened at `scope` — the root for `None`.
    fn scope_layer(&self, scope: Option<usize>) -> Retained<CALayer> {
        scope.map_or_else(
            || self.root.clone(),
            |index| {
                slot_layer(&self.slots[index])
                    .expect("an attached scope has a layer")
                    .clone()
            },
        )
    }

    /// The run that draws command `index`, by its first command.
    fn run_of(&self, index: usize) -> Option<usize> {
        // A run's commands after its first are all covered.
        for start in (0..=index).rev() {
            match &self.slots[start] {
                Slot::Run { len, .. } => return (start + len > index).then_some(start),
                Slot::Covered => {}
                Slot::Layer(_) | Slot::End => return None,
            }
        }
        None
    }

    /// Applies `updates`, addressing this tree's list, to the layers they
    /// name.
    pub fn update(&mut self, updates: &[SlotUpdate], ctx: Ctx<'_>) {
        let _ = self.list.apply(updates.iter().cloned());
        let mut touched: Vec<usize> = updates.iter().map(|u| u.command as usize).collect();
        touched.sort_unstable();
        touched.dedup();
        let regrouped = touched
            .iter()
            .any(|&index| matches!(self.list.commands()[index], Command::BeginGroup { .. }));
        if regrouped && plan::runs(self.list.commands()) != self.runs {
            self.build(ctx);
            return;
        }
        let mut runs = Vec::new();
        for index in touched {
            match self.run_of(index) {
                Some(start) => runs.push(start),
                None => self.refresh(index, ctx),
            }
        }
        runs.dedup();
        for start in runs {
            let Slot::Run { len, layer } = &self.slots[start] else {
                unreachable!("a run starts at its Run slot");
            };
            layers::draw_run(layer, self.list.commands(), start..start + len, ctx);
        }
    }

    /// Brings the layer of command `index` up to its command.
    fn refresh(&mut self, index: usize, ctx: Ctx<'_>) {
        let node = plan::describe(&self.list.commands()[index])
            .expect("an update addresses a command with operands");
        let Slot::Layer(entry) = &mut self.slots[index] else {
            unreachable!("a command outside every run has a layer");
        };
        if entry.node == node {
            return;
        }
        if entry.built.update(&entry.node, &node, ctx) {
            entry.node = node;
            return;
        }
        // The operand changed which layer realizes the command.
        let old = core::mem::replace(&mut **entry, lower(&self.list.commands()[index], ctx));
        old.built.layer.removeFromSuperlayer();
        let commands = self.list.commands();
        if plan::scope_end(&commands[index]).is_some() {
            attach(
                commands,
                &self.slots,
                Some(index),
                &self.scope_layer(Some(index)),
            );
        }
        let parent = self.parent_scope(index);
        attach(commands, &self.slots, parent, &self.scope_layer(parent));
    }

    /// The scope opener that directly holds command `index`.
    fn parent_scope(&self, index: usize) -> Option<usize> {
        let commands = self.list.commands();
        (0..index)
            .rev()
            .find(|&open| plan::scope_end(&commands[open]).is_some_and(|end| end > index))
    }

    /// Applies a structural edit: the splices, then the slot updates that
    /// address the list they leave.
    pub fn edit(&mut self, edit: &Edit, ctx: Ctx<'_>) {
        for splice in &edit.splices {
            self.splice(splice, ctx);
        }
        self.update(&edit.updates, ctx);
    }

    fn splice(&mut self, splice: &Splice, ctx: Ctx<'_>) {
        let at = splice.at as usize;
        let remove = splice.remove as usize;
        let count = splice.insert.commands().len();
        // A splice inside a run, or at its first command, moves the run.
        let into_run = self.slots[at..at + remove]
            .iter()
            .chain(self.slots.get(at))
            .any(|slot| matches!(slot, Slot::Covered | Slot::Run { .. }));
        self.list.splice(splice);
        let shifted: Vec<Range<usize>> = self
            .runs
            .iter()
            .map(|run| {
                if run.start >= at + remove {
                    run.start - remove + count..run.end - remove + count
                } else {
                    run.clone()
                }
            })
            .collect();
        if into_run || plan::runs(self.list.commands()) != shifted {
            self.build(ctx);
            return;
        }
        self.runs = shifted;
        let inserted = (0..count).map(|_| Slot::Covered);
        for removed in self.slots.splice(at..at + remove, inserted) {
            if let Some(layer) = slot_layer(&removed) {
                layer.removeFromSuperlayer();
            }
        }
        let commands = self.list.commands();
        let mut index = at;
        while index < at + count {
            build_command(commands, &mut self.slots, index, ctx);
            index = plan::scope_end(&commands[index]).map_or(index + 1, |end| end + 1);
        }
        let parent = self.parent_scope(at);
        attach(commands, &self.slots, parent, &self.scope_layer(parent));
    }

    /// Shows the mesh paints the GPU has finished.
    pub fn show_completed(&self) {
        for slot in &self.slots {
            if let Slot::Layer(entry) = slot {
                entry.show_completed();
            }
        }
    }

    /// Waits for every mesh paint the GPU is still running and shows it:
    /// for a test that rasterizes the tree right away.
    #[cfg(all(test, target_os = "macos"))]
    pub fn settle(&self) {
        for slot in &self.slots {
            if let Slot::Layer(entry) = slot {
                entry.settle();
            }
        }
    }
}
