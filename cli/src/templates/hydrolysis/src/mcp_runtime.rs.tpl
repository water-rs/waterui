//! Hydrolysis MCP runtime for {{ ctx.app_display_name }}.

use waterui_mcp::ServerInfo;
use waterui_preview_protocol::hydrolysis::{MCP_RUN_CONFIG_ENV, McpRunConfig};
use waterui_testing::{RuntimeFlavor, ui};

/// Mounts the app headless and serves it to an agent over MCP stdio.
pub(crate) fn run() {
    let config =
        crate::run_config::load_run_config::<McpRunConfig>(MCP_RUN_CONFIG_ENV, "mcp");
    waterui_mcp::serve_stdio(
        || {
            let app = {{ ctx.crate_name_ident() }}::app(
                waterui::configure_environment!(waterui::env::Environment::new()),
            );
            let mut builder_env = waterui::env::Environment::new();
            // The CLI staged the graph's declared fonts into the resource directory.
            builder_env.insert(hydrolysis::DeclaredFonts::default());
            ui()
                .environment(builder_env)
                .viewport(config.width, config.height)
                .runtime(RuntimeFlavor::Application)
                .scale_factor(config.scale_factor)
                .theme(hydrolysis_m3::Material3::defaults())
                .mount_app(app)
        },
        ServerInfo {
            name: "{{ ctx.app_display_name }}".to_owned(),
            version: Some(env!("CARGO_PKG_VERSION").to_owned()),
        },
    )
    .unwrap_or_else(|error| panic!("hydrolysis mcp: server failed: {error}"));
}
