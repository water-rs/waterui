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
    use core::any::type_name_of_val;

    use waterui::{Environment, OneOf2, OneOf3, OneOf8, View, view, view_builder};

    fn takes_view(_: impl View) {}

    fn assert_not_dynamic<V: View>(view: V) {
        let name = type_name_of_val(&view);
        assert!(
            !name.contains("Dynamic"),
            "view_builder/view! must unify branches statically without introducing Dynamic, got `{name}`"
        );
        takes_view(view);
    }

    #[expect(
        clippy::option_if_let_else,
        reason = "the test pins `#[view_builder]` lowering of `if let` with different branch types"
    )]
    #[view_builder]
    fn choose(flag: bool) -> impl View {
        let content = if flag { Some("ready") } else { None };

        if let Some(content) = content {
            content
        } else {
            ()
        }
    }

    #[test]
    fn fn_if_let() {
        assert_not_dynamic(choose(true));
        assert_not_dynamic(choose(false));
    }

    // An `else` block is the user's own braces: the lowering must not hand it
    // to `OneOf2::B` as an argument, or every plain `if`/`else` warns about
    // braces the user had to write.
    #[deny(unused_braces)]
    #[view_builder]
    fn branch_blocks(flag: bool) -> impl View {
        if flag { "on" } else { "off" }
    }

    #[test]
    fn branch_blocks_lower_without_unused_braces() {
        assert_not_dynamic(branch_blocks(false));
    }

    // Plain user enum: neither `Signal`, `Copy`, nor `Clone`, with an owned
    // payload moved out in an arm.
    enum Mode {
        Grid,
        List,
        Compact(String),
    }

    #[deny(unused_braces)]
    #[view_builder]
    fn tile(mode: Mode) -> impl View {
        match mode {
            Mode::Grid => "grid",
            Mode::List => {
                let empty = ();
                empty
            }
            Mode::Compact(label) if !label.is_empty() => label,
            Mode::Compact(_) => (),
        }
    }

    #[test]
    fn non_signal_user_enum_match() {
        assert_not_dynamic(tile(Mode::Grid));
        assert_not_dynamic(tile(Mode::List));
        assert_not_dynamic(tile(Mode::Compact(String::from("dense"))));
        assert_not_dynamic(tile(Mode::Compact(String::new())));

        let view_expr: OneOf3<&str, (), String> = view! {
            match Mode::Grid {
                Mode::Grid => "grid",
                Mode::List => (),
                Mode::Compact(label) => label,
            }
        };
        assert_not_dynamic(view_expr);
    }

    #[view_builder]
    fn let_chain_view(opt: Option<&'static str>, enabled: bool) -> impl View {
        if let Some(label) = opt
            && enabled
            && !label.is_empty()
        {
            label
        } else {
            ()
        }
    }

    #[test]
    fn if_let_chain_compiles_without_dynamic() {
        assert_not_dynamic(let_chain_view(Some("ok"), true));
        assert_not_dynamic(let_chain_view(Some("ok"), false));
        assert_not_dynamic(let_chain_view(None, true));

        let opt_view: Option<&str> = view! {
            if let Some(label) = Some("ok")
                && !label.is_empty()
            {
                label
            }
        };
        assert_not_dynamic(opt_view);
    }

    #[view_builder]
    fn nested_and_early_return(flag: bool, tier: u8) -> impl View {
        if flag {
            return if tier > 0 { "early" } else { () };
        }
        if tier == 0 { "zero" } else { () }
    }

    #[test]
    fn nested_branches_and_early_return() {
        assert_not_dynamic(nested_and_early_return(true, 1));
        assert_not_dynamic(nested_and_early_return(false, 0));

        let nested: OneOf2<&str, OneOf2<(), &str>> = view! {
            if false {
                "a"
            } else if true {
                ()
            } else {
                "c"
            }
        };
        assert_not_dynamic(nested);

        let nine_way: OneOf8<&str, (), &str, (), &str, (), &str, OneOf2<(), &str>> = view! {
            match 8u8 {
                0 => "0",
                1 => (),
                2 => "2",
                3 => (),
                4 => "4",
                5 => (),
                6 => "6",
                7 => (),
                _ => "8+",
            }
        };
        assert_not_dynamic(nine_way);
    }

    struct Sample(bool);

    impl View for Sample {
        #[view_builder]
        fn body(self, _env: &Environment) -> impl View {
            match self.0 {
                true => "enabled",
                false => (),
            }
        }
    }

    #[test]
    fn impl_view_body() {
        assert_not_dynamic(Sample(true).body(&Environment::new()));
    }

    #[test]
    fn view_macro_closure() {
        let builder = |flag: bool| -> OneOf2<&str, ()> {
            view! {
                match flag {
                    true => "visible",
                    false => (),
                }
            }
        };

        assert_not_dynamic(builder(true));
    }

    #[test]
    #[expect(
        clippy::no_effect,
        reason = "`view!` drops `;`-terminated statements; that token shape is what this case pins"
    )]
    fn semicolon_drop() {
        let with_else: OneOf2<&str, ()> = view! {
            "discarded";
            if true {
                "shown"
            } else {
                ()
            }
        };
        assert_not_dynamic(with_else);

        let without_else: Option<()> = view! {
            if true {
                "ignored";
            }
        };
        assert_not_dynamic(without_else);
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

    use waterui_testing::{PerfReport, TestStyle, UiBuilder};

    // The manual form's signature names `PerfReport`; the expansion must use
    // that declaration rather than its own path, or the import is unused.
    #[waterui::bench(theme = TestStyle::new())]
    #[ignore = "compile-only: the expansion's use of the declared return type is what this pins"]
    fn declared_report(ui: UiBuilder) -> PerfReport {
        ui.perf_with(
            || "idle",
            |perf| {
                perf.measure("idle", |_| {});
            },
        )
    }
}
