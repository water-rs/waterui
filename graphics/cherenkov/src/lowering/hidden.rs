//! The layers a lowering walk skips, shared by both backends.
//!
//! A layer that [shows nothing](crate::LayerNode::shows_nothing) changes no
//! pixel of its parent, so the walk skips it with everything beneath it:
//! its content, its children and their backdrop samples. A backdrop group
//! whose every member lies in a skipped layer is not captured either.
//!
//! A hidden layer is still walked when something beneath it is needed
//! outside it: the capture point — the anchor, or the first member of an
//! unanchored group — of a group a drawn member samples, or anything the
//! backend reports with [`HiddenLayers::keep`] (a promoted plane, a
//! projected image, an external frame). Skipping it then would change what
//! the rest of the surface draws.

use rustc_hash::FxHashSet;

use crate::LayerId;

/// What a hidden layer holds that the walk may need outside it.
#[derive(Clone, Copy, Debug)]
enum Need {
    /// Always: the backend reported it with [`HiddenLayers::keep`].
    Always,
    /// The capture point of this group.
    Capture(u64),
    /// This anchor layer, the capture point of its groups.
    Anchor(LayerId),
}

/// The hidden layers a planning walk meets, and which of them the drawing
/// walk skips.
///
/// The planning walk reports each layer with [`Self::enter`] and
/// [`Self::leave`], every backdrop member with [`Self::member`] and every
/// anchor with [`Self::anchor`]; [`Self::resolve`] then decides, and the
/// drawing walk asks [`Self::skips`] and [`Self::captures`].
#[derive(Debug, Default)]
pub struct HiddenLayers {
    /// The hidden layers open on the planning walk, outermost first.
    open: Vec<LayerId>,
    /// Every hidden layer the planning walk entered.
    hidden: Vec<LayerId>,
    /// What each hidden layer holds that may be needed outside it.
    needs: Vec<(LayerId, Need)>,
    /// Every backdrop member: its group and its innermost hidden ancestor,
    /// itself included.
    members: Vec<(u64, Option<LayerId>)>,
    /// The hidden layers the drawing walk still walks.
    needed: FxHashSet<LayerId>,
    /// The hidden layers the drawing walk skips.
    skipped: FxHashSet<LayerId>,
    /// The groups a drawn member samples.
    captured: FxHashSet<u64>,
}

impl HiddenLayers {
    /// Forgets the last walk, keeping the allocations.
    pub fn clear(&mut self) {
        self.open.clear();
        self.hidden.clear();
        self.needs.clear();
        self.members.clear();
        self.needed.clear();
        self.skipped.clear();
        self.captured.clear();
    }

    /// The bytes the retained allocations hold.
    #[must_use]
    pub fn heap_bytes(&self) -> u64 {
        let layers = (self.open.capacity() + self.hidden.capacity()) * size_of::<LayerId>()
            + (self.needed.capacity() + self.skipped.capacity()) * size_of::<LayerId>()
            + self.needs.capacity() * size_of::<(LayerId, Need)>()
            + self.members.capacity() * size_of::<(u64, Option<LayerId>)>()
            + self.captured.capacity() * size_of::<u64>();
        layers as u64
    }

    /// The planning walk enters layer `id`, which shows nothing when
    /// `hides`. Every layer entered is left with [`Self::leave`] once its
    /// children are planned.
    pub fn enter(&mut self, id: LayerId, hides: bool) {
        if hides {
            self.open.push(id);
            self.hidden.push(id);
        }
    }

    /// The planning walk leaves layer `id`.
    pub fn leave(&mut self, id: LayerId) {
        if self.open.last() == Some(&id) {
            self.open.pop();
        }
    }

    /// Whether a hidden layer is open at this point of the planning walk:
    /// only then can [`Self::keep`] change what the drawing walk skips.
    #[must_use]
    pub const fn in_hidden(&self) -> bool {
        !self.open.is_empty()
    }

    /// What the walk plans at this point is needed whether or not it shows:
    /// every hidden layer around it is walked.
    pub fn keep(&mut self) {
        self.needs
            .extend(self.open.iter().map(|&layer| (layer, Need::Always)));
    }

    /// A member of group `group` is planned at this point; it is the
    /// group's capture point when `captures_here`.
    pub fn member(&mut self, group: u64, captures_here: bool) {
        self.members.push((group, self.open.last().copied()));
        if captures_here {
            self.needs
                .extend(self.open.iter().map(|&layer| (layer, Need::Capture(group))));
        }
    }

    /// Anchor layer `id` is planned at this point.
    pub fn anchor(&mut self, id: LayerId) {
        self.needs
            .extend(self.open.iter().map(|&layer| (layer, Need::Anchor(id))));
    }

    /// Decides which hidden layers the drawing walk skips and which groups
    /// it captures, once the planning walk has ended; a walk resolves once,
    /// after [`Self::clear`]. `links` lists every anchor with each group it
    /// anchors.
    pub fn resolve(&mut self, links: &[(LayerId, u64)]) {
        debug_assert!(self.open.is_empty(), "the planning walk left every layer");
        debug_assert!(
            self.needed.is_empty() && self.skipped.is_empty() && self.captured.is_empty(),
            "a walk resolves once, after `clear`"
        );
        if self.hidden.is_empty() {
            return;
        }
        self.needed.extend(
            self.needs
                .iter()
                .filter(|(_, need)| matches!(need, Need::Always))
                .map(|&(layer, _)| layer),
        );
        // A walked layer can draw a member whose capture point lies in
        // another hidden layer, which must then be walked too: the needed
        // set grows until it holds every capture point a drawn member
        // reads.
        loop {
            let needed = &self.needed;
            self.captured.extend(
                self.members
                    .iter()
                    .filter(|(_, hidden)| hidden.is_none_or(|layer| needed.contains(&layer)))
                    .map(|&(group, _)| group),
            );
            let before = self.needed.len();
            for &(layer, need) in &self.needs {
                let reads = match need {
                    Need::Always => false,
                    Need::Capture(group) => self.captured.contains(&group),
                    Need::Anchor(anchor) => links
                        .iter()
                        .any(|&(at, group)| at == anchor && self.captured.contains(&group)),
                };
                if reads {
                    self.needed.insert(layer);
                }
            }
            if self.needed.len() == before {
                break;
            }
        }
        let needed = &self.needed;
        self.skipped.extend(
            self.hidden
                .iter()
                .copied()
                .filter(|layer| !needed.contains(layer)),
        );
    }

    /// Whether the drawing walk skips layer `id` with everything beneath
    /// it.
    #[must_use]
    pub fn skips(&self, id: LayerId) -> bool {
        !self.skipped.is_empty() && self.skipped.contains(&id)
    }

    /// Whether the drawing walk captures group `group`: some drawn member
    /// samples it.
    #[must_use]
    pub fn captures(&self, group: u64) -> bool {
        self.hidden.is_empty() || self.captured.contains(&group)
    }
}

#[cfg(test)]
mod tests {
    use super::HiddenLayers;
    use crate::LayerId;

    fn layer(raw: u64) -> LayerId {
        LayerId::new(raw)
    }

    #[test]
    fn a_walk_without_hidden_layers_skips_nothing_and_captures_every_group() {
        let mut hidden = HiddenLayers::default();
        hidden.enter(layer(1), false);
        hidden.member(7, true);
        hidden.leave(layer(1));
        hidden.resolve(&[]);
        assert!(!hidden.skips(layer(1)));
        assert!(hidden.captures(7));
    }

    #[test]
    fn a_member_under_a_hidden_layer_is_skipped_with_its_capture() {
        let mut hidden = HiddenLayers::default();
        hidden.enter(layer(1), true);
        hidden.enter(layer(2), false);
        hidden.member(7, true);
        hidden.leave(layer(2));
        hidden.leave(layer(1));
        hidden.resolve(&[]);
        assert!(hidden.skips(layer(1)));
        assert!(!hidden.captures(7));
    }

    #[test]
    fn a_hidden_capture_point_a_drawn_member_reads_is_walked() {
        let mut hidden = HiddenLayers::default();
        hidden.enter(layer(1), true);
        hidden.member(7, true);
        hidden.leave(layer(1));
        hidden.enter(layer(2), false);
        hidden.member(7, false);
        hidden.leave(layer(2));
        hidden.resolve(&[]);
        assert!(!hidden.skips(layer(1)));
        assert!(hidden.captures(7));
    }

    #[test]
    fn a_hidden_anchor_a_drawn_member_reads_is_walked() {
        let mut hidden = HiddenLayers::default();
        hidden.enter(layer(1), true);
        hidden.anchor(layer(3));
        hidden.leave(layer(1));
        hidden.member(7, false);
        hidden.resolve(&[(layer(3), 7)]);
        assert!(!hidden.skips(layer(1)));
        assert!(hidden.captures(7));
    }

    #[test]
    fn a_kept_layer_walks_its_hidden_ancestors_and_the_captures_its_members_read() {
        // Layer 1 holds group 7's capture point; layer 2 must be walked and
        // draws a member of group 7, so layer 1 is walked as well.
        let mut hidden = HiddenLayers::default();
        hidden.enter(layer(1), true);
        hidden.member(7, true);
        hidden.leave(layer(1));
        hidden.enter(layer(2), true);
        hidden.enter(layer(4), true);
        hidden.keep();
        hidden.member(7, false);
        hidden.leave(layer(4));
        hidden.leave(layer(2));
        hidden.enter(layer(5), true);
        hidden.leave(layer(5));
        hidden.resolve(&[]);
        assert!(!hidden.skips(layer(1)));
        assert!(!hidden.skips(layer(2)));
        assert!(!hidden.skips(layer(4)));
        assert!(hidden.skips(layer(5)));
        assert!(hidden.captures(7));
    }
}
