//! A tab's icon must survive the trip to the backend.
//!
//! A tab bar item is an image beside a title rather than one composed view, so
//! the icon has to reach the backend separately. Once the label is erased to an
//! `AnyView` nothing can recover which part of it was the icon, which is why
//! `Tab` lifts it out at construction.

use waterui::Binding;
use waterui::component::label::label;
use waterui::icon::system_icon;
use waterui::navigation::NavigationView;
use waterui::navigation::tab::{Tab, TabIcon};
use waterui::text::Text;
use waterui_icons_lucide as lucide;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Pane {
    Home,
    Browse,
    Plain,
}

fn tab(pane: Pane, label: impl waterui::component::IntoLabel) -> Tab<Pane> {
    Tab::new(pane, label, || {
        NavigationView::new("Root", Text::new("content"))
    })
}

#[test]
fn a_system_symbol_stays_a_symbol() {
    let tab = tab(Pane::Home, label("Home").icon(system_icon::home()));

    // A symbol the platform knows by name: the backend renders it natively
    // rather than receiving a bitmap.
    match tab.icon {
        Some(TabIcon::System(icon)) => assert_eq!(icon.name.as_str(), "house"),
        other => panic!("expected a system symbol, got {other:?}"),
    }
}

#[test]
fn a_packaged_icon_arrives_as_a_view() {
    let tab = tab(Pane::Browse, label("Browse").icon(lucide::compass()));

    // Not a platform symbol, so the backend has to draw it: the icon crosses as
    // a view for the host to rasterize.
    match tab.icon {
        Some(TabIcon::View(_)) => {}
        other => panic!("expected an icon view, got {other:?}"),
    }
}

#[test]
fn a_label_without_an_icon_carries_none() {
    let tab = tab(Pane::Plain, "Plain");
    assert!(tab.icon.is_none(), "a title-only tab has no icon to place");
}

#[test]
fn erasing_tab_identity_keeps_the_icon() {
    use waterui::navigation::Tabs;

    let selection = Binding::container(Pane::Home);
    let tabs = Tabs::new(
        &selection,
        vec![tab(Pane::Home, label("Home").icon(system_icon::home()))],
    );

    // The icon has to survive the projection into the `Id`-keyed container the
    // C ABI carries, which is the only form a backend ever sees.
    let layout = tabs.into_layout();
    match &layout.tabs[0].icon {
        Some(TabIcon::System(icon)) => assert_eq!(icon.name.as_str(), "house"),
        other => panic!("expected the symbol to survive erasure, got {other:?}"),
    }
}
