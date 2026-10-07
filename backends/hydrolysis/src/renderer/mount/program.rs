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
}

impl PartialEq for ItemKey {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Node(a), Self::Node(b)) => Rc::ptr_eq(a, b),
            (Self::Scope(a), Self::Scope(b)) => a == b,
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

/// One entry of a layer's ordered content.
pub enum Item {
    Run(Recording),
    Node(Rc<NodeCell>),
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

/// A material wrapper's backdrop request.
pub struct MaterialRequest {
    pub runtime: Rc<crate::renderer::material::MaterialRuntime>,
    pub bounds: kurbo::Rect,
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
