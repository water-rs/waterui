//! The framework-wide default type scale must rank like the markdown heading
//! ladder `heading_style` builds on it: H1 (`Title`) larger than H2
//! (`Headline`), H5 (`Footnote`) not smaller than H6 (`Caption`).

use waterui_text::font::{Body, Caption, FontSlot, Footnote, Headline, Subheadline, Title};

#[test]
fn default_heading_slots_never_increase_with_level() {
    let sizes = [
        ("Title", Title::DEFAULT.size()),
        ("Headline", Headline::DEFAULT.size()),
        ("Body", Body::DEFAULT.size()),
        ("Subheadline", Subheadline::DEFAULT.size()),
        ("Footnote", Footnote::DEFAULT.size()),
        ("Caption", Caption::DEFAULT.size()),
    ];
    for pair in sizes.windows(2) {
        assert!(
            pair[0].1 >= pair[1].1,
            "{} ({}pt) must not be smaller than {} ({}pt)",
            pair[0].0,
            pair[0].1,
            pair[1].0,
            pair[1].1
        );
    }
    assert!(
        Title::DEFAULT.size() > Headline::DEFAULT.size(),
        "H1 must render larger than H2"
    );
}
