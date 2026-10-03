//! The `secure_field` leaf: `Native<SecureFieldConfig>` rendered through
//! the kit's masked text field inside a container that also holds the
//! label child.
//!
//! Mirrors `WuiSecureField`: the `value` `Binding<Secure>` is two-way —
//! watchers push the exposed text onto the field and user edits write back
//! as a fresh `Secure` — the label mounts above the field and names it to
//! assistive technology, `Disabled` pushes `set_enabled`, and the theme
//! `Body` font drives the field's font. The measure is
//! `WuiSecureField.sizeThatFits`: a 100pt-width floor, the label stacked
//! over the field with 4pt of spacing, height always at least intrinsic.
//!
//! The plaintext lives only inside the platform control and the `Secure`
//! wrapper: watcher copies are scoped to the setter call, and edit
//! writebacks move the `String` straight into `Secure::new`.
//!
//! The field is also this subtree's focus anchor — the kit marks it so
//! `Metadata<Focused>` resolves it the way `installWuiFocusTarget` marked
//! it in Swift, as in `text_field`.

use alloc::rc::Rc;

use cocoa_ui::{PlatformView, Rect, Retained, focus, view};
use waterui::component::form::secure::{Secure, SecureFieldConfig};
use waterui::resolve::Resolvable;
use waterui::text::StyledStr;
use waterui::text::font::{Body, FontDesign, FontWeight, ResolvedFont};
use waterui_core::interaction::Disabled;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{HostView, SecureField};
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::{HostView, SecureField};
}

use platform::{HostView, SecureField};

/// The gap between the label and the field — `WuiSecureField.verticalSpacing`.
const VERTICAL_SPACING: f64 = 4.0;
/// The narrowest width the leaf reports — `WuiSecureField`'s 100pt floor.
const MIN_WIDTH: f64 = 100.0;

/// The platform weight of a `FontWeight` on the `UIFont`/`NSFont` scale —
/// the same table `text_field` uses.
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

/// The comma-separated candidates of a CSS-style family list — as in
/// `text_field`.
fn family_candidates(family: &str) -> impl Iterator<Item = &str> {
    family
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
}

/// The platform face a resolved font names — the same rule as
/// `text_field`'s `platform_font` with no italic pass: a masked field
/// draws plain text.
///
/// # Panics
///
/// When the family names no installed font and no generic — the same
/// `fatalError` the Swift port raises.
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

/// The field's share of a measure — the text height `WuiSecureField`
/// reports. `AppKit` reads `intrinsicContentSize`; `UIKit` floors
/// `sizeThatFits` at intrinsic so the field never reports shorter than its
/// font needs.
#[cfg(target_os = "macos")]
fn field_height(field: &SecureField, _width: f64) -> f64 {
    field.measured_height()
}

/// The field's share of a measure — see the `AppKit` variant.
#[cfg(target_os = "ios")]
fn field_height(field: &SecureField, width: f64) -> f64 {
    field.measured_height(width)
}

/// The leaf's live state: the platform field and the mounted label child.
struct FieldState {
    field: Retained<SecureField>,
    label: Mounted,
}

/// Lays out the children inside `view`'s bounds: the label at top-leading,
/// then the field spanning the full width below it — `WuiSecureField`'s
/// constraints as manual frames. `UIKit` centers the input inside the
/// region between the label and the container's bottom, the way the
/// field's `inputRegion` layout guide does.
fn layout_children(view: &PlatformView, state: &FieldState) {
    let bounds = view::bounds(view);
    let label = state.label.layout().measure(ProposalSize::UNSPECIFIED).size;
    let has_label = label.height > 0.0;
    view::set_hidden(state.label.view(), !has_label);

    view::set_frame(
        state.label.view(),
        Rect::new(0.0, 0.0, f64::from(label.width), f64::from(label.height)),
    );

    let region_top = if has_label {
        f64::from(label.height) + VERTICAL_SPACING
    } else {
        0.0
    };
    let region_height = (bounds.size.height - region_top).max(0.0);
    let field_height = field_height(&state.field, bounds.size.width).min(region_height);
    #[cfg(target_os = "macos")]
    let field_frame = Rect::new(0.0, region_top, bounds.size.width, field_height);
    #[cfg(target_os = "ios")]
    let field_frame = Rect::new(
        0.0,
        region_top + (region_height - field_height) / 2.0,
        bounds.size.width,
        field_height,
    );
    let field_view: &PlatformView = &state.field;
    view::set_frame(field_view, field_frame);
}

/// The container's layout face: reports `WuiSecureField.sizeThatFits`'s
/// answer — the proposed width floored at `max(label, 100pt)` and height
/// never below intrinsic — stretching horizontally at priority 0.
struct SecureFieldSubView {
    state: Rc<FieldState>,
}

impl core::fmt::Debug for SecureFieldSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SecureFieldSubView").finish_non_exhaustive()
    }
}

/// The width the field's own measure is taken at: the proposed width under
/// `WuiSecureField`'s 100pt floor, and never narrower than the label.
#[cfg(target_os = "ios")]
fn measure_width(proposal: ProposalSize, label_width: f64) -> f64 {
    proposal
        .width
        .map_or(MIN_WIDTH, |w| f64::from(w).max(MIN_WIDTH))
        .max(label_width)
}

/// `AppKit` measures the field at its intrinsic height regardless of the
/// offer; the proposal is ignored the way `WuiSecureField` ignores it.
#[cfg(target_os = "macos")]
const fn measure_width(_proposal: ProposalSize, _label_width: f64) -> f64 {
    0.0
}

impl SubView for SecureFieldSubView {
    // `measure` speaks f32; the geometry math runs in f64 — the narrowing
    // is the layout contract, as in `text_field`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the layout contract is f32; measured points always fit"
    )]
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let state = &self.state;
        let label = state.label.layout().measure(ProposalSize::UNSPECIFIED).size;
        let has_label = label.height > 0.0;

        let text_height = field_height(
            &state.field,
            measure_width(proposal, f64::from(label.width)),
        );
        let intrinsic_height =
            f64::from(label.height) + if has_label { VERTICAL_SPACING } else { 0.0 } + text_height;
        let min_width = f64::from(label.width).max(MIN_WIDTH);
        let width = proposal
            .width
            .map_or(min_width, |w| f64::from(w).max(min_width));
        let height = proposal
            .height
            .map_or(intrinsic_height, |h| f64::from(h).max(intrinsic_height));
        ViewDimensions::new(Size::new(width as f32, height as f32))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Installs the `secure_field` handler on the dispatcher:
/// `Native<SecureFieldConfig>` maps to a container with the platform
/// secure field and the mounted label child; the binding, the theme font,
/// disabled state and accessibility stay live through watchers.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<SecureFieldConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let field = SecureField::new(mtm);

        let field_view: &PlatformView = &field;
        host.add_subview(field_view);

        // The label renders above the field; its semantic text names the
        // control while the visual label leaves the accessibility tree —
        // `WuiControlAccessibility`'s split.
        let accessibility_label = config.label.accessibility_label();
        let host_view: &PlatformView = &host;
        let mounted = ctx
            .render(waterui_backend_core::AnyView::new(config.label))
            .mount(host_view);
        view::hide_from_accessibility(mounted.view());

        let state = Rc::new(FieldState {
            field: field.clone(),
            label: mounted,
        });
        host.set_layout_handler({
            let state = Rc::clone(&state);
            move |view| layout_children(view, &state)
        });

        let control = view::retain_base(field_view);
        let mut leaf = NativeLeaf::new(
            host_view,
            SecureFieldSubView {
                state: Rc::clone(&state),
            },
        );

        // Binding → field: external writes land on the field; an equal
        // value is skipped so a user's own edit cannot echo back and move
        // the caret (the role `isSyncingFromBinding` played in Swift —
        // nami notifies synchronously inside `binding.set`).
        leaf.bind(&config.value, {
            let field = field.clone();
            move |secure: Secure| {
                let text = secure.expose();
                if field.string() != text {
                    field.set_string(text);
                }
            }
        });

        // Field → binding: user edits write back; the `String` moves into
        // `Secure` with no plaintext copy kept.
        #[cfg(target_os = "ios")]
        leaf.keep(field.install_change_handler({
            let binding = config.value.clone();
            move |field| binding.set(Secure::new(field.string()))
        }));
        #[cfg(target_os = "macos")]
        field.on_change({
            let binding = config.value.clone();
            move |field| binding.set(Secure::new(field.string()))
        });

        // The theme `Body` font drives the field's font —
        // `bodyFontObservation` in `WuiSecureField`.
        let body = Body.resolve(ctx.env());
        leaf.bind(&body, {
            let field = field.clone();
            move |font| field.set_font(&platform_font(mtm, &font))
        });

        // Announce the label's semantic text on the field.
        leaf.bind(&accessibility_label, {
            move |styled: StyledStr| {
                let plain = cocoa_ui::text::strip_bidi_controls(&styled.to_plain());
                view::set_accessibility_label(&control, &plain);
            }
        });

        // A disabled subtree must not respond to input.
        let disabled = Disabled::resolve(ctx.env(), false);
        leaf.bind(&disabled, {
            let field = field.clone();
            move |is_disabled| field.set_enabled(!is_disabled)
        });

        // The field is this subtree's focus anchor, as in `text_field`.
        leaf.keep(focus::install(mtm, field_view));

        leaf.keep(state);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measure constants `WuiSecureField` fixes: a 4pt gap and a 100pt
    /// floor — the same contract `text_field` documents.
    #[test]
    fn measure_constants_match_the_swift_values() {
        assert_eq!(VERTICAL_SPACING.to_bits(), 4.0_f64.to_bits());
        assert_eq!(MIN_WIDTH.to_bits(), 100.0_f64.to_bits());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn measure_width_ignores_the_proposal() {
        let proposal = ProposalSize::new(Some(42.0), None);
        assert_eq!(measure_width(proposal, 60.0).to_bits(), 0.0_f64.to_bits());
    }

    #[cfg(target_os = "ios")]
    #[test]
    fn measure_width_floors_at_100_and_the_label() {
        let proposal = ProposalSize::new(Some(42.0), None);
        assert_eq!(measure_width(proposal, 0.0).to_bits(), 100.0_f64.to_bits());
        assert_eq!(
            measure_width(proposal, 250.0).to_bits(),
            250.0_f64.to_bits()
        );
        let unspecified = ProposalSize::UNSPECIFIED;
        assert_eq!(
            measure_width(unspecified, 0.0).to_bits(),
            100.0_f64.to_bits()
        );
    }
}
