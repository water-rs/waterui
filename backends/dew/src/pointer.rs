//! Retained pointer hit testing for Dew controls.

use core::cell::RefCell;
use std::rc::Rc;

use kurbo::{Point, Rect};
use waterui_core::Environment;

use crate::board::PointerSample;

/// Stateful control input bound to one retained hit-test target.
pub trait PointerHandler {
    fn pointer_down(&mut self, _point: Point, _bounds: Rect, _env: &Environment) -> bool {
        false
    }

    fn pointer_move(&mut self, _point: Point, _bounds: Rect, _env: &Environment) -> bool {
        false
    }

    fn pointer_up(&mut self, _point: Point, _bounds: Rect, _env: &Environment) -> bool {
        false
    }

    fn pointer_cancel(&mut self, _env: &Environment) -> bool {
        false
    }
}

/// Shareable identity for a retained pointer handler.
#[derive(Clone)]
pub struct PointerTargetHandle(Rc<RefCell<dyn PointerHandler>>);

impl PointerTargetHandle {
    pub(super) fn new(handler: impl PointerHandler + 'static) -> Self {
        Self(Rc::new(RefCell::new(handler)))
    }

    fn same_target(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

struct PointerTarget {
    bounds: Rect,
    handler: PointerTargetHandle,
}

struct ActivePointer {
    bounds: Rect,
    handler: PointerTargetHandle,
}

/// Routes a single board pointer stream to the topmost registered target.
#[derive(Default)]
pub struct PointerRouter {
    targets: Vec<PointerTarget>,
    active: Option<ActivePointer>,
}

impl core::fmt::Debug for PointerRouter {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PointerRouter")
            .field("targets", &self.targets.len())
            .field("active", &self.active.is_some())
            .finish()
    }
}

impl PointerRouter {
    pub(super) fn begin_frame(&mut self) {
        self.targets.clear();
    }

    pub(super) fn register(&mut self, bounds: Rect, handler: PointerTargetHandle) {
        self.targets.push(PointerTarget { bounds, handler });
    }

    pub(super) fn finish_frame(&mut self) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if let Some(target) = self
            .targets
            .iter()
            .rev()
            .find(|target| target.handler.same_target(&active.handler))
        {
            active.bounds = target.bounds;
        } else {
            self.active = None;
        }
    }

    pub(super) fn dispatch(&mut self, sample: PointerSample, env: &Environment) -> bool {
        let point = Point::new(sample.x, sample.y);
        match sample.phase {
            waterui_backend_core::input::TouchPhase::Started => {
                let mut changed = self.cancel_active(env);
                let Some(target) = self
                    .targets
                    .iter()
                    .rev()
                    .find(|target| target.bounds.contains(point))
                else {
                    return changed;
                };
                let active = ActivePointer {
                    bounds: target.bounds,
                    handler: target.handler.clone(),
                };
                changed |= active
                    .handler
                    .0
                    .borrow_mut()
                    .pointer_down(point, active.bounds, env);
                self.active = Some(active);
                changed
            }
            waterui_backend_core::input::TouchPhase::Moved => {
                self.active.as_ref().is_some_and(|active| {
                    active
                        .handler
                        .0
                        .borrow_mut()
                        .pointer_move(point, active.bounds, env)
                })
            }
            waterui_backend_core::input::TouchPhase::Ended => {
                let Some(active) = self.active.take() else {
                    return false;
                };
                active
                    .handler
                    .0
                    .borrow_mut()
                    .pointer_up(point, active.bounds, env)
            }
            waterui_backend_core::input::TouchPhase::Cancelled => self.cancel_active(env),
        }
    }

    fn cancel_active(&mut self, env: &Environment) -> bool {
        let Some(active) = self.active.take() else {
            return false;
        };
        active.handler.0.borrow_mut().pointer_cancel(env)
    }
}
