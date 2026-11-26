//! The `link` leaf: `Native<LinkConfig>` realized as a native link control
//! with link accessibility semantics, platform link appearance, pointer
//! affordance, and URL activation through `cocoa_ui::open_url`.

use cocoa_ui::accessibility::Role;
use waterui::Signal as _;
use waterui::component::ControlSize;
use waterui::component::button::ButtonStyle;
use waterui::component::link::LinkConfig;

use crate::components::button::realize_control_button;
use crate::contract::{Held, NativeLeaf, RenderContext};

/// Realizes `LinkConfig<Held>` as a native Apple link control.
pub fn realize_link_config(config: LinkConfig<Held>, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let (label, url) = config.into_parts();
    realize_control_button(
        &label,
        ButtonStyle::Link,
        ControlSize::Small,
        None,
        Some(Role::Link),
        move |_| {
            let target_url = url.snapshot();
            if !cocoa_ui::open_url(target_url.as_str(), mtm) {
                tracing::error!("Failed to open URL '{}'", target_url.as_str());
            }
        },
        ctx,
    )
}
