//! The navigation metadata wrappers: the transition source/destination
//! markers and the navigation-link hint.
//!
//! All three are transparent containers — they forward the child's layout
//! face and lay it out over their bounds — whose only state is a tag the
//! platform chrome reads:
//!
//! - `Metadata<NavigationTransitionSource>` / `…Destination` mark the views
//!   a matched-geometry transition pairs: `tag` on `UIKit`, `identifier`
//!   `dev.waterui.navigation.transition.<id>` on `AppKit`.
//! - `Metadata<NavigationLinkHint>` marks a destination-following
//!   row so `WuiList` can draw the platform's affordance; the marker is the
//!   accessibility identifier `dev.waterui.navigation-link-hint`.

use waterui::navigation::{
    NavigationLinkHint, NavigationTransitionDestination, NavigationTransitionSource,
};
use waterui_core::Metadata;

use crate::components::passthrough::passthrough_leaf;
use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

/// The marker a link-hint leaf carries; `WuiList`'s `containsNavigationLink`
/// matches on it.
pub const LINK_HINT_IDENTIFIER: &str = "dev.waterui.navigation-link-hint";

/// The `AppKit` interface identifier a transition-tagged leaf carries.
#[cfg(target_os = "macos")]
const TRANSITION_IDENTIFIER_PREFIX: &str = "dev.waterui.navigation.transition.";

/// Tags `leaf`'s platform view as a transition source or destination: a
/// `tag` on `UIKit`, an interface identifier on `AppKit`. A zero id is the
/// same `fatalError` the Swift wrapper raised.
#[cfg(target_os = "ios")]
fn tag_transition(leaf: &NativeLeaf, id: waterui_core::id::Id) {
    // `Id` is already nonzero — the `NonZeroI32` it wraps is the contract.
    cocoa_ui::view::set_tag(leaf.view(), i32::from(id) as isize);
}

/// Tags `leaf`'s platform view as a transition source or destination: a
/// `tag` on `UIKit`, an interface identifier on `AppKit`.
#[cfg(target_os = "macos")]
fn tag_transition(leaf: &NativeLeaf, id: waterui_core::id::Id) {
    use cocoa_ui::objc2_app_kit::NSUserInterfaceItemIdentification as _;
    let identifier = objc2_foundation::NSString::from_str(&alloc::format!(
        "{TRANSITION_IDENTIFIER_PREFIX}{}",
        i32::from(id)
    ));
    leaf.view().setIdentifier(Some(&identifier));
}

/// Installs the three metadata handlers on the dispatcher.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<NavigationTransitionSource>>(|metadata, ctx| {
        let leaf = passthrough_leaf(metadata.content, ctx);
        tag_transition(&leaf, metadata.value.id());
        leaf
    });

    dispatcher.register_view::<Metadata<NavigationTransitionDestination>>(|metadata, ctx| {
        let leaf = passthrough_leaf(metadata.content, ctx);
        tag_transition(&leaf, metadata.value.id());
        leaf
    });

    dispatcher.register_view::<Metadata<NavigationLinkHint>>(|metadata, ctx| {
        let leaf = passthrough_leaf(metadata.content, ctx);
        cocoa_ui::view::set_accessibility_identifier(leaf.view(), Some(LINK_HINT_IDENTIFIER));
        leaf
    });
}
