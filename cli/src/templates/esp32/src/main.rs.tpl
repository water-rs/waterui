//! ESP32 firmware entry for {{ ctx.app_display_name }}.

use waterui_core::Environment;
use waterui_dew::espidf::{PanelConfig, run};

fn main() {
    run(
        {{ ctx.crate_name_ident() }}::app(Environment::new()),
        PanelConfig::new({{ ctx.esp32.panel_width }}, {{ ctx.esp32.panel_height }}, {{ ctx.esp32.band_height }}),
    );
}
