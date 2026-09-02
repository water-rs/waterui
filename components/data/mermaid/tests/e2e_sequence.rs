//! What a rendered sequence diagram owes its reader.
//!
//! A sequence diagram is mostly text — participant names, message labels,
//! fragment keywords and their guards — so almost everything worth asserting is
//! an accessibility node. The geometry assertions are the ones that catch a
//! label placed at the origin instead of on its message.

use hydrolysis_m3::install;
use waterui_mermaid::mermaid;
use waterui_testing::{OffscreenApp, Role, ui as test_ui};

const CONVERSATION: &str = "\
sequenceDiagram
    participant Alice
    actor Bob
    Alice->>Bob: hello
    Bob-->>Alice: hi
    Note right of Bob: thinking
";

const FRAGMENTS: &str = "\
sequenceDiagram
    participant Alice
    participant Bob
    loop every minute
        Alice->>Bob: poll
    end
    alt is ready
        Bob-->>Alice: yes
    else is not
        Bob-->>Alice: no
    end
";

fn app(source: &'static str) -> OffscreenApp {
    test_ui()
        .viewport(900, 700)
        .theme(install)
        .mount_offscreen(move || mermaid(source))
}

#[core::prelude::v1::test]
fn participants_and_messages_are_accessible() {
    let mut app = app(CONVERSATION);
    for label in ["Alice", "Bob", "hello", "hi", "thinking"] {
        app.query().role(Role::LABEL).label(label).assert_exists();
    }
}

/// A participant is drawn twice — a header at the top and one at the bottom —
/// so its name appears twice, and the two must be at the same `x` and different
/// `y`. Getting one of the two headers wrong is invisible to a "does the name
/// exist" check.
#[core::prelude::v1::test]
fn a_participant_has_a_header_at_each_end_of_its_lifeline() {
    let mut app = app(CONVERSATION);
    let headers: Vec<_> = app
        .query()
        .role(Role::LABEL)
        .label("Alice")
        .all()
        .iter()
        .map(waterui_testing::ElementRef::bounds)
        .collect();

    assert_eq!(
        headers.len(),
        2,
        "a participant should be labelled at both ends of its lifeline, got {headers:?}"
    );
    assert!(
        (headers[0].center().0 - headers[1].center().0).abs() < 1.0,
        "the two headers should share a column: {headers:?}"
    );
    assert!(
        (headers[0].y() - headers[1].y()).abs() > 10.0,
        "the two headers should be at opposite ends: {headers:?}"
    );
}

/// Messages run down the page in source order.
#[core::prelude::v1::test]
fn messages_are_ordered_down_the_page() {
    let mut app = app(CONVERSATION);
    let hello = app
        .query()
        .role(Role::LABEL)
        .label("hello")
        .single()
        .bounds();
    let hi = app.query().role(Role::LABEL).label("hi").single().bounds();
    assert!(
        hello.y() < hi.y(),
        "`hello` is sent before `hi`, so it belongs above it: {hello:?} vs {hi:?}"
    );
}

/// A message label sits between the two participants it runs between, not at
/// the diagram's origin.
#[core::prelude::v1::test]
fn a_message_label_sits_between_its_participants() {
    let mut app = app(CONVERSATION);
    let alice = app
        .query()
        .role(Role::LABEL)
        .label("Alice")
        .all()
        .iter()
        .next()
        .expect("Alice is labelled")
        .bounds();
    let bob = app
        .query()
        .role(Role::LABEL)
        .label("Bob")
        .all()
        .iter()
        .next()
        .expect("Bob is labelled")
        .bounds();
    let hello = app
        .query()
        .role(Role::LABEL)
        .label("hello")
        .single()
        .bounds();

    let (left, right) = if alice.center().0 < bob.center().0 {
        (alice, bob)
    } else {
        (bob, alice)
    };
    assert!(
        hello.center().0 > left.center().0 && hello.center().0 < right.center().0,
        "the message label should sit between its participants: {hello:?} between {left:?} and {right:?}"
    );
}

/// `loop` and `alt` frames carry their keyword and their guard.
#[core::prelude::v1::test]
fn fragment_keywords_and_guards_are_accessible() {
    let mut app = app(FRAGMENTS);
    for label in ["loop", "every minute", "alt", "is ready"] {
        app.query().role(Role::LABEL).label(label).assert_exists();
    }
}

/// The `alt` frame opens below the `loop` frame closes, because that is the
/// order they are written in. A fragment whose vertical extent was dropped
/// would put both at the same place.
#[core::prelude::v1::test]
fn fragments_are_stacked_in_source_order() {
    let mut app = app(FRAGMENTS);
    let loop_keyword = app
        .query()
        .role(Role::LABEL)
        .label("loop")
        .single()
        .bounds();
    let alt_keyword = app.query().role(Role::LABEL).label("alt").single().bounds();
    assert!(
        loop_keyword.y() < alt_keyword.y(),
        "`loop` is written before `alt`, so its frame opens above: {loop_keyword:?} vs {alt_keyword:?}"
    );
}

/// Exports the diagrams as PNGs for visual review. The assertions above cover
/// the semantics; only a picture covers whether the lifelines are dashed, the
/// actor is a stick figure and the fragment frames enclose what they should.
#[core::prelude::v1::test]
fn export_sequence_images_for_visual_review() {
    for (case, source) in [("conversation", CONVERSATION), ("fragments", FRAGMENTS)] {
        let mut app = app(source);
        let captured = app.capture_snapshot("mermaid", case, "rendered");
        assert!(captured.path().is_file());
        println!("{}: {}", case, captured.path().display());
    }
}
