//! Backend receiver protocol for [`Dynamic`] views.

use nami::watcher::Context;
use waterui_core::Dynamic;
use waterui_core::backend::connect_dynamic;

/// Backend receiver for a [`Dynamic`] view's runtime content updates.
///
/// Backends implement this trait on the handle that owns or weakly references
/// their mounted dynamic host, then call [`DynamicNode::connect`] when
/// realizing [`NativeBackend::dynamic`](crate::NativeBackend::dynamic).
/// Application code never depends on `waterui-backend-core`, so the connection
/// protocol is not reachable from app-facing APIs.
pub trait DynamicNode<V: 'static>: Sized + 'static {
    /// Handles a view update delivered by the dynamic view's handler.
    ///
    /// Called synchronously during [`connect`](Self::connect) if a view was set
    /// before the backend realized the node, and on every subsequent update
    /// delivered through [`DynamicHandler`](waterui_core::dynamic::DynamicHandler).
    fn update(&self, update: Context<V>);

    /// Connects this node to `dynamic` so any pre-set view and every future
    /// update are delivered to [`update`](Self::update).
    fn connect(self, dynamic: Dynamic<V>) {
        connect_dynamic(dynamic, move |context| self.update(context));
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use nami::watcher::Context;
    use waterui_core::Dynamic;

    use super::DynamicNode;

    struct Collector(Rc<RefCell<Vec<u32>>>);

    impl DynamicNode<u32> for Collector {
        fn update(&self, update: Context<u32>) {
            self.0.borrow_mut().push(update.into_value());
        }
    }

    #[test]
    fn delivers_initial_and_subsequent_updates() {
        let (handler, dynamic) = Dynamic::new();
        handler.set(1);

        let received = Rc::new(RefCell::new(Vec::new()));
        Collector(Rc::clone(&received)).connect(dynamic);
        handler.set(2);
        handler.set(3);

        assert_eq!(&*received.borrow(), &[1, 2, 3]);
    }
}
