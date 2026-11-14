//! Expansions of the `WaterUI` macros that must compile and run.
//!
//! The rejection side of each macro is pinned by the `compile_fail` cases in
//! `trybuild_macro_contracts`; the accepted shapes live here, compiled by the
//! test build itself. Under `--test` the `#[test]` wrapper that
//! `#[waterui::test]` emits is type-checked and executed, which a trybuild
//! `pass` case — a plain binary build, where rustc drops `#[test]` items —
//! never did.

mod text {
    #[test]
    fn named_placeholders() {
        let name = "world";
        let _ = waterui::text!("Hello, {name}");
        let _ = waterui::text!("{count} items", count = 3);
        let _ = waterui::text!("I have {#count} apple", count = 3);
        let _ = waterui::text!("{{literal}} braces");
    }
}

mod view_builder {
    fn takes_view(_: impl waterui::View) {}

    #[waterui::view_builder]
    fn choose(flag: bool) -> impl waterui::View {
        let content = if flag { Some("ready") } else { None };

        if let Some(content) = content {
            content
        } else {
            ()
        }
    }

    #[test]
    fn fn_if_let() {
        let _ = choose(true);
    }

    // An `else` block is the user's own braces: the lowering must not hand it
    // to `Err` as an argument, or every plain `if`/`else` warns about braces
    // the user had to write.
    #[deny(unused_braces)]
    #[waterui::view_builder]
    fn branch_blocks(flag: bool) -> impl waterui::View {
        if flag { "on" } else { "off" }
    }

    #[test]
    fn branch_blocks_lower_without_unused_braces() {
        takes_view(branch_blocks(false));
    }

    struct Sample(bool);

    impl waterui::View for Sample {
        #[waterui::view_builder]
        fn body(self, _env: &waterui::Environment) -> impl waterui::View {
            match self.0 {
                true => "enabled",
                false => (),
            }
        }
    }

    #[test]
    fn impl_view_body() {
        let _ = Sample(true);
    }

    #[test]
    fn view_macro_closure() {
        let builder = |flag: bool| {
            waterui::view! {
                match flag {
                    true => "visible",
                    false => (),
                }
            }
        };

        takes_view(builder(true));
    }

    #[test]
    #[expect(
        clippy::no_effect,
        reason = "`view!` drops `;`-terminated statements; that token shape is what this case pins"
    )]
    fn semicolon_drop() {
        takes_view(waterui::view! {
            "discarded";
            if true {
                "shown"
            } else {
                ()
            }
        });

        takes_view(waterui::view! {
            if true {
                "ignored";
            }
        });
    }
}

mod waterui_test {
    fn sample_view() -> impl waterui::View {}

    #[waterui::test(sample_view)]
    fn sample_sync_test(app: &mut waterui_testing::SemanticApp) {
        let _ = app.tree();
    }

    #[waterui::test(sample_view)]
    async fn sample_async_test(app: &mut waterui_testing::SemanticApp) {
        let _ = app.tree();
    }

    #[waterui::test]
    fn manual_mount(ui: waterui_testing::UiBuilder) {
        let app = ui.mount(sample_view);
        let _ = app.tree();
    }
}

mod waterui_bench {
    #![deny(unused_imports)]

    use waterui_testing::{PerfReport, Styled, TestStyle, UiBuilder};

    // The manual form's signature names `PerfReport`; the expansion must use
    // that declaration rather than its own path, or the import is unused.
    #[waterui::bench(theme = TestStyle::new())]
    #[ignore = "compile-only: the expansion's use of the declared return type is what this pins"]
    fn declared_report(ui: UiBuilder<Styled<TestStyle>>) -> PerfReport {
        ui.perf_with(
            || "idle",
            |perf| {
                perf.measure("idle", |_| {});
            },
        )
    }
}
