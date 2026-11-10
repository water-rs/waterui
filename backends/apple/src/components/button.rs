//! The `button` leaf: `Native<ButtonConfig>` rendered as a platform button
//! carrying a `WaterUI` label laid out by Rust.
//!
//! Mirrors `WuiButton`: a `HostView` container holds the platform button
//! plus a hit-test-transparent label container (`inside` the button on
//! `UIKit`, a sibling above it on `AppKit`). The label subtree renders
//! against a cloned environment whose `Foreground` slot is the color the
//! style dictates — accent, primary or accent-foreground, per platform.
//! Disabled state, the accent color and the accessibility label arrive
//! through watchers; every change is an imperative kit call. The style and
//! size mapping is [`button_chrome`]'s, shared with the menu trigger.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::action::ActionTarget;
use cocoa_ui::{PlatformView, Rect as KitRect, Retained, view};
use waterui::component::button::{ButtonConfig, ButtonStyle};
use waterui::reactive::Computed;
use waterui::text::StyledStr;
use waterui_backend_core::{AnyView, Environment};
use waterui_core::handler::BoxedAction;
use waterui_core::interaction::Disabled;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};

use crate::components::button_chrome;
use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;
use crate::proposal;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::{Button, HitTest, HostView};

    /// An owned handle to the platform button: `Retained` on `AppKit`.
    pub(super) type OwnedButton = cocoa_ui::Retained<Button>;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::action::ControlEvents;
    pub(super) use cocoa_ui::uikit::{Button, HitTest, HostView};

    /// An owned handle to the platform button: the wrapper itself on `UIKit`.
    pub(super) type OwnedButton = Button;
}

use platform::{Button, HitTest, HostView};

/// The button as its platform view, for frames, alpha and accessibility.
#[cfg(target_os = "macos")]
fn as_view(button: &Button) -> &PlatformView {
    button
}

/// The button as its platform view, for frames, alpha and accessibility.
#[cfg(target_os = "ios")]
fn as_view(button: &Button) -> &PlatformView {
    button.as_ref()
}

/// The button as the platform button its style draws on.
#[cfg(target_os = "macos")]
fn styled(button: &platform::OwnedButton) -> button_chrome::StyledButton {
    Retained::into_super(button.clone())
}

/// The button as the platform button its style draws on.
#[cfg(target_os = "ios")]
fn styled(button: &platform::OwnedButton) -> button_chrome::StyledButton {
    button.clone()
}

/// Sets the button's chrome for `style` and reports the padding the label
/// keeps inside it: `UIKit` reads it from the configuration's own content
/// insets.
#[cfg(target_os = "ios")]
fn configure_button(
    _button: &Button,
    style: ButtonStyle,
    mtm: cocoa_ui::MainThreadMarker,
) -> (f32, f32) {
    button_chrome::chrome_padding(style, mtm)
}

/// Sets the button's bezel for `style` and reports the padding the bezel
/// cell keeps around its content.
#[cfg(target_os = "macos")]
fn configure_button(
    button: &Button,
    style: ButtonStyle,
    size: button_chrome::NSControlSize,
) -> (f32, f32) {
    button_chrome::apply_bezel(button, style, size);
    // A transparent borderless NSButton reports AXUnknown instead of
    // AXButton.
    button.mark_accessible_as_button();
    if button_chrome::draws_chrome(style) {
        let (horizontal, vertical) = button.content_padding();
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the layout contract is f32; chrome insets are a few points"
        )]
        (horizontal as f32, vertical as f32)
    } else {
        (0.0, 0.0)
    }
}

/// The offer the embedded label measures under: the button's proposal
/// minus its padding on each axis, never negative.
fn label_offer(proposal: ProposalSize, horizontal: f32, vertical: f32) -> ProposalSize {
    ProposalSize {
        width: proposal
            .width
            .map(|width| horizontal.mul_add(-2.0, width).max(0.0)),
        height: proposal
            .height
            .map(|height| vertical.mul_add(-2.0, height).max(0.0)),
    }
}

/// The label's measurements plus the chrome padding around them.
struct ButtonSubView {
    child: crate::contract::Mounted,
    padding: (f32, f32),
}

impl SubView for ButtonSubView {
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let (horizontal, vertical) = self.padding;
        let measured = self
            .child
            .layout()
            .measure(label_offer(proposal, horizontal, vertical));
        ViewDimensions::new(Size::new(
            horizontal.mul_add(2.0, measured.size.width),
            vertical.mul_add(2.0, measured.size.height),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::None
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Registers the `button` leaf on `dispatcher`.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ButtonConfig>(|config, ctx| {
        let mtm = ctx.mtm();
        let style = config.style();

        // The label subtree resolves `Foreground` from the color the style
        // prescribes for its platform, and on `AppKit` its default `Body`
        // from the control size.
        let label_env = button_chrome::label_environment(style, config.size(), ctx.env());

        let container = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        let button = Button::new(mtm);
        let label_container = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        // The label never intercepts input meant for the button's chrome.
        label_container.set_hit_test_handler(|_, _| HitTest::Pass);

        #[cfg(target_os = "ios")]
        let padding = configure_button(&button, style, mtm);
        #[cfg(target_os = "macos")]
        let padding = configure_button(&button, style, button_chrome::control_size(config.size()));

        let (label, action, ..) = config.into_parts();
        let accessibility_label = label.accessibility_label();
        let label_leaf = ctx.with_env(&label_env).render(AnyView::new(label));
        let child_view = view::retain_base(label_leaf.view());
        let mounted = label_leaf.mount(&label_container);

        // `AppKit`: the transparent label sits above the bezel as a sibling;
        // `UIKit`: inside the button so the chrome draws beneath it.
        view::add_subview(&container, as_view(&button));
        #[cfg(target_os = "macos")]
        view::add_subview(&container, &label_container);
        #[cfg(target_os = "ios")]
        as_view(&button).addSubview(&label_container);

        // Frames: the button fills the container; the label container sits
        // inside it, inset by the chrome's own padding. The child view fills
        // the label container through its own layout handler.
        {
            let button = button.clone();
            let label_container = label_container.clone();
            container.set_layout_handler(move |host| {
                // The button draws in its response rectangle, centred in
                // the frame it was assigned (docs/layout-spec.md §2.4).
                let bounds = proposal::response(host);
                view::set_frame(as_view(&button), bounds);
                let horizontal = f64::from(padding.0);
                let vertical = f64::from(padding.1);
                let label_frame = KitRect::new(
                    horizontal,
                    vertical,
                    horizontal.mul_add(-2.0, bounds.size.width).max(0.0),
                    vertical.mul_add(-2.0, bounds.size.height).max(0.0),
                );
                // `AppKit`'s label is the bezel's sibling in the host's
                // space; `UIKit`'s is inside the button, in its own.
                #[cfg(target_os = "macos")]
                let label_frame = label_frame.translated(bounds.origin);
                view::set_frame(&label_container, label_frame);
            });
        }
        #[cfg(target_os = "macos")]
        let link_child = child_view.clone();
        {
            label_container.set_layout_handler(move |host| {
                view::set_frame(&child_view, view::bounds(host));
            });
        }

        let mut leaf = NativeLeaf::new(
            &container,
            ButtonSubView {
                child: mounted,
                padding,
            },
        );
        leaf.keep(label_env);

        // The action runs against the button's own environment — the one
        // `Button<Action>` extracted state into — not the label's clone.
        let action = Rc::new(RefCell::new(action));
        let action_env = ctx.env().clone();
        wire_action(&mut leaf, &button, action, action_env);

        bind_accessibility(&mut leaf, &button, &label_container, &accessibility_label);
        bind_disabled(&mut leaf, &button, &label_container, ctx.env());
        button_chrome::bind_tint(&mut leaf, styled(&button), style, ctx.env(), mtm);
        #[cfg(target_os = "macos")]
        if style == ButtonStyle::Link {
            enable_link_chrome(&button, &link_child);
        }
        leaf
    });
}

/// Wires `config.action` through `ActionTarget` so a tap runs it on the
/// main thread.
fn wire_action(
    leaf: &mut NativeLeaf,
    button: &Button,
    action: Rc<RefCell<BoxedAction>>,
    env: Environment,
) {
    #[cfg(target_os = "macos")]
    leaf.keep(ActionTarget::new(button.control(), move |_| {
        (action.borrow_mut())(&env);
    }));

    #[cfg(target_os = "ios")]
    {
        leaf.keep(ActionTarget::new(
            button.control(),
            platform::ControlEvents::TOUCH_UP_INSIDE,
            move |_| {
                (action.borrow_mut())(&env);
            },
        ));
    }
}

/// `UIKit` pressed/disabled feedback: touch-down dims the label to 0.55,
/// any lift or exit restores it — disabled stays at 0.45.
#[cfg(target_os = "ios")]
fn wire_highlight(
    leaf: &mut NativeLeaf,
    button: &platform::OwnedButton,
    label_container: &Retained<HostView>,
) {
    use platform::ControlEvents;
    let update = {
        let button = button.clone();
        let label_container = label_container.clone();
        Rc::new(move |highlighted: bool| {
            let alpha = if button.is_enabled() {
                if highlighted { 0.55 } else { 1.0 }
            } else {
                0.45
            };
            view::set_alpha(&label_container, alpha);
        })
    };
    let down = update.clone();
    leaf.keep(ActionTarget::new(
        button.control(),
        ControlEvents::TOUCH_DOWN | ControlEvents::TOUCH_DRAG_ENTER,
        move |_| down(true),
    ));
    leaf.keep(ActionTarget::new(
        button.control(),
        ControlEvents::TOUCH_UP_INSIDE
            | ControlEvents::TOUCH_UP_OUTSIDE
            | ControlEvents::TOUCH_CANCEL
            | ControlEvents::TOUCH_DRAG_EXIT,
        move |_| update(false),
    ));
}

/// Pushes the label's accessibility text onto the chrome's element and
/// removes the visual label from the accessibility hierarchy.
fn bind_accessibility(
    leaf: &mut NativeLeaf,
    button: &Button,
    label_container: &HostView,
    label: &Computed<StyledStr>,
) {
    let button_view = view::retain_base(as_view(button));
    leaf.bind(label, move |styled| {
        let plain = cocoa_ui::text::strip_bidi_controls(styled.to_plain().as_str());
        view::set_accessibility_label(&button_view, &plain);
    });
    view::hide_from_accessibility(label_container);
}

/// The disabled signal: `isEnabled` flips and the label dims.
fn bind_disabled(
    leaf: &mut NativeLeaf,
    button: &platform::OwnedButton,
    label_container: &Retained<HostView>,
    env: &Environment,
) {
    let disabled = Disabled::resolve(env, false);
    {
        let button = button.clone();
        let label_container = label_container.clone();
        leaf.bind(&disabled, move |is_disabled| {
            button.set_enabled(!is_disabled);
            view::set_alpha(&label_container, if is_disabled { 0.45 } else { 1.0 });
        });
    }
    #[cfg(target_os = "ios")]
    wire_highlight(leaf, button, label_container);
}

/// The link style's affordances on `AppKit`: the pointing-hand cursor over
/// the button, and press feedback dimming the label — `AppKit` draws no
/// pressed state for a transparent bezel.
#[cfg(target_os = "macos")]
fn enable_link_chrome(button: &Button, child_view: &PlatformView) {
    button.set_link_cursor(true);
    let child_view = view::retain_base(child_view);
    button.set_press_handler(move |_, pressed| {
        view::set_alpha(&child_view, if pressed { 0.5 } else { 1.0 });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_offer_subtracts_padding_and_clamps() {
        let offer = label_offer(
            ProposalSize {
                width: Some(120.0),
                height: Some(40.0),
            },
            16.0,
            6.0,
        );
        assert_eq!(offer.width, Some(88.0));
        assert_eq!(offer.height, Some(28.0));

        // Padding larger than the proposal clamps to zero rather than going
        // negative, matching the Swift `max(0, ...)`.
        let clamped = label_offer(
            ProposalSize {
                width: Some(10.0),
                height: None,
            },
            16.0,
            6.0,
        );
        assert_eq!(clamped.width, Some(0.0));
        assert_eq!(clamped.height, None);
    }
}
