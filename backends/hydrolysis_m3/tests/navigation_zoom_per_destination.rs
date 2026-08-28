//! A matched transition names a pair that differs per destination.
//!
//! A stack declares one transition for every push, which cannot name a matched
//! pair: a gallery's tile and the page it opens share an identity the next tile
//! does not. The repository's own navigation example shows the consequence —
//! one `zoom(id)` on the whole stack, one of six tiles marked as its source, so
//! only that tile could ever zoom. A destination declares its own now.

use std::cell::Cell;
use std::rc::Rc;

use waterui::navigation::{
    NativeNavigationTransition, NavigationLink, NavigationPath, NavigationStack,
    NavigationTransition, NavigationTransitionDirection, NavigationTransitionFrame,
    NavigationTransitionViewExt as _, NavigationView, RetainedNavigationTransition,
    navigation_transition,
};
use waterui::prelude::*;
use waterui_core::id::Id;
use waterui_testing::UiBuilder;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Tile(i32);

fn tile_id(index: i32) -> Id {
    Id::try_from(index + 1).expect("a matched transition id must be non-zero")
}

/// A transition that records having been asked for a frame.
///
/// This is the discriminator: whether the renderer honoured what the
/// *destination* declared is otherwise invisible — navigation succeeds either
/// way, and the difference between two motions does not reach the
/// accessibility tree. Resolving frames is something only a `Frames` transition
/// is asked to do, so a non-zero count is proof the destination's own
/// declaration was the one used rather than the stack's.
#[derive(Debug, Clone)]
struct CountingTransition {
    frames: Rc<Cell<usize>>,
}

impl NavigationTransition for CountingTransition {
    fn frame(
        &self,
        _progress: f32,
        _direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame {
        self.frames.set(self.frames.get() + 1);
        NavigationTransitionFrame::IDENTITY
    }

    fn native(&self) -> Option<NativeNavigationTransition> {
        None
    }

    fn retained(&self) -> RetainedNavigationTransition {
        RetainedNavigationTransition::Frames
    }
}

fn tile(index: i32) -> impl View {
    NavigationLink::value(text(format!("Tile {index}")), Tile(index))
        .navigation_transition_source(tile_id(index))
}

/// The stack declares a transition that resolves no frames; only the second
/// tile's destination declares one that does.
fn gallery(frames: Rc<Cell<usize>>) -> impl View {
    let counting = CountingTransition { frames };
    NavigationStack::with_path(
        NavigationPath::<Tile>::new(),
        vstack((tile(0), tile(1))).title("Gallery"),
    )
    .destination(move |Tile(index)| {
        let page = NavigationView::new(
            format!("Photo {index}"),
            text(format!("photo {index} content"))
                .navigation_transition_destination(tile_id(index)),
        );
        if index == 1 {
            page.transition(counting.clone())
        } else {
            page
        }
    })
    .transition(navigation_transition::automatic())
}

/// Opening a tile uses the transition its own destination declared.
///
/// A stack declares one transition for every push, which cannot name a matched
/// pair: a gallery's tile and the page it opens share an identity the next tile
/// does not. The repository's own navigation example shows the consequence —
/// one `zoom(id)` on the whole stack, one of six tiles marked as its source, so
/// only that tile could ever zoom.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 400))]
fn a_destination_transition_wins_over_the_stack(ui: UiBuilder) {
    let frames = Rc::new(Cell::new(0usize));
    let mut app = ui.mount({
        let frames = Rc::clone(&frames);
        move || gallery(Rc::clone(&frames))
    });
    app.settle();

    let second = app.query().label("Tile 1").single().bounds();
    let (x, y) = second.center();
    app.tap_at(x, y);
    app.settle();

    let _ = app.query().label("photo 1 content").single();
    assert!(
        frames.get() > 0,
        "the destination declared its own transition, so the renderer must have \
         asked that one to resolve frames rather than the stack's"
    );
}
