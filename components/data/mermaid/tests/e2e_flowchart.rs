//! What a rendered flowchart owes its reader.
//!
//! The assertions here are all about text, which is the point: a Mermaid
//! diagram drawn as one opaque picture would pass a "did anything get painted"
//! check and fail every one of these. Each node label, each edge label and each
//! subgraph title has to reach the accessibility tree as its own node, at the
//! position the geometry put it.

use hydrolysis_m3::install;
use waterui_mermaid::mermaid;
use waterui_testing::{OffscreenApp, Role, ui as test_ui};

const DECISION: &str = "\
flowchart TD
    A[Start] --> B{Ready?}
    B -->|yes| C([Go])
    B -->|no| D[(Wait)]
    D --> A
";

const SUBGRAPHS: &str = "\
flowchart LR
    subgraph Ingest
        A[Read] --> B[Parse]
    end
    subgraph Serve
        C[Render]
    end
    B --> C
";

fn app(source: &'static str) -> OffscreenApp {
    test_ui()
        .viewport(800, 600)
        .theme(install)
        .mount_offscreen(move || mermaid(source))
}

#[core::prelude::v1::test]
fn every_node_label_is_its_own_accessible_node() {
    let mut app = app(DECISION);
    for label in ["Start", "Ready?", "Go", "Wait"] {
        app.query().role(Role::LABEL).label(label).assert_exists();
    }
}

#[core::prelude::v1::test]
fn edge_labels_are_accessible_too() {
    let mut app = app(DECISION);
    for label in ["yes", "no"] {
        app.query().role(Role::LABEL).label(label).assert_exists();
    }
}

#[core::prelude::v1::test]
fn subgraph_titles_are_accessible() {
    let mut app = app(SUBGRAPHS);
    for label in ["Ingest", "Serve", "Read", "Parse", "Render"] {
        app.query().role(Role::LABEL).label(label).assert_exists();
    }
}

/// A `flowchart TD` puts its first node above the ones it points at, and a
/// `flowchart LR` puts it to their left. Asserting on the labels' own bounds is
/// how we know the geometry reached the views rather than every label piling up
/// at the origin.
#[core::prelude::v1::test]
fn layout_direction_reaches_the_placed_labels() {
    let mut down = app(DECISION);
    let start = down
        .query()
        .role(Role::LABEL)
        .label("Start")
        .single()
        .bounds();
    let ready = down
        .query()
        .role(Role::LABEL)
        .label("Ready?")
        .single()
        .bounds();
    assert!(
        start.y() + start.height() <= ready.y(),
        "`flowchart TD` must stack downward: Start {start:?} is not above Ready? {ready:?}"
    );

    let mut across = app(SUBGRAPHS);
    let read = across
        .query()
        .role(Role::LABEL)
        .label("Read")
        .single()
        .bounds();
    let parse = across
        .query()
        .role(Role::LABEL)
        .label("Parse")
        .single()
        .bounds();
    assert!(
        read.x() + read.width() <= parse.x(),
        "`flowchart LR` must flow rightward: Read {read:?} is not left of Parse {parse:?}"
    );
}

/// The whole reason this crate installs its own text measurer: a label has to
/// fit inside the box layout reserved for it. If the two disagreed — boxes sized
/// from one set of font metrics, glyphs drawn from another — the longer a label
/// is, the further its bounds would spill past its node.
#[core::prelude::v1::test]
fn labels_fit_the_boxes_that_were_measured_for_them() {
    const WIDTHS: &str = "\
flowchart TD
    A[i] --> B[a considerably longer label than the one above it]
";
    let mut app = test_ui()
        .viewport(900, 400)
        .theme(install)
        .mount_offscreen(|| mermaid(WIDTHS));

    let short = app.query().role(Role::LABEL).label("i").single().bounds();
    let long = app
        .query()
        .role(Role::LABEL)
        .label("a considerably longer label than the one above it")
        .single()
        .bounds();

    assert!(
        long.width() > short.width(),
        "the longer label should measure wider: {long:?} vs {short:?}"
    );
    // Both labels are centred in their node, and both nodes are centred on the
    // same column, so their centres agree to within rounding.
    assert!(
        (long.center().0 - short.center().0).abs() < 1.0,
        "labels on one column should share a centre: {long:?} vs {short:?}"
    );
}

/// A diagram that cannot be laid out says so, in words, rather than rendering
/// as an empty box.
#[core::prelude::v1::test]
fn a_broken_diagram_reports_itself() {
    let mut app = test_ui()
        .viewport(600, 200)
        .theme(install)
        .mount_offscreen(|| mermaid("this is not a diagram"));

    assert!(
        !app.query().role(Role::LABEL).all().is_empty(),
        "an undrawable diagram must render its error, not nothing"
    );
}

/// Exports the diagrams as PNGs so a person — or an agent with eyes — can look
/// at them. The assertions above cover the semantics; only a picture covers
/// whether the arrowheads point the right way and the diamond is a diamond.
///
/// Run with `--no-capture` to see where the files landed.
#[core::prelude::v1::test]
fn export_flowchart_images_for_visual_review() {
    for (case, source) in [("decision", DECISION), ("subgraphs", SUBGRAPHS)] {
        let mut app = test_ui()
            .viewport(800, 600)
            .theme(install)
            .mount_offscreen(move || mermaid(source));
        let captured = app.capture_snapshot("mermaid", case, "rendered");
        assert!(captured.path().is_file());
        println!("{}: {}", case, captured.path().display());
    }
}
