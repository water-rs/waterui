use core::cell::RefCell;
use std::collections::BTreeSet;

use gtk4::prelude::*;

struct Entry {
    id: i32,
    object: glib::BoxedAnyObject,
}

/// A GTK list model reconciled by `WaterUI`'s semantic collection IDs.
///
/// Existing model objects survive insertions, removals, and moves, so GTK only
/// rebinds rows whose position or identity actually changed.
pub(super) struct KeyedModel {
    store: gtk4::gio::ListStore,
    entries: RefCell<Vec<Entry>>,
}

impl KeyedModel {
    pub(super) fn new() -> Self {
        Self {
            store: gtk4::gio::ListStore::new::<glib::BoxedAnyObject>(),
            entries: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn store(&self) -> gtk4::gio::ListStore {
        self.store.clone()
    }

    pub(super) fn reconcile(&self, ids: &[i32]) {
        let unique = ids.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(unique.len(), ids.len(), "GTK collection IDs must be unique");
        let mut entries = self.entries.borrow_mut();

        for (target, id) in ids.iter().copied().enumerate() {
            if entries.get(target).is_some_and(|entry| entry.id == id) {
                continue;
            }

            if let Some(source) = entries
                .iter()
                .enumerate()
                .skip(target + 1)
                .find_map(|(index, entry)| (entry.id == id).then_some(index))
            {
                let entry = entries.remove(source);
                self.store.remove(index_to_u32(source));
                self.store.insert(index_to_u32(target), &entry.object);
                entries.insert(target, entry);
            } else {
                let object = glib::BoxedAnyObject::new(id);
                self.store.insert(index_to_u32(target), &object);
                entries.insert(target, Entry { id, object });
            }
        }

        while entries.len() > ids.len() {
            let index = entries.len() - 1;
            entries.pop();
            self.store.remove(index_to_u32(index));
        }
    }
}

pub(super) fn list_item_id(item: &gtk4::ListItem) -> i32 {
    let object = item
        .item()
        .expect("GTK collection list item must own a model object")
        .downcast::<glib::BoxedAnyObject>()
        .expect("GTK collection model object must be BoxedAnyObject");
    *object.borrow::<i32>()
}

fn index_to_u32(index: usize) -> u32 {
    u32::try_from(index).expect("GTK collection index must fit in u32")
}
