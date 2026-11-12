//! The live motion of one list row: the swipe and reorder displacement of
//! its layer, and the dismiss background a swipe uncovers.
//!
//! A row's displacement is an operand its layer follows, and the dismiss
//! background's one value is the swipe's progress, so a swipe tracking the
//! finger, springing back or a reorder drag sliding rows aside moves live
//! operands and records no row again. Only a swipe or a lift beginning or
//! ending changes what the list lists.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use cherenkov_record::Recorder;
use kurbo::{Affine, Rect, Vec2};
use waterui_core::animation::Animation;
use waterui_core::interaction::InteractionState;

use crate::renderer::chrome::LiveSource;
use crate::renderer::recording::{LoweredResources, SceneResources};
use crate::theme::live::Operand;
use crate::theme::{Change, CustomPart, Inputs, RealizedCustom};

/// A swipe's progress toward the dismiss threshold: `-1.0..=1.0`, negative
/// toward the physical left edge.
fn swipe_progress(offset: f64, width: f64, threshold: f64) -> f64 {
    let distance = (width * threshold).max(f64::EPSILON);
    (offset / distance).clamp(-1.0, 1.0)
}

/// What a row keeps across renders to move live.
pub struct RowMotion {
    /// The row layer's displacement from its resting place.
    layer: Operand<Affine>,
    /// The swipe's progress the dismiss background reads.
    progress: Cell<f64>,
    /// Whether the row has been swiped: its dismiss background is listed
    /// from then on, until the row leaves the visible window.
    swiping: Cell<bool>,
    /// Whether the row is lifted by a reorder drag.
    lifted: Cell<bool>,
    dismiss: RefCell<Option<Dismiss>>,
}

impl RowMotion {
    pub fn new() -> Self {
        Self {
            layer: Operand::new(Affine::IDENTITY),
            progress: Cell::new(0.0),
            swiping: Cell::new(false),
            lifted: Cell::new(false),
            dismiss: RefCell::new(None),
        }
    }

    /// The operand the row's layer follows.
    pub fn layer(&self) -> Operand<Affine> {
        self.layer.clone()
    }

    /// Moves the row to `offset` from its resting place, animated with
    /// `animation` or snapped when `None`.
    pub fn displace(&self, offset: Vec2, animation: Option<&Animation>) {
        self.layer.set(Affine::translate(offset), animation);
    }

    /// The row follows a swipe that has travelled `offset` across a row
    /// `width` wide, dismissing past `threshold` of it.
    pub fn track_swipe(&self, offset: f64, width: f64, threshold: f64) {
        self.swiping.set(true);
        self.displace(Vec2::new(offset, 0.0), None);
        self.set_progress(swipe_progress(offset, width, threshold), None);
    }

    /// The row springs back to rest under `animation`.
    pub fn settle(&self, animation: &Animation) {
        self.displace(Vec2::ZERO, Some(animation));
        self.set_progress(0.0, Some(animation));
    }

    /// The row snaps to rest.
    pub fn rest(&self) {
        self.displace(Vec2::ZERO, None);
        self.set_progress(0.0, None);
    }

    /// Whether the row's dismiss background is listed.
    pub fn swiping(&self) -> bool {
        self.swiping.get()
    }

    /// Whether a reorder drag lifts the row.
    pub fn lifted(&self) -> bool {
        self.lifted.get()
    }

    pub fn set_lifted(&self, lifted: bool) {
        self.lifted.set(lifted);
    }

    fn set_progress(&self, progress: f64, animation: Option<&Animation>) {
        self.progress.set(progress);
        if let Some(dismiss) = &*self.dismiss.borrow() {
            dismiss.source.retarget(progress, animation);
        }
    }

    /// The source listing `part` over `frame`: the one already realized
    /// when it realizes the same part over the same frame, otherwise a
    /// fresh one at the swipe's current progress.
    pub fn dismiss_source(
        &self,
        part: &Rc<dyn CustomPart<f64>>,
        frame: Rect,
    ) -> Rc<dyn LiveSource> {
        let mut dismiss = self.dismiss.borrow_mut();
        let held = dismiss
            .as_ref()
            .filter(|held| std::ptr::addr_eq(Rc::as_ptr(&held.part), Rc::as_ptr(part)))
            .filter(|held| held.source.frame == frame);
        if held.is_none() {
            *dismiss = Some(Dismiss {
                part: Rc::clone(part),
                source: DismissSource::new(part.as_ref(), frame, self.progress.get()),
            });
        }
        Rc::<DismissSource>::clone(
            &dismiss
                .as_ref()
                .expect("the dismiss was just ensured")
                .source,
        )
    }
}

/// A row's realized dismiss background.
struct Dismiss {
    part: Rc<dyn CustomPart<f64>>,
    source: Rc<DismissSource>,
}

/// The dismiss background realized over a frame: its recording, and the
/// operands it reads.
struct DismissSource {
    part: RealizedCustom<f64>,
    frame: Rect,
    progress: Cell<f64>,
    /// The subscriptions to the signals the part's values follow.
    guards: RefCell<Vec<Box<dyn Any>>>,
}

impl DismissSource {
    fn new(part: &dyn CustomPart<f64>, frame: Rect, progress: f64) -> Rc<Self> {
        let part = part.realize(&Inputs::at_rest(
            frame,
            InteractionState::empty(),
            &progress,
        ));
        let source = Rc::new(Self {
            part,
            frame,
            progress: Cell::new(progress),
            guards: RefCell::new(Vec::new()),
        });
        let mut guards: Vec<Box<dyn Any>> = Vec::new();
        for driven in &source.part.drivens {
            let weak: Weak<Self> = Rc::downgrade(&source);
            let target = Rc::downgrade(driven);
            let notify: Rc<dyn Fn(Option<Animation>)> =
                Rc::new(move |animation: Option<Animation>| {
                    if let (Some(source), Some(target)) = (weak.upgrade(), target.upgrade()) {
                        let progress = source.progress.get();
                        target.retarget(
                            &Inputs::at_rest(source.frame, InteractionState::empty(), &progress),
                            Change::Animated(animation.as_ref()),
                        );
                    }
                });
            if let Some(guard) = driven.subscribe(notify) {
                guards.push(guard);
            }
        }
        *source.guards.borrow_mut() = guards;
        source
    }

    /// The swipe's progress became `progress`: the part's operands move
    /// under `animation`, or snap when `None`.
    fn retarget(&self, progress: f64, animation: Option<&Animation>) {
        self.progress.set(progress);
        let inputs = Inputs::at_rest(self.frame, InteractionState::empty(), &progress);
        for driven in &self.part.drivens {
            driven.retarget(&inputs, Change::Animated(animation));
        }
    }
}

impl LiveSource for DismissSource {
    fn record(
        &self,
        recorder: &mut Recorder,
        _resources: &SceneResources,
    ) -> Option<LoweredResources> {
        self.part.record(recorder, self.frame);
        None
    }
}
