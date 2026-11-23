//! Shared allocations retained by picture grids and image snapshots.

use std::{ptr::NonNull, sync::Arc};

#[derive(Default)]
pub(in crate::render) struct Memory {
    allocations: rustc_hash::FxHashSet<NonNull<()>>,
}

impl Memory {
    pub fn source(&mut self, picture: &cherenkov::Picture) -> u64 {
        let list = picture.display_list();
        if !self.allocations.insert(NonNull::from(list).cast()) {
            return 0;
        }
        // The Arc allocation holds its counters and the display list itself.
        (2 * size_of::<usize>() + size_of_val(list)) as u64
            + list.heap_bytes(|command| match command {
                cherenkov::Command::Picture { picture, .. } => self.source(picture),
                command => cherenkov::lowering::memory::command_bytes(command),
            })
    }

    pub fn cell(&mut self, cell: &Arc<[i32; 2]>) -> u64 {
        if self.allocations.insert(NonNull::from(cell.as_ref()).cast()) {
            (2 * size_of::<usize>() + size_of::<[i32; 2]>()) as u64
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cherenkov::{
        Draw, Picture, WorkingColor,
        kurbo::{Affine, Rect},
    };

    #[test]
    fn shared_and_nested_sources_count_once() {
        let leaf = Picture::record(|c| {
            c.fill(Rect::new(0.0, 0.0, 8.0, 8.0), WorkingColor::new([1.0; 4]));
        });
        let parent = Picture::record(|c| {
            c.picture(&leaf, Affine::IDENTITY);
            c.picture(&leaf, Affine::translate((8.0, 0.0)));
        });
        let mut memory = Memory::default();
        let leaf_bytes = memory.source(&leaf);
        let parent_bytes = memory.source(&parent);
        assert!(leaf_bytes > 0);
        assert!(parent_bytes > 0);
        assert_eq!(memory.source(&leaf.clone()), 0);
        assert_eq!(memory.source(&parent.clone()), 0);
        assert_eq!(Memory::default().source(&parent), parent_bytes + leaf_bytes);
        // An equal recording still has a separate allocation, including when
        // the previous recording survives only in an image snapshot.
        let replacement = Picture::from_list(parent.display_list().clone());
        let replacement_bytes = Memory::default().source(&replacement) - leaf_bytes;
        assert!(replacement_bytes > 0);
        assert_eq!(memory.source(&replacement), replacement_bytes);
    }

    #[test]
    fn grid_and_snapshot_share_cell_storage() {
        let cell = Arc::new([0, 0]);
        let snapshot = Arc::clone(&cell);
        let mut memory = Memory::default();
        assert!(memory.cell(&cell) > 0);
        assert_eq!(memory.cell(&snapshot), 0);
        assert!(memory.cell(&Arc::new([0, 0])) > 0);
    }
}
