//! The heartbeat's decision pipeline end-to-end on host: the same
//! `observe::install()` the app installs, fed the exact events the
//! planner emits (`cherenkov::planes` DEBUG `plane decision`), must
//! report the verdict — and a candidate that was never offered (its
//! display layer still probing `readyForDisplay`) reads `unseen`.

use cherenkov::LayerId;

use apple_planes::observe;

/// Stand-in for the `Ineligible` debug payload the planner emits as
/// `decision = ?why` — Debug renders the variant name and payload.
#[derive(Debug)]
struct TranslucentAbove(#[expect(dead_code, reason = "read through the Debug impl")] LayerId);

#[test]
fn the_planners_decision_reaches_the_heartbeat() {
    let decisions = observe::install();

    // The emission shape at gpu/src/render/mod.rs: a verdict every
    // planned frame for each offered candidate — DEBUG on the
    // `cherenkov::planes` target.
    tracing::debug!(target: "cherenkov::planes", layer = ?LayerId::new(2), decision = "promoted", "plane decision");
    tracing::debug!(target: "cherenkov::planes", layer = ?LayerId::new(3), decision = ?TranslucentAbove(LayerId::new(7)), "plane decision");

    assert_eq!(decisions.decision(2), "promoted");
    assert_eq!(decisions.decision(3), "TranslucentAbove(LayerId(7))");
    assert_eq!(decisions.decision(4), "unseen");
}
