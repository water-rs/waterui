//! ESP32 firmware entry for {{ ctx.app_display_name }}.

use waterui_core::Environment;
use waterui_dew::espidf::{PanelConfig, run};

/// Fonts bundled into flash for dew text shaping, from `[backends.esp32]
/// fonts` in `Water.toml`. Firmware has no font directory to enumerate, so a
/// text-rendering app must list at least one TTF/OTF here; dew fails fast at
/// the first text layout otherwise.
const FONTS: &[&[u8]] = &[
{%- for font in ctx.esp32.fonts %}
    include_bytes!("{{ font }}"),
{%- endfor %}
];

fn main() {
    run(
        {{ ctx.crate_name_ident() }}::app(Environment::new()),
        PanelConfig::new({{ ctx.esp32.panel_width }}, {{ ctx.esp32.panel_height }}, {{ ctx.esp32.band_height }}),
        FONTS,
    );
}
