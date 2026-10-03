//! The `picker` leaf: `Native<PickerConfig>` rendered as a container view
//! holding the platform selection control and, on macOS, its leading label.
//!
//! Mirrors `WuiPicker`: a menu becomes `NSPopUpButton` / a `UIButton` with a
//! `UIMenu`, segmented becomes `NSSegmentedControl` / `UISegmentedControl`,
//! and radio becomes a vertical stack of radio buttons on `AppKit` or the
//! single-column `UIPickerView` wheel `UIKit` offers for `.inline`. The
//! `selection` `Binding<Id>` is two-way: watchers push the tag's index onto
//! the control, and the control's action writes the picked index's tag back.
//!
//! Each item's label is its own `Computed<StyledStr>`: the leaf keeps one
//! watcher per item so a retitled row updates in place rather than
//! re-rendering, and the `items` collection itself rebuilds the control's
//! rows when it changes.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use cocoa_ui::{PlatformView, Rect, Retained};
#[cfg(target_os = "macos")]
use waterui::component::LabelDisplayMode;
use waterui::component::form::picker::{PickerConfig, PickerItem, PickerStyle as WuiPickerStyle};
use waterui::reactive::{Computed, Signal};
use waterui::resolve::Resolvable;
use waterui::text::StyledStr;
use waterui::text::font::{Body, FontDesign, FontWeight, ResolvedFont};
use waterui_core::id::Id;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HostView, Picker};
    pub(super) use cocoa_ui::picker::PickerStyle;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::picker::PickerStyle;
    pub(super) use cocoa_ui::uikit::{HostView, Picker};
}

use platform::{HostView, Picker, PickerStyle};

/// The gap between the leading label and the control.
const LABEL_SPACING: f64 = 8.0;

/// The guard a per-item label watcher returns — held until the items list
/// is rebuilt or the leaf drops.
type ItemGuard = <Computed<StyledStr> as Signal>::Guard;

/// A label that measures empty contributes no spacing beside it either.
fn spacing(spacing: f64, beside: f64) -> f64 {
    if beside > 0.0 { spacing } else { 0.0 }
}

/// The kit style a `PickerStyle` resolves to; `Automatic` takes the
/// platform default, which `WuiPicker` sets to the menu presentation.
const fn platform_style(style: WuiPickerStyle) -> PickerStyle {
    match style {
        WuiPickerStyle::Radio => PickerStyle::Radio,
        WuiPickerStyle::Segmented => PickerStyle::Segmented,
        // `Automatic`, `Menu` and future styles take the menu control.
        _ => PickerStyle::Menu,
    }
}

/// A styled string as display text: the plain characters with the bidi
/// control characters interpolation inserts for layout stripped.
fn item_title(styled: &StyledStr) -> String {
    styled
        .to_plain()
        .chars()
        .filter(|c| {
            !matches!(c,
                '\u{200e}' | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

/// The leaf's live state, shared between the watchers, the layout face and
/// the host's layout handler.
struct PickerState {
    /// The platform selection control.
    picker: Picker,
    /// The two-way selection the control writes into.
    selection: waterui::reactive::Binding<Id>,
    /// `index → tag`, parallel to `titles`.
    tags: Vec<Id>,
    /// The plain titles currently shown, parallel to `tags`.
    titles: Vec<String>,
    /// The per-item label watchers; replaced wholesale on rebuild.
    item_watchers: Vec<ItemGuard>,
    /// The rendered label child — only on `AppKit` when the label isn't
    /// hidden; `UIKit` speaks the label on the container instead.
    /// `Option` only because the layout handler is installed before the
    /// child exists.
    label: Option<Mounted>,
    /// The style for layout: radio centers the label on the first row.
    style: PickerStyle,
}

/// Rebuilds the control's rows from `items`: new tags, new titles, a fresh
/// watcher per item label, then re-syncs the selection onto the new rows.
fn rebuild_items(state: &Rc<RefCell<PickerState>>, items: &[PickerItem<Id>]) {
    let mut borrowed = state.borrow_mut();
    borrowed.tags = items.iter().map(|item| item.tag).collect();
    borrowed.titles = items
        .iter()
        .map(|item| item_title(&item.content.content().snapshot()))
        .collect();
    let titles = borrowed.titles.clone();
    let selection_index = borrowed
        .tags
        .iter()
        .position(|tag| *tag == borrowed.selection.snapshot());
    borrowed.picker.set_items(&titles);
    borrowed.picker.set_selected_index(selection_index);

    borrowed.item_watchers = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let state = Rc::clone(state);
            item.content.content().watch(move |ctx| {
                let mut borrowed = state.borrow_mut();
                borrowed.titles[index] = item_title(ctx.value());
                crate::measure_memo::invalidate();
                let titles = borrowed.titles.clone();
                borrowed.picker.set_items(&titles);
                let index = borrowed
                    .tags
                    .iter()
                    .position(|tag| *tag == borrowed.selection.snapshot());
                borrowed.picker.set_selected_index(index);
            })
        })
        .collect();
}

/// Lays out the children inside `view`'s bounds: the label leading, then
/// the control filling the rest of the row. For the radio style the label
/// centers on the first row, not the whole stack — `WuiPicker`'s
/// `radioLabelAlignment` constraint. RTL mirrors the row.
fn layout_children(view: &PlatformView, state: &PickerState) {
    let bounds = cocoa_ui::view::bounds(view);
    let width = bounds.size.width;
    let height = bounds.size.height;
    let rtl = cocoa_ui::view::is_right_to_left(view);

    let place = |child: &PlatformView, x: f64, y: f64, w: f64, h: f64| {
        let x = if rtl { width - x - w } else { x };
        cocoa_ui::view::set_frame(child, Rect::new(x, y, w, h));
    };

    let label_size = state.label.as_ref().map_or_else(Size::default, |label| {
        label.layout().measure(ProposalSize::UNSPECIFIED).size
    });
    let label_w = f64::from(label_size.width);
    let control_x = label_w + spacing(LABEL_SPACING, label_w);

    // The control hugs its content: its own intrinsic width beside the
    // label, never the leaf's leftover — a wider leaf just means slack the
    // layout already positioned around us.
    let control_w = {
        #[cfg(target_os = "macos")]
        {
            state.picker.intrinsic_size().width
        }
        #[cfg(not(target_os = "macos"))]
        {
            (width - control_x).max(0.0)
        }
    };
    place(state.picker.view(), control_x, 0.0, control_w, height);

    if let Some(label) = &state.label {
        let label_h = f64::from(label_size.height);
        // Radio rows align the label to the first button's center; every
        // other style centers it on the control. `UIKit` never mounts a
        // label, so `first_row_height` is `AppKit`-only.
        let center_y = {
            #[cfg(target_os = "macos")]
            {
                state
                    .picker
                    .first_row_height()
                    .filter(|_| state.style == PickerStyle::Radio)
                    .map_or(height / 2.0, |row| row / 2.0)
            }
            #[cfg(not(target_os = "macos"))]
            {
                height / 2.0
            }
        };
        place(
            label.view(),
            0.0,
            center_y - label_h / 2.0,
            label_w,
            label_h,
        );
    }
}

/// The container's layout face: the label plus the control across, the
/// taller of the two high. The `UIKit` wheel is the exception —
/// `WuiPicker` stretches it to the proposed width.
struct PickerSubView {
    state: Rc<RefCell<PickerState>>,
}

impl core::fmt::Debug for PickerSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PickerSubView").finish_non_exhaustive()
    }
}

impl SubView for PickerSubView {
    // `measure` speaks f32; the geometry math runs in f64 — the narrowing is
    // the layout contract, as in `text`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = self.state.borrow();
        let label_size = state.label.as_ref().map_or_else(Size::default, |label| {
            label.layout().measure(ProposalSize::UNSPECIFIED).size
        });
        let control_size = state.picker.intrinsic_size();
        let label_w = f64::from(label_size.width);
        let intrinsic_width = label_w + spacing(LABEL_SPACING, label_w) + control_size.width;
        let intrinsic_height = f64::from(label_size.height).max(control_size.height);

        // A `SwiftUI` `Picker` hugs its content: label plus control at their
        // intrinsic sizes. The `UIKit` wheel is the exception — `WuiPicker`
        // stretches it to the proposed width.
        let wheel = state.style == PickerStyle::Radio && cfg!(target_os = "ios");
        let width = if wheel {
            proposal.width.map_or(intrinsic_width, f64::from)
        } else {
            intrinsic_width
        };
        let height = if wheel {
            intrinsic_height
        } else {
            proposal
                .height
                .map_or(intrinsic_height, |h| f64::from(h).max(intrinsic_height))
        };
        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        let _ = self;
        // `SwiftUI`'s picker does not stretch — even unlabeled it takes its
        // intrinsic size; an hstack spacer positions it, not the leaf.
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The platform weight of a `FontWeight` on the `NSFont`/`UIFont` scale.
const fn platform_weight(weight: FontWeight) -> f64 {
    use cocoa_ui::font::weight;
    match weight {
        FontWeight::Thin => weight::THIN,
        FontWeight::UltraLight => weight::ULTRA_LIGHT,
        FontWeight::Light => weight::LIGHT,
        FontWeight::Normal => weight::REGULAR,
        FontWeight::Medium => weight::MEDIUM,
        FontWeight::SemiBold => weight::SEMI_BOLD,
        FontWeight::Bold => weight::BOLD,
        FontWeight::UltraBold => weight::HEAVY,
        FontWeight::Black => weight::BLACK,
    }
}

/// The comma-separated candidates of a CSS-style family list, trimmed with
/// the empties dropped.
fn family_candidates(family: &str) -> impl Iterator<Item = &str> {
    family
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
}

/// The platform face a resolved font names — the `Body` resolved font
/// applied to the control's items.
///
/// # Panics
///
/// When the family names no installed font and no generic — the same
/// `fatalError` `text` raises.
fn platform_font(
    mtm: cocoa_ui::MainThreadMarker,
    resolved: &ResolvedFont,
) -> Retained<cocoa_ui::Font> {
    let size = f64::from(resolved.size);
    let weight = platform_weight(resolved.weight);
    match resolved.family.as_deref() {
        Some(family) if !family.is_empty() => {
            let mut resolved_font = None;
            for candidate in family_candidates(family) {
                resolved_font = match candidate {
                    "system" | "sans-serif" => Some(cocoa_ui::font::system(mtm, size, weight)),
                    _ => cocoa_ui::font::named(candidate, size),
                };
                if resolved_font.is_some() {
                    break;
                }
            }
            resolved_font.unwrap_or_else(|| {
                panic!(
                    "WaterUI: font family '{family}' not found. Ensure the font is bundled and registered."
                )
            })
        }
        _ => match resolved.design {
            FontDesign::Default => cocoa_ui::font::system(mtm, size, weight),
            FontDesign::Monospaced => cocoa_ui::font::monospaced(mtm, size, weight),
        },
    }
}

/// Installs the `picker` handler on the dispatcher: `Native<PickerConfig>`
/// maps to a container view with the platform selection control and, on
/// `AppKit`, the visible leading label child.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<PickerConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let picker = Picker::new(mtm, platform_style(config.style));
        let host_view: &PlatformView = &host;
        cocoa_ui::view::add_subview(host_view, picker.view());

        let state = Rc::new(RefCell::new(PickerState {
            picker,
            selection: config.selection.clone(),
            tags: Vec::new(),
            titles: Vec::new(),
            item_watchers: Vec::new(),
            label: None,
            style: platform_style(config.style),
        }));

        // Control → binding: a picked index writes its tag back.
        state.borrow().picker.install_action({
            let state = Rc::clone(&state);
            let selection = config.selection.clone();
            move |index| {
                if let Some(tag) = state.borrow().tags.get(index).copied() {
                    selection.set(tag);
                }
            }
        });

        // The label is visible chrome on `AppKit` unless hidden; on `UIKit`
        // it is spoken text on the container only.
        #[cfg(target_os = "macos")]
        {
            if !matches!(
                config.label.display_mode_preference(),
                LabelDisplayMode::Hidden
            ) {
                let label = ctx
                    .render(waterui_backend_core::AnyView::new(config.label.clone()))
                    .mount(host_view);
                cocoa_ui::view::hide_from_accessibility(label.view());
                state.borrow_mut().label = Some(label);
            }
        }

        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state.borrow())
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            PickerSubView {
                state: Rc::clone(&state),
            },
        );

        // Items → control: rebuild rows and per-item label watchers on each
        // change (`bind` applies the current list immediately).
        leaf.bind(&config.items, {
            let state = Rc::clone(&state);
            move |items| rebuild_items(&state, &items)
        });

        // Binding → control: a changed tag selects its index.
        leaf.watch(&config.selection, {
            let state = Rc::clone(&state);
            move |ctx| {
                let index = state
                    .borrow_mut()
                    .tags
                    .iter()
                    .position(|tag| *tag == *ctx.value());
                state.borrow().picker.set_selected_index(index);
            }
        });

        // The theme's body font drives the control's item faces.
        let body = Body.resolve(ctx.env());
        leaf.bind(&body, {
            let state = Rc::clone(&state);
            move |font: ResolvedFont| {
                state.borrow().picker.set_font(&platform_font(mtm, &font));
            }
        });

        // The label's semantic text is announced on the container — as the
        // accessibility label (and tooltip on `AppKit`), exactly the
        // `WuiControlAccessibility` treatment.
        let accessibility_label = config.label.accessibility_label();
        leaf.bind(&accessibility_label, {
            let host_view = cocoa_ui::view::retain_base(host_view);
            move |styled| {
                cocoa_ui::view::set_accessibility_label(&host_view, &item_title(&styled));
            }
        });

        // A disabled subtree must not respond to input.
        if let Some(disabled) = ctx.env().get::<Disabled>() {
            leaf.bind(disabled.signal(), {
                let state = Rc::clone(&state);
                move |is_disabled: bool| state.borrow().picker.set_enabled(!is_disabled)
            });
        }

        leaf.keep(state);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_collapses_beside_an_empty_label() {
        assert_eq!(spacing(LABEL_SPACING, 0.0).to_bits(), 0.0_f64.to_bits());
        assert_eq!(spacing(LABEL_SPACING, 12.0).to_bits(), 8.0_f64.to_bits());
    }

    #[test]
    fn style_maps_automatic_to_menu() {
        assert_eq!(platform_style(WuiPickerStyle::Automatic), PickerStyle::Menu);
        assert_eq!(platform_style(WuiPickerStyle::Menu), PickerStyle::Menu);
        assert_eq!(platform_style(WuiPickerStyle::Radio), PickerStyle::Radio);
        assert_eq!(
            platform_style(WuiPickerStyle::Segmented),
            PickerStyle::Segmented
        );
    }

    #[test]
    fn item_title_strips_bidi_marks() {
        assert_eq!(item_title(&StyledStr::from("a\u{202a}b\u{202c}")), "ab");
    }
}
