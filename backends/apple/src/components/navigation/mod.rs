//! The navigation group: navigation stacks, split views, tab containers,
//! the navigation bar model, menus and the window's toolbar, plus the
//! metadata the group consumes.
//!
//! `Native<NavigationView>` is a page: chrome (title, items, search) plus
//! content. Inside a `NavigationStack` the platform's own navigation chrome
//! hosts the bar (`UINavigationBar` on `UIKit`, the window toolbar on
//! `AppKit`); standalone it draws an in-content bar. `Native<NavigationStack>`
//! is the stack container, `Native<NavigationSplitLayout>` a two- or
//! three-column split, and `Native<TabsLayout>` the platform's tab
//! container.

use alloc::string::String;

use cocoa_ui::PlatformView;

pub mod bar;
pub mod metadata;
pub mod nav_view;
pub mod split;
pub mod stack;
pub mod tabs;

/// Installs every navigation handler on the dispatcher.
pub fn install(dispatcher: &mut crate::dispatch::Dispatcher) {
    metadata::install(dispatcher);
    nav_view::install(dispatcher);
    stack::install(dispatcher);
    split::install(dispatcher);
    tabs::install(dispatcher);
}

/// The first plain-text string a rendered subtree's text views carry —
/// `extractNavigationTitleText`'s walk: labels on `UIKit`, text fields on
/// `AppKit`, depth-first.
#[cfg(target_os = "macos")]
pub fn extract_title_text(view: &PlatformView) -> Option<String> {
    use cocoa_ui::objc2_app_kit::NSTextField;
    if let Some(field) = view.downcast_ref::<NSTextField>() {
        let text = field.stringValue().to_string();
        if !text.is_empty() {
            return Some(text);
        }
        let attributed = field.attributedStringValue().string().to_string();
        if !attributed.is_empty() {
            return Some(attributed);
        }
    }
    for subview in &cocoa_ui::view::subviews(view) {
        if let Some(found) = extract_title_text(subview) {
            return Some(found);
        }
    }
    None
}

/// The first plain-text string a rendered subtree's text views carry —
/// `extractNavigationTitleText`'s walk on `UIKit`.
#[cfg(target_os = "ios")]
pub fn extract_title_text(view: &PlatformView) -> Option<String> {
    use cocoa_ui::objc2_ui_kit::UILabel;
    if let Some(label) = view.downcast_ref::<UILabel>() {
        let text = label
            .attributedText()
            .map(|text| text.string().to_string())
            .or_else(|| label.text().map(|text| text.to_string()));
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            return Some(text);
        }
    }
    for subview in &cocoa_ui::view::subviews(view) {
        if let Some(found) = extract_title_text(subview) {
            return Some(found);
        }
    }
    None
}

/// A text toolbar button as the bar's own title item.
///
/// `None` when the item is not a single text button — an icon, a custom
/// view, or plain text — so the caller hosts that content instead. A hosted
/// `UIButton` whose configuration carries no title is what the bar draws as
/// an empty capsule.
#[cfg(target_os = "ios")]
#[must_use]
pub fn text_button_bar_item(
    item: &bar::BarItem,
    action: Option<alloc::rc::Rc<dyn Fn()>>,
    mtm: cocoa_ui::MainThreadMarker,
) -> Option<cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIBarButtonItem>> {
    use cocoa_ui::objc2::rc::Weak;
    use cocoa_ui::objc2_foundation::NSString;

    let button = cocoa_ui::uikit::first_button(item.leaf.view())?;
    let label = button_text_label(&button)?;
    let title = label_plain(&label);
    let item = cocoa_ui::uikit::title_bar_item(mtm, &title, action);
    // The action retains the button, and the button retains this label. A
    // strong hold on the bar item here would cycle back through the action.
    let weak = Weak::new(&*item);
    label.set_text_handler(move |text| {
        let Some(item) = weak.load() else {
            return;
        };
        let plain = cocoa_ui::text::strip_bidi_controls(text);
        item.setTitle(Some(&NSString::from_str(&plain)));
    });
    Some(item)
}

/// A navigation bar item as a `UIBarButtonItem`.
///
/// The action is forwarded to the rendered content's first button. A symbol
/// icon becomes the item's image. A view icon is measured and framed, then
/// `view_icon` turns it into an item. A single text button is a title item
/// the bar draws; any other content is hosted as the item's custom view. The
/// item follows the button's enabled state and the title signal feeds its
/// accessibility label.
#[cfg(target_os = "ios")]
#[must_use]
pub fn button_bar_item(
    item: &bar::BarItem,
    keep: &mut crate::contract::KeepAlive,
    mtm: cocoa_ui::MainThreadMarker,
    view_icon: impl FnOnce(
        &crate::contract::NativeLeaf,
        Option<alloc::rc::Rc<dyn Fn()>>,
        &mut crate::contract::KeepAlive,
    ) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIBarButtonItem>,
) -> cocoa_ui::Retained<cocoa_ui::objc2_ui_kit::UIBarButtonItem> {
    use waterui_core::layout::ProposalSize;

    let leaf_button = cocoa_ui::uikit::first_button(item.leaf.view());
    let action = leaf_button.clone().map(|button| {
        alloc::rc::Rc::new(move || {
            button.sendActionsForControlEvents(
                cocoa_ui::objc2_ui_kit::UIControlEvents::PrimaryActionTriggered,
            );
        }) as alloc::rc::Rc<dyn Fn()>
    });
    let button = match &item.icon {
        Some(bar::BarItemIcon::System(symbol)) => {
            cocoa_ui::uikit::bar_item(mtm, Some(symbol), None, action)
        }
        Some(bar::BarItemIcon::View(icon)) => {
            // The declared icon draws inside the bar's chrome as a template
            // image the bar tints; a view that cannot render stays hosted as
            // the item's custom view.
            let size = icon
                .layout()
                .measure(ProposalSize {
                    width: None,
                    height: None,
                })
                .size;
            cocoa_ui::view::set_frame(
                icon.view(),
                cocoa_ui::geometry::Rect::new(
                    0.0,
                    0.0,
                    f64::from(size.width),
                    f64::from(size.height),
                ),
            );
            view_icon(icon, action, keep)
        }
        None => {
            // A `UIButton` whose configuration carries no title is what the
            // bar draws as an empty capsule. A single text label becomes the
            // title the bar draws itself; anything else is hosted, and must
            // arrive with a real frame because the bar never lays a custom
            // view out.
            text_button_bar_item(item, action, mtm).unwrap_or_else(|| {
                cocoa_ui::view::set_frame(item.leaf.view(), bar::bar_item_frame(item));
                cocoa_ui::uikit::bar_item(mtm, None, Some(item.leaf.view()), None)
            })
        }
    };
    // A bar item the chrome draws follows the button's enabled state — the
    // programmatic `sendActions` the action runs ignores `isEnabled`, so the
    // bar item is what disables it.
    if let Some(leaf_button) = &leaf_button {
        cocoa_ui::uikit::mirror_enabled(leaf_button, &button);
    }
    if let Some(title) = &item.title {
        keep.bind(title, {
            let button = button.clone();
            move |title| {
                cocoa_ui::uikit::bar_item_accessibility_label(&button, &title.to_plain());
            }
        });
    }
    button
}

/// What one hosted subtree contributes to a text-button decision.
#[cfg(target_os = "ios")]
enum ButtonContent {
    /// No visible content.
    Empty,
    /// Exactly one non-empty text label.
    Text(cocoa_ui::Retained<cocoa_ui::uikit::Label>),
    /// An icon, a second label, or any other view.
    Other,
}

/// The button's label when its hosted content is exactly one text label.
///
/// Configuration chrome on the `UIButton` itself is skipped: only `HostView`
/// subviews are the label the button rendered.
#[cfg(target_os = "ios")]
#[must_use]
fn button_text_label(
    button: &cocoa_ui::objc2_ui_kit::UIButton,
) -> Option<cocoa_ui::Retained<cocoa_ui::uikit::Label>> {
    use cocoa_ui::uikit::HostView;
    let mut text = None;
    for sub in &button.subviews() {
        if sub.downcast_ref::<HostView>().is_none() {
            continue;
        }
        match button_content(&sub) {
            ButtonContent::Empty => {}
            ButtonContent::Text(label) => {
                if text.is_some() {
                    return None;
                }
                text = Some(label);
            }
            ButtonContent::Other => return None,
        }
    }
    text
}

/// Classifies `view` for [`button_text_label`].
#[cfg(target_os = "ios")]
#[must_use]
fn button_content(view: &PlatformView) -> ButtonContent {
    use cocoa_ui::Retained;
    use cocoa_ui::uikit::{HostView, Label};
    if let Some(label) = view.downcast_ref::<Label>() {
        return if label_plain(label).is_empty() {
            ButtonContent::Empty
        } else {
            ButtonContent::Text(Retained::from(label))
        };
    }
    if view.downcast_ref::<HostView>().is_none() {
        return ButtonContent::Other;
    }
    let mut text = None;
    for sub in &cocoa_ui::view::subviews(view) {
        match button_content(sub) {
            ButtonContent::Empty => {}
            ButtonContent::Text(label) => {
                if text.is_some() {
                    return ButtonContent::Other;
                }
                text = Some(label);
            }
            ButtonContent::Other => return ButtonContent::Other,
        }
    }
    text.map_or(ButtonContent::Empty, ButtonContent::Text)
}

/// The label's plain string, without the bidi controls text shaping inserts.
#[cfg(target_os = "ios")]
#[must_use]
fn label_plain(label: &cocoa_ui::uikit::Label) -> String {
    let raw = label
        .source_text()
        .map(|text| text.string().to_string())
        .unwrap_or_default();
    cocoa_ui::text::strip_bidi_controls(&raw)
}
