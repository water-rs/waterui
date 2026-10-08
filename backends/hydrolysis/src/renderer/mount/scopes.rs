//! [`RetainedScopes`]: the scope state a node's last record pushed around
//! its descendants (spec §B.3).
//!
//! A record's `push_*` calls append to [`RetainedScopes::pending`]; the
//! matching `pop_*` removes them again. Pushes still open while a nested
//! record starts are copied into [`RetainedScopes::pushes`] — the state a
//! partial descent must replay at `enter`/`exit` when a descendant
//! re-records without its ancestors re-recording (commit 4). Transient
//! pushes that never spanned a child record are never stored.

use std::cell::RefCell;
use std::rc::Rc;

/// One scope a record held open while a descendant recorded.
#[derive(Clone)]
pub enum ScopePush {
    /// A `push_render_owner` frame — renders and joins the accessibility
    /// owner chain.
    RenderOwner(Rc<()>),
    /// A `push_input_owner` frame — input ancestry only.
    InputOwner(Rc<()>),
    /// An `OnKeyPress` scope — the bubble chain for targets below.
    KeyHandler(Rc<RefCell<OnKeyPress>>),
    /// An accessibility parent the record pushed (`parent_stack`).
    #[cfg(feature = "accessibility")]
    A11yParent(AccessibilityNodeId),
    /// A `push_suppression` the record held open around its subtree.
    #[cfg(feature = "accessibility")]
    A11ySuppression,
}

/// The scope state a node pushed at its last record: `pending` collects
/// pushes made during the record (pops remove them again); `pushes` keeps
/// the ones descendants recorded under — the replay state.
#[derive(Default)]
pub struct RetainedScopes {
    /// Pushes currently open inside the active record.
    pending: Vec<ScopePush>,
    /// Pushes live while any descendant record ran — the ancestor state a
    /// partial descent replays.
    pushes: Vec<ScopePush>,
}

impl RetainedScopes {
    /// Starts a fresh record: pending and replayed pushes reset.
    pub fn begin_record(&mut self) {
        self.pending.clear();
        self.pushes.clear();
    }

    /// The record just ended a scope it pushed earlier.
    fn pop<T>(&mut self, matches: impl Fn(&ScopePush) -> Option<T>) {
        if let Some(position) = self
            .pending
            .iter()
            .rposition(|push| matches(push).is_some())
        {
            self.pending.remove(position);
        }
    }

    /// A descendant's record is starting under this node: every push still
    /// open is ancestor state for it — keep them all.
    pub fn freeze_for_descendant(&mut self) {
        for push in &self.pending {
            if !self.pushes.iter().any(|stored| stored.same_scope(push)) {
                self.pushes.push(push.clone());
            }
        }
    }
}

impl ScopePush {
    /// Identity match for dedup — two pushes are the same scope when they
    /// carry the same payload.
    fn same_scope(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::RenderOwner(a), Self::RenderOwner(b))
            | (Self::InputOwner(a), Self::InputOwner(b)) => Rc::ptr_eq(a, b),
            (Self::KeyHandler(a), Self::KeyHandler(b)) => Rc::ptr_eq(a, b),
            #[cfg(feature = "accessibility")]
            (Self::A11yParent(a), Self::A11yParent(b)) => a == b,
            #[cfg(feature = "accessibility")]
            (Self::A11ySuppression, Self::A11ySuppression) => true,
            _ => false,
        }
    }
}

/// Recording the scope pushes the record made — called from the
/// `push_*`/`pop_*` helpers on `SemanticCore` and `HydrolysisRenderer` while
/// a node records.
impl RetainedScopes {
    pub fn push_render_owner(&mut self, owner: &Rc<()>) {
        self.pending.push(ScopePush::RenderOwner(Rc::clone(owner)));
    }
    pub fn pop_render_owner(&mut self) {
        self.pop(|push| matches!(push, ScopePush::RenderOwner(_)).then_some(()));
    }
    pub fn push_input_owner(&mut self, owner: &Rc<()>) {
        self.pending.push(ScopePush::InputOwner(Rc::clone(owner)));
    }
    pub fn pop_input_owner(&mut self) {
        self.pop(|push| matches!(push, ScopePush::InputOwner(_)).then_some(()));
    }
    pub fn push_key_handler(&mut self, handler: &Rc<RefCell<OnKeyPress>>) {
        self.pending.push(ScopePush::KeyHandler(Rc::clone(handler)));
    }
    pub fn pop_key_handler(&mut self) {
        self.pop(|push| matches!(push, ScopePush::KeyHandler(..)).then_some(()));
    }
    #[cfg(feature = "accessibility")]
    pub fn push_a11y_parent(&mut self, id: AccessibilityNodeId) {
        self.pending.push(ScopePush::A11yParent(id));
    }
    #[cfg(feature = "accessibility")]
    pub fn pop_a11y_parent(&mut self) {
        self.pop(|push| matches!(push, ScopePush::A11yParent(_)).then_some(()));
    }
    #[cfg(feature = "accessibility")]
    pub fn push_a11y_suppression(&mut self) {
        self.pending.push(ScopePush::A11ySuppression);
    }
    #[cfg(feature = "accessibility")]
    pub fn pop_a11y_suppression(&mut self) {
        self.pop(|push| matches!(push, ScopePush::A11ySuppression).then_some(()));
    }
}

#[cfg(feature = "accessibility")]
use accesskit::NodeId as AccessibilityNodeId;
use waterui_core::key::OnKeyPress;
