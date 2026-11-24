//! Integration smoke tests proving `waterui-testing` runs under ordinary `cargo test`.

use waterui::component::text;
use waterui::env::Environment;
use waterui::graphics::color::Srgb;
use waterui::locale::TranslationCatalog;
use waterui::{Plugin, ResourceContext, View, ViewExt as _};
use waterui_testing::{Role, TestStyle, UiBuilder, ui};

fn sample_view() -> impl waterui::View {
    text("Macro-mounted label")
        .body()
        .foreground(Srgb::WHITE)
        .background(Srgb::BLACK)
}

#[waterui::test(sample_view)]
fn waterui_test_macro_runs_inside_cargo_test(app: &mut waterui_testing::SemanticApp) {
    let label = app
        .query()
        .role(Role::LABEL)
        .label("Macro-mounted label")
        .single();
    assert_eq!(label.node().label(), Some("Macro-mounted label"));
}

#[test]
fn direct_ui_test_mount_runs_inside_cargo_test() {
    let mut app = ui().viewport(240, 120).mount(sample_view);
    let label = app
        .query()
        .role(Role::LABEL)
        .label("Macro-mounted label")
        .single();
    assert_eq!(label.node().label(), Some("Macro-mounted label"));
}

#[waterui::test]
fn waterui_test_additive_env_preserves_resources_and_catalog(ui: UiBuilder) {
    struct MockService(&'static str);
    struct TestPlugin(&'static str);
    struct PluginInstalled(&'static str);

    impl Plugin for TestPlugin {
        fn install(self, env: &mut Environment) {
            env.insert(PluginInstalled(self.0));
        }
    }

    struct EnvProbe;

    impl View for EnvProbe {
        fn body(self, env: &Environment) -> impl View {
            let service = env
                .get::<MockService>()
                .expect("MockService must stay installed")
                .0;
            let plugin = env
                .get::<PluginInstalled>()
                .expect("TestPlugin value must stay installed")
                .0;
            assert!(
                env.get::<ResourceContext>().is_some(),
                "ApplicationResources must stay installed"
            );
            assert!(
                env.get::<TranslationCatalog>().is_some(),
                "TranslationCatalog must stay installed"
            );
            text(format!("{service}:{plugin}")).body()
        }
    }

    let mut app = ui
        .with(MockService("service"))
        .install(TestPlugin("plugin"))
        .mount(|| EnvProbe);
    app.query()
        .role(Role::LABEL)
        .label("service:plugin")
        .assert_exists();
}

#[waterui::test(sample_view)]
fn mounted_test_accepts_result(app: &mut waterui_testing::SemanticApp) -> Result<(), &'static str> {
    let label = app
        .query()
        .role(Role::LABEL)
        .label("Macro-mounted label")
        .single()
        .node()
        .label()
        .map(str::to_owned)
        .ok_or("missing label")?;
    assert_eq!(label, "Macro-mounted label");
    Ok(())
}

#[waterui::test(sample_view)]
async fn async_test_accepts_result(
    app: &mut waterui_testing::SemanticApp,
) -> Result<(), &'static str> {
    let label = app
        .query()
        .role(Role::LABEL)
        .label("Macro-mounted label")
        .single()
        .node()
        .label()
        .map(str::to_owned)
        .ok_or("expected mounted label")?;
    assert_eq!(label, "Macro-mounted label");
    Ok(())
}

#[waterui::test(theme = TestStyle::new(), viewport = (240, 120))]
fn themed_manual_test_takes_plain_uibuilder_and_result(ui: UiBuilder) -> Result<(), &'static str> {
    let mut app = ui.mount_offscreen(sample_view);
    let label = app
        .query()
        .role(Role::LABEL)
        .label("Macro-mounted label")
        .single()
        .node()
        .label()
        .map(str::to_owned)
        .ok_or("missing rendered label")?;
    assert_eq!(label, "Macro-mounted label");
    Ok(())
}
