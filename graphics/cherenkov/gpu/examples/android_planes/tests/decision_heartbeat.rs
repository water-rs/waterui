//! The heartbeat's decision pipeline end-to-end on host: the same
//! `observe::install()` the APK installs, fed the exact events the
//! planner emits (`cherenkov::planes` DEBUG `external frame
//! eligibility` + `plane decision`), must report a non-`unseen`
//! decision — the round-6 device failure where every layer read
//! `unseen` because `Forward`'s `enabled` disabled the callsites
//! globally for `Capture` too.

use cherenkov::LayerId;

use android_planes::observe;

/// Stand-in for the `Ineligible` debug payload the planner emits as
/// `decision = ?why` — Debug renders the bare variant name.
#[derive(Debug)]
struct NoOverlayUsage;

#[test]
fn the_planners_decision_reaches_the_heartbeat() {
    let decisions = observe::install();

    // The emission shape at gpu/src/render/mod.rs: an eligibility
    // record at install, then a verdict every planned frame — all
    // DEBUG on the `cherenkov::planes` target.
    tracing::debug!(target: "cherenkov::planes", layer = ?LayerId::new(2), reason = ?None::<&str>, "external frame eligibility");
    tracing::debug!(target: "cherenkov::planes", layer = ?LayerId::new(2), decision = "promoted", "plane decision");
    tracing::debug!(target: "cherenkov::planes", layer = ?LayerId::new(3), decision = ?NoOverlayUsage, "plane decision");

    assert_eq!(decisions.decision(2), "promoted");
    assert_eq!(decisions.decision(3), "NoOverlayUsage");
    assert_eq!(decisions.decision(4), "unseen");
}
