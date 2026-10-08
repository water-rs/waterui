//! A node's record: what it drew, in paint order, before it is lowered
//! into the node's retained layers at commit (§C).
//!
//! Every recording node owns one [`ProgramBuilder`] while it records:
//! drawing lands in the trailing [`Item::Run`], a child node appends an
//! [`Item::Node`], and a widget-internal clip or group opens a named
//! [`Item::Scope`]. The finished [`Program`] waits on the node until the
//! commit lowers it.

use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashSet;
use waterui_graphics::draw::ShapeData;

use super::cell::NodeCell;
use super::placement::Placement;
use crate::renderer::recording::Recording;

/// A widget-internal scope's identity: a literal role and an item (a row
/// id, a page identity, or `0`). Within one record a key is unique.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ScopeKey {
    pub role: &'static str,
    pub item: u64,
}

/// What a run sits directly beneath.
#[derive(Clone)]
pub enum ItemKey {
    Node(Rc<NodeCell>),
    Scope(ScopeKey),
    /// A `ChromeMaterial` member layer, identified by its material
    /// ordinal in the node's record (water-rs/waterui#1788).
    Chrome(u32),
}

impl PartialEq for ItemKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Node(a), Self::Node(b)) => Rc::ptr_eq(a, b),
            (Self::Scope(a), Self::Scope(b)) => a == b,
            (Self::Chrome(a), Self::Chrome(b)) => a == b,
            _ => false,
        }
    }
}

/// A run's identity: the item it is drawn beneath, or the tail of its list.
#[derive(Clone, PartialEq)]
pub enum RunKey {
    Before(ItemKey),
    Tail,
}

/// A scope's layer props, in the owning layer's space. The scope's hit
/// state lives on its placement instead — `open_scope_with` gates the
/// registrations inside through [`crate::renderer::mount::HitGate`],
/// never here.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeProps {
    pub transform: kurbo::Affine,
    pub clip: Option<ShapeData>,
    pub alpha: f32,
}

/// A `ChromeMaterial` render layer (water-rs/waterui#1788): one backdrop
/// material split out of a [`Content::record_layered`]
/// (`cherenkov_record::Content::record_layered`) recording — the member's
/// layer, drawn between the scene segment below it and the one above.
#[derive(Debug, Clone)]
pub struct ChromeMaterial {
    /// The material's ordinal in the node's record: its mount key under
    /// the node — a re-recorded chrome rebinds onto the member and the
    /// group its ordinal already mounts.
    pub ordinal: u32,
    /// The material: clip shape, backdrop shader, capture class, effect
    /// and material scope, all live.
    pub material: cherenkov_record::BackdropMaterial,
    /// The member's transform in its parent layer's space —
    /// `draw_context`'s `ctx.local`.
    pub transform: kurbo::Affine,
    /// Whether the member's ancestry presented this frame: a member
    /// under a fully transparent ancestry holds no membership.
    pub visible: bool,
}

/// One entry of a layer's ordered content.
pub enum Item {
    Run(Recording),
    Node(Rc<NodeCell>),
    Chrome(ChromeMaterial),
    Scope {
        key: ScopeKey,
        props: ScopeProps,
        /// The placement registrations inside the scope resolve through.
        placement: Rc<Placement>,
        items: Vec<Self>,
    },
}

impl Item {
    pub(crate) fn key(&self) -> Option<ItemKey> {
        match self {
            Self::Run(_) => None,
            Self::Node(cell) => Some(ItemKey::Node(Rc::clone(cell))),
            Self::Chrome(chrome) => Some(ItemKey::Chrome(chrome.ordinal)),
            Self::Scope { key, .. } => Some(ItemKey::Scope(*key)),
        }
    }
}

/// The scroll node's `inner` layer: viewport clip, scroll offset and the
/// content child (§E).
pub struct InnerProgram {
    pub viewport: kurbo::Rect,
    pub offset: kurbo::Vec2,
    pub items: Vec<Item>,
}

/// A producer-backed node's content, installed by the target at commit.
pub enum ProducerContent {
    Scene(SceneContentSource),
    Gpu {
        runtime: Rc<RefCell<crate::gpu_view::GpuContentRuntime>>,
        bounds: kurbo::Rect,
        visible: bool,
    },
    External {
        runtime: Rc<RefCell<crate::gpu_view::ExternalFrameRuntime>>,
        bounds: kurbo::Rect,
        visible: bool,
    },
}

/// A `SceneView`'s content: it records at commit against the target's
/// resources (§C).
pub struct SceneContentSource {
    pub content: Rc<RefCell<Box<dyn waterui_graphics::SceneContent>>>,
    pub invalidator: waterui_graphics::SceneInvalidator,
    pub association: Rc<RefCell<Option<std::rc::Weak<crate::renderer::recording::SceneResources>>>>,
    pub bounds: kurbo::Rect,
}

/// A material wrapper's backdrop request: the member's backdrop-group
/// terms and clip. The group it joins is keyed by `(scope, level,
/// scheme, install canvas)` — the canvas is resolved at commit — and one
/// group runs one key's chain (water-rs/waterui#1999).
#[derive(Clone, Copy)]
pub struct MaterialRequest {
    /// The nearest enclosing `.material_group()` node's identity — the
    /// address of its cell — or `None` for a group of the member's own.
    pub scope: Option<usize>,
    /// The member's within-window level — part of its backdrop-group key.
    pub level: crate::renderer::material::WithinWindowLevel,
    /// The member's resolved colour scheme at flush: a subtree may
    /// install its own scheme, so the scheme keys the backdrop group.
    pub scheme: waterui::theme::ColorScheme,
    /// The view's rect in its own space: the member's clip.
    pub bounds: kurbo::Rect,
    /// Whether the member presents this frame: an invisible member
    /// releases its membership instead of joining a group.
    pub visible: bool,
}

/// A node's finished record.
pub struct Program {
    pub opacity: f32,
    pub clip: Option<ShapeData>,
    pub filter: Option<Rc<RefCell<crate::renderer::effects::FilteredRuntime>>>,
    pub material: Option<MaterialRequest>,
    pub producer: Option<ProducerContent>,
    pub inner: Option<InnerProgram>,
    pub items: Vec<Item>,
}

impl Program {
    const fn new() -> Self {
        Self {
            opacity: 1.0,
            clip: None,
            filter: None,
            material: None,
            producer: None,
            inner: None,
            items: Vec::new(),
        }
    }
}

struct OpenScope {
    key: ScopeKey,
    props: ScopeProps,
    placement: Rc<Placement>,
    items: Vec<Item>,
    /// The record-space transform of the scope's layer origin, relative to
    /// the node's frame.
    origin: kurbo::Affine,
}

/// The recording target while a node records (`renderer.program`).
pub struct ProgramBuilder {
    cell: Rc<NodeCell>,
    program: Program,
    scopes: Vec<OpenScope>,
    inner_open: bool,
    /// The placement the inner list's children are ordered under (the
    /// scroll content placement, carrying the content offset).
    inner_anchor: Option<Rc<Placement>>,
    keys: FxHashSet<ScopeKey>,
    /// How many `ChromeMaterial` items the record pushed: the next
    /// material's ordinal.
    chromes: u32,
}

impl ProgramBuilder {
    pub(crate) fn new(cell: Rc<NodeCell>) -> Self {
        Self {
            cell,
            program: Program::new(),
            scopes: Vec::new(),
            inner_open: false,
            inner_anchor: None,
            keys: FxHashSet::default(),
            chromes: 0,
        }
    }

    pub(crate) const fn cell(&self) -> &Rc<NodeCell> {
        &self.cell
    }

    pub(crate) const fn program_mut(&mut self) -> &mut Program {
        &mut self.program
    }

    fn items_mut(&mut self) -> &mut Vec<Item> {
        if let Some(scope) = self.scopes.last_mut() {
            return &mut scope.items;
        }
        if self.inner_open {
            return &mut self
                .program
                .inner
                .as_mut()
                .expect("hydrolysis program: inner list open without an inner layer")
                .items;
        }
        &mut self.program.items
    }

    /// The trailing run, opened when the last item is not a run.
    pub(crate) fn run(&mut self) -> &mut Recording {
        let items = self.items_mut();
        if !matches!(items.last(), Some(Item::Run(_))) {
            items.push(Item::Run(Recording::new()));
        }
        match items.last_mut() {
            Some(Item::Run(recording)) => recording,
            _ => unreachable!("a run was just ensured"),
        }
    }

    /// Appends a child node's frame.
    pub(crate) fn push_node(&mut self, cell: Rc<NodeCell>) {
        self.items_mut().push(Item::Node(cell));
    }

    /// Appends a `ChromeMaterial` member: a backdrop material split out
    /// of the node's layered recording, drawn as a render layer between
    /// the scene segments recorded around it.
    pub(crate) fn push_chrome(
        &mut self,
        material: cherenkov_record::BackdropMaterial,
        transform: kurbo::Affine,
        visible: bool,
    ) {
        let ordinal = self.chromes;
        self.chromes += 1;
        self.items_mut().push(Item::Chrome(ChromeMaterial {
            ordinal,
            material,
            transform,
            visible,
        }));
    }

    /// Opens a named scope. A key repeated within one record panics.
    pub(crate) fn open_scope(
        &mut self,
        key: ScopeKey,
        props: ScopeProps,
        placement: Rc<Placement>,
    ) {
        assert!(
            self.keys.insert(key),
            "hydrolysis program: scope {key:?} opened twice in one record"
        );
        let origin = self.origin() * props.transform;
        self.scopes.push(OpenScope {
            key,
            props,
            placement,
            items: Vec::new(),
            origin,
        });
    }

    /// Drops the item recorded last, which must be a closed scope: a
    /// presentation that decides after recording that it is gone.
    pub(crate) fn discard_last_scope(&mut self) {
        match self.items_mut().pop() {
            Some(Item::Scope { key, .. }) => {
                self.keys.remove(&key);
            }
            _ => panic!("hydrolysis program: discard_last_scope with no scope recorded last"),
        }
    }

    pub(crate) fn close_scope(&mut self) {
        let mut scope = self
            .scopes
            .pop()
            .expect("hydrolysis program: close_scope without an open scope");
        // Runs record in the node's space; a scope layer placed by a
        // transform draws them in its own.
        if scope.origin != kurbo::Affine::IDENTITY {
            let into_scope = scope.origin.inverse();
            for item in &mut scope.items {
                if let Item::Run(run) = item {
                    let mut moved = Recording::new();
                    moved.append(run, into_scope);
                    *run = moved;
                }
            }
        }
        drop_empty_runs(&mut scope.items);
        self.items_mut().push(Item::Scope {
            key: scope.key,
            props: scope.props,
            placement: scope.placement,
            items: scope.items,
        });
    }

    /// Opens the scroll node's `inner` list: until [`Self::end_inner`],
    /// items land under the clipped, scrolled inner layer.
    pub(crate) fn begin_inner(
        &mut self,
        viewport: kurbo::Rect,
        offset: kurbo::Vec2,
        anchor: Rc<Placement>,
    ) {
        assert!(
            self.scopes.is_empty() && self.program.inner.is_none(),
            "hydrolysis program: the inner layer opens once, outside every scope"
        );
        self.program.inner = Some(InnerProgram {
            viewport,
            offset,
            items: Vec::new(),
        });
        self.inner_open = true;
        self.inner_anchor = Some(anchor);
    }

    pub(crate) fn end_inner(&mut self) {
        assert!(
            self.inner_open && self.scopes.is_empty(),
            "hydrolysis program: end_inner without an open inner layer"
        );
        self.inner_open = false;
        self.inner_anchor = None;
    }

    /// The record-space transform of the current item list's layer origin,
    /// relative to the node's frame.
    /// The opacity this program applies at the current record point: the
    /// node's own times every open scope's.
    pub(crate) fn alpha(&self) -> f32 {
        self.scopes
            .iter()
            .fold(self.program.opacity, |alpha, scope| {
                alpha * scope.props.alpha
            })
    }

    pub(crate) fn origin(&self) -> kurbo::Affine {
        self.scopes
            .last()
            .map_or(kurbo::Affine::IDENTITY, |scope| scope.origin)
    }

    /// The placement a child appended now is ordered under.
    pub(crate) fn anchor(&self) -> Rc<Placement> {
        if let Some(scope) = self.scopes.last() {
            return Rc::clone(&scope.placement);
        }
        Rc::clone(
            self.inner_anchor
                .as_ref()
                .unwrap_or_else(|| self.cell.placement()),
        )
    }

    /// The record-space transform, from the window, of the current item
    /// list's origin: the frame's world, the scroll offset when the inner
    /// list is open, and the open scope's origin.
    pub(crate) fn finish(self) -> Program {
        assert!(
            self.scopes.is_empty(),
            "hydrolysis program: {} scope(s) left open at the end of a record",
            self.scopes.len()
        );
        assert!(
            !self.inner_open,
            "hydrolysis program: the inner layer left open at the end of a record"
        );
        let mut program = self.program;
        drop_empty_runs(&mut program.items);
        if let Some(inner) = &mut program.inner {
            drop_empty_runs(&mut inner.items);
        }
        program
    }
}

/// Drops runs nothing drew into: an opened but unused run mounts no layer.
fn drop_empty_runs(items: &mut Vec<Item>) {
    items.retain(|item| !matches!(item, Item::Run(recording) if recording.is_empty()));
}
