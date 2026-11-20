//! A tab's icon must survive the trip to the backend.
//!
//! A tab bar item is an image beside a title rather than one composed view, so
//! the icon has to reach the backend separately. Once the label is erased to an
//! `AnyView` nothing can recover which part of it was the icon, which is why
//! `Tab` lifts it out at construction.

use waterui::Binding;
use waterui::View;
use waterui::component::label::label;
use waterui::navigation::NavigationView;
use waterui::navigation::tab::Tab;
use waterui::text::Text;
use waterui_icons_lucide as lucide;

#[derive(Clone, Copy)]
struct MarkerIcon;

impl View for MarkerIcon {
    fn body(self, _env: &waterui::Environment) -> impl View {}
}

fn tab(
    label: impl waterui::component::IntoLabel,
) -> Tab<impl waterui::handler::ViewBuilder<Output = NavigationView<Text>>> {
    Tab::new(label, || NavigationView::new("Root", Text::new("content")))
}

#[test]
fn a_packaged_icon_arrives_as_a_view() {
    let tab = tab(label("Browse").icon(lucide::compass()));

    assert!(
        tab.icon().is_some(),
        "an icon-bearing label should preserve its icon builder"
    );
}

#[test]
fn a_label_without_an_icon_carries_none() {
    let tab = tab("Plain");
    assert!(
        tab.icon().is_none(),
        "a title-only tab has no icon to place"
    );
}

#[test]
fn erasing_tab_identity_keeps_the_typed_icon() {
    use waterui::AnyView;
    use waterui::navigation::Tabs;

    let selection = Binding::container(0usize);
    let tabs = Tabs::new(&selection, vec![tab(label("Home").icon(MarkerIcon))]);

    // The icon has to survive the projection into the `Id`-keyed container the
    // C ABI carries, which is the only form a backend ever sees, and preserve
    // its concrete view type so backends can downcast backend-provided icons.
    let layout = tabs.into_layout::<AnyView>();
    let mounted = layout.tabs().snapshot();
    let icon = mounted
        .get_view(0)
        .expect("a mounted tab")
        .icon()
        .expect("icon should survive erasure")
        .build();
    assert!(
        icon.downcast_ref::<MarkerIcon>().is_some(),
        "typed icon should survive AnyView erasure"
    );
}
