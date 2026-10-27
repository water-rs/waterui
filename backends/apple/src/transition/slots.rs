//! Collection transition slots (§4): one ordered sequence mixing live
//! member ids with the ghosts their removals left behind, plus the frozen
//! placement a ghost's layout slot is built from. Pure data — no views —
//! so the whole file unit-tests without a window.
//!
//! A ghost never holds an index. Each [`SlotSequence::reconcile`]
//! re-anchors every ghost to the nearest live id that precedes it and
//! survives into the new order — or to the head when none does — so a
//! ghost follows its anchor through any later membership edit, and an id
//! coming back while its ghost still runs joins as a fresh live member.

use alloc::vec::Vec;
use core::hash::Hash;
use std::collections::{HashMap, HashSet};

use waterui_core::layout::ViewDimensions;
use waterui_core::transition::GhostSlot;

/// A removal ghost's identity inside a [`SlotSequence`] — unique per
/// departed member, so a reinserted id never collides with the ghost its
/// earlier instance left.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct GhostKey(u64);

/// One sequence member: a live collection id or a held removal ghost.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot<Id> {
    /// A live member — the collection id `AnyViews` answers.
    Live(Id),
    /// A removal ghost occupying its frozen slot until completion.
    Ghost(GhostKey),
}

/// The ordered members layout sees: live ids in collection order with
/// each ghost spliced in after its anchor — the nearest preceding live
/// id, or the sequence head.
pub struct SlotSequence<Id> {
    /// Live ids and ghosts in member order.
    slots: Vec<Slot<Id>>,
    /// The next ghost key — monotone, never reused.
    next_key: u64,
}

/// What a [`SlotSequence::reconcile`] changed: the departed ids — each
/// with a ghost key when the caller asked to hold a slot for it — and the
/// ids new to the live set, which join as structural insertions.
pub struct Reconciled<Id> {
    /// Departed ids in their old member order — `Some` key when the
    /// member left a ghost behind.
    pub departed: Vec<(Id, Option<GhostKey>)>,
    /// Ids new to the live set, in the new order.
    pub inserted: Vec<Id>,
}

impl<Id: Copy + Eq + Hash> SlotSequence<Id> {
    /// A sequence holding only live ids — no ghosts yet.
    pub fn new(initial: impl IntoIterator<Item = Id>) -> Self {
        Self {
            slots: initial.into_iter().map(Slot::Live).collect(),
            next_key: 0,
        }
    }

    /// Applies `next` as the new live order.
    ///
    /// `wants_ghost` decides, per departed id, whether a ghost replaces
    /// it in place. Every ghost — old and new — re-anchors to the nearest
    /// preceding live id that survives into `next`, or to the head when
    /// none does, so positions are never stored and later edits rebase
    /// ghosts for free. The rebuilt sequence is: head ghosts in their
    /// previous relative order, then for each id of `next`, that id
    /// followed by its anchored ghosts in their previous relative order —
    /// Hydrolysis's `slot_order`, so an insertion at the head lands after
    /// the head ghosts. A surviving id that merely moves is neither
    /// departed nor inserted — moves play nothing.
    pub fn reconcile(
        &mut self,
        next: &[Id],
        mut wants_ghost: impl FnMut(Id) -> bool,
    ) -> Reconciled<Id> {
        let next_set: HashSet<Id> = next.iter().copied().collect();
        let mut departed = Vec::new();
        let mut head_ghosts: Vec<GhostKey> = Vec::new();
        let mut anchored: HashMap<Id, Vec<GhostKey>> = HashMap::new();
        // The nearest live id seen so far that survives into `next` —
        // the anchor a departed member or a re-anchored ghost takes.
        let mut anchor = None;
        let old = core::mem::take(&mut self.slots);
        let mut push_ghost = |anchor: Option<Id>, key: GhostKey| match anchor {
            Some(id) => anchored.entry(id).or_default().push(key),
            None => head_ghosts.push(key),
        };
        for &slot in &old {
            match slot {
                Slot::Live(id) if next_set.contains(&id) => anchor = Some(id),
                Slot::Live(id) => {
                    let key = wants_ghost(id).then(|| {
                        let key = GhostKey(self.next_key);
                        self.next_key += 1;
                        key
                    });
                    departed.push((id, key));
                    if let Some(key) = key {
                        push_ghost(anchor, key);
                    }
                }
                Slot::Ghost(key) => push_ghost(anchor, key),
            }
        }
        let live: HashSet<Id> = old
            .iter()
            .filter_map(|slot| match slot {
                Slot::Live(id) => Some(*id),
                Slot::Ghost(_) => None,
            })
            .collect();
        let mut slots = Vec::with_capacity(old.len());
        // A ghost with no surviving live id ahead of it leads the sequence.
        slots.extend(head_ghosts.iter().map(|key| Slot::Ghost(*key)));
        let mut inserted = Vec::new();
        for &id in next {
            slots.push(Slot::Live(id));
            if !live.contains(&id) {
                inserted.push(id);
            }
            if let Some(keys) = anchored.get(&id) {
                slots.extend(keys.iter().map(|key| Slot::Ghost(*key)));
            }
        }
        self.slots = slots;
        Reconciled { departed, inserted }
    }

    /// Drops a ghost whose transition completed. Panics when the key is
    /// unknown — a completion firing twice or a key from another sequence
    /// is a bug, not an event.
    pub fn remove_ghost(&mut self, key: GhostKey) {
        let position = self
            .slots
            .iter()
            .position(|slot| matches!(slot, Slot::Ghost(k) if *k == key))
            .expect("transition ghost key is not a member of this sequence");
        self.slots.remove(position);
    }

    /// The member order layout sees.
    pub fn iter(&self) -> impl Iterator<Item = Slot<Id>> + '_ {
        self.slots.iter().copied()
    }
}

impl<Id: Copy + Eq + Hash> core::fmt::Debug for SlotSequence<Id> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SlotSequence").finish_non_exhaustive()
    }
}

/// What a container last placed for a live member: the measurement the
/// placement pass negotiated, with the size replaced by the frame the
/// member actually landed in. A ghost's slot is built from this — never
/// re-measured, so a mid-flight size change moves the slot's origin but
/// not its size (§4.2).
#[derive(Clone, Debug)]
pub struct FrozenPlacement {
    /// `measure()` at the pass's own proposal, size replaced by the
    /// placed frame size.
    pub dimensions: ViewDimensions,
    /// The member's layout priority at that pass.
    pub priority: i32,
}

impl FrozenPlacement {
    /// The fixed-size layout member the ghost stands in as — core's
    /// `GhostSlot`, reporting exactly the frozen placement under every
    /// proposal.
    pub fn ghost_slot(&self) -> GhostSlot {
        GhostSlot::new(self.dimensions.clone(), self.priority)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sequence's slots as a comparable vector.
    fn slots(sequence: &SlotSequence<char>) -> Vec<Slot<char>> {
        sequence.iter().collect()
    }

    /// Ghosts every departure asks for.
    fn all(_id: char) -> bool {
        true
    }

    #[test]
    fn ghost_stays_after_its_anchor() {
        let mut sequence = SlotSequence::new(['a', 'b', 'c']);
        let reconciled = sequence.reconcile(&['a', 'c'], all);
        let key = reconciled.departed[0].1.expect("b left a ghost");
        assert_eq!(
            slots(&sequence),
            vec![Slot::Live('a'), Slot::Ghost(key), Slot::Live('c')]
        );
        // A later edit that keeps the anchor keeps the ghost behind it.
        let reconciled = sequence.reconcile(&['a', 'c', 'd'], all);
        assert_eq!(reconciled.departed, Vec::new());
        assert_eq!(
            slots(&sequence),
            vec![
                Slot::Live('a'),
                Slot::Ghost(key),
                Slot::Live('c'),
                Slot::Live('d')
            ]
        );
    }

    #[test]
    fn ghost_reanchors_when_anchor_departs() {
        let mut sequence = SlotSequence::new(['a', 'b', 'c', 'd']);
        let reconciled = sequence.reconcile(&['a', 'b', 'd'], all);
        let ghost_c = reconciled.departed[0].1.expect("c left a ghost");
        // Removing 'b' without a ghost re-anchors 'c's ghost to 'a'.
        let reconciled = sequence.reconcile(&['a', 'd'], |_| false);
        assert_eq!(reconciled.departed, vec![('b', None)]);
        assert_eq!(
            slots(&sequence),
            vec![Slot::Live('a'), Slot::Ghost(ghost_c), Slot::Live('d')]
        );
    }

    #[test]
    fn head_ghosts_stay_first() {
        let mut sequence = SlotSequence::new(['a', 'b', 'c']);
        let reconciled = sequence.reconcile(&['b', 'c'], all);
        let ghost_a = reconciled.departed[0].1.expect("a left a ghost");
        assert_eq!(
            slots(&sequence),
            vec![Slot::Ghost(ghost_a), Slot::Live('b'), Slot::Live('c')]
        );
        // 'b' ghosts too: both head ghosts keep their removal order.
        let reconciled = sequence.reconcile(&['c'], all);
        let ghost_b = reconciled.departed[0].1.expect("b left a ghost");
        assert_eq!(
            slots(&sequence),
            vec![Slot::Ghost(ghost_a), Slot::Ghost(ghost_b), Slot::Live('c')]
        );
        // An insertion at the head lands after the head ghosts.
        let reconciled = sequence.reconcile(&['d', 'c'], all);
        assert_eq!(reconciled.inserted, vec!['d']);
        assert_eq!(
            slots(&sequence),
            vec![
                Slot::Ghost(ghost_a),
                Slot::Ghost(ghost_b),
                Slot::Live('d'),
                Slot::Live('c')
            ]
        );
    }

    #[test]
    fn reinserted_id_is_a_new_live_entry() {
        let mut sequence = SlotSequence::new(['a', 'b', 'c']);
        let reconciled = sequence.reconcile(&['a', 'c'], all);
        let ghost_b = reconciled.departed[0].1.expect("b left a ghost");
        // 'b' comes back mid-flight: a fresh live member, while the old
        // ghost keeps its anchor and finishes independently.
        let reconciled = sequence.reconcile(&['a', 'b', 'c'], all);
        assert_eq!(reconciled.inserted, vec!['b']);
        assert_eq!(reconciled.departed, Vec::new());
        assert_eq!(
            slots(&sequence),
            vec![
                Slot::Live('a'),
                Slot::Ghost(ghost_b),
                Slot::Live('b'),
                Slot::Live('c')
            ]
        );
        sequence.remove_ghost(ghost_b);
        assert_eq!(
            slots(&sequence),
            vec![Slot::Live('a'), Slot::Live('b'), Slot::Live('c')]
        );
    }

    #[test]
    fn moves_play_nothing() {
        let mut sequence = SlotSequence::new(['a', 'b', 'c']);
        let reconciled = sequence.reconcile(&['c', 'b', 'a'], all);
        assert_eq!(
            (reconciled.departed, reconciled.inserted),
            (Vec::new(), Vec::new())
        );
        assert_eq!(
            slots(&sequence),
            vec![Slot::Live('c'), Slot::Live('b'), Slot::Live('a')]
        );
    }

    #[test]
    #[should_panic(expected = "transition ghost key is not a member")]
    fn remove_ghost_panics_on_unknown_key() {
        let mut sequence = SlotSequence::new(['a']);
        sequence.remove_ghost(GhostKey(u64::MAX));
    }
}
