//! Runtime-owned routing. Callback identities never keep a mount alive.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::{Rc, Weak},
};

pub struct SceneRegistry<T>(Rc<Registry<T>>);

struct Registry<T> {
    next: Cell<u64>,
    mounts: RefCell<BTreeMap<u64, Weak<T>>>,
}

impl<T> Clone for SceneRegistry<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Default for SceneRegistry<T> {
    fn default() -> Self {
        Self(Rc::new(Registry {
            next: Cell::new(1),
            mounts: RefCell::default(),
        }))
    }
}

impl<T> core::fmt::Debug for SceneRegistry<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SceneRegistry")
            .field("mount_count", &self.0.mounts.borrow().len())
            .finish()
    }
}

impl<T> SceneRegistry<T> {
    pub fn register(&self, state: T) -> Registration<T> {
        let id = self.0.next.get();
        self.0
            .next
            .set(id.checked_add(1).expect("scene identity exhausted"));
        let state = Rc::new(state);
        self.0.mounts.borrow_mut().insert(id, Rc::downgrade(&state));
        Registration {
            registry: Rc::downgrade(&self.0),
            id,
            state,
        }
    }

    pub fn get(&self, id: u64) -> Option<Rc<T>> {
        self.0.mounts.borrow().get(&id).and_then(Weak::upgrade)
    }
}

pub struct Registration<T> {
    registry: Weak<Registry<T>>,
    pub id: u64,
    pub state: Rc<T>,
}

impl<T> Drop for Registration<T> {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.mounts.borrow_mut().remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SceneRegistry;
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn mount_routes_are_isolated_and_late_callbacks_cannot_reach_replacements() {
        let registry = SceneRegistry::default();
        let first = registry.register(Cell::new(10));
        let second = registry.register(Cell::new(20));
        let first_id = first.id;
        let first_lifetime = Rc::downgrade(&first.state);
        registry.get(first_id).unwrap().set(11);
        assert_eq!(registry.get(second.id).unwrap().get(), 20);
        drop(first);
        assert!(first_lifetime.upgrade().is_none());
        assert!(registry.get(first_id).is_none());
        let replacement = registry.register(Cell::new(30));
        assert_ne!(replacement.id, first_id);
        assert!(registry.get(first_id).is_none());
        assert_eq!(registry.get(second.id).unwrap().get(), 20);
        drop(second);
        drop(replacement);
        assert!(registry.0.mounts.borrow().is_empty());
    }
}
