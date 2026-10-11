//! Mounting a TypeScript module through `tsx!`, end to end.
//!
//! The bundle is the real one — `fixtures/mount.js`, the library plus one
//! module plus the `installRuntimeGlobal` call, bundled by
//! `scripts/build-test-library.sh` — loaded into the engine this target
//! selects with the facade's host table installed. What the tests assert is
//! the accessibility tree the mounted module produces, because that is what a
//! person using the application meets.
//!
//! The contract table the bundle publishes is seeded from
//! `PromoProps::CONTRACT_HASH` before the bundle is evaluated. That stands in
//! for the `water` CLI, which reads the same constant out of the compiled
//! artifact and writes it into the entry it generates; here both ends of the
//! check are one Rust constant, so a test that declares the wrong hash has to
//! say so deliberately.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nami::{Binding, Computed};
use waterui::ts::{Components, Mount, NoProps, RuntimeHandle, TsError, TsRuntime};
use waterui::tsx;
use waterui_core::{AnyView, Environment};
use waterui_graphics::color::ColorScheme;
use waterui_testing::{SemanticApp, ui};
use waterui_ts::engine::{JsRuntime as _, JsValue};
use waterui_ts::schema::{TsProps, decode_mount};

#[path = "support/artifact.rs"]
mod artifact;

/// The whole application bundle: library, module and entry.
const BUNDLE: &str = include_str!("fixtures/mount.js");

/// The module id `tsx!("fixtures/promo.tsx", …)` resolves to, and the key the
/// bundle publishes the module under.
const MODULE: &str = "tests/fixtures/promo.tsx";

/// The props `fixtures/promo.tsx` is typed against.
#[derive(TsProps)]
struct PromoProps {
    headline: String,
    unread: Binding<u32>,
    #[ts(rename = "onDismiss")]
    on_dismiss: Box<dyn Fn()>,
}

/// The headline every test mounts the module with.
const HEADLINE: &str = "Welcome back";

/// The props a test that is not about the props themselves mounts with.
fn props(unread: Binding<u32>) -> PromoProps {
    PromoProps {
        headline: String::from(HEADLINE),
        unread,
        on_dismiss: Box::new(|| {}),
    }
}

/// A runtime whose loaded bundle declares `hash` as the module's contract.
fn runtime_declaring(hash: u64) -> TsRuntime {
    let environment = Environment::new()
        .store::<ColorScheme, Computed<ColorScheme>>(Computed::constant(ColorScheme::Light));
    let runtime = TsRuntime::new(environment, Components).expect("the runtime constructs");
    runtime
        .bridge()
        .engine()
        .eval(
            &format!("globalThis.__waterui_test_contracts = {{ \"{MODULE}\": \"{hash:016x}\" }};"),
            "test:contracts.js",
        )
        .expect("seeding the contract table");
    runtime
        .load(BUNDLE)
        .unwrap_or_else(|error| panic!("loading the bundle: {error}"));
    runtime
}

/// A runtime whose bundle declares the contract the binary actually mounts.
fn runtime() -> RuntimeHandle {
    RuntimeHandle::new(runtime_declaring(PromoProps::CONTRACT_HASH))
}

/// The environment a mount finds `handle` in.
fn environment(handle: &RuntimeHandle) -> Environment {
    handle.clone().install(&Environment::new())
}

/// Mounts one view in a test session.
///
/// A view crosses from TypeScript once, so the builder hands it over exactly
/// once and says so if it is asked twice.
fn session(view: AnyView) -> SemanticApp {
    let view = RefCell::new(Some(view));
    ui().viewport(320, 240).mount(move || {
        view.borrow_mut()
            .take()
            .expect("the test session realizes its root once")
    })
}

#[test]
fn ts_tsx_mounts_the_module_the_path_names() {
    let unread = Binding::container(3_u32);
    let dismissed = Rc::new(Cell::new(false));
    let view = {
        let dismissed = Rc::clone(&dismissed);
        tsx!(
            "fixtures/promo.tsx",
            PromoProps {
                headline: String::from(HEADLINE),
                unread,
                on_dismiss: Box::new(move || dismissed.set(true)),
            }
        )
    };
    assert_eq!(view.module(), MODULE, "the macro resolved the module id");

    let handle = runtime();
    let mounted = view
        .try_mount(&environment(&handle))
        .unwrap_or_else(|error| panic!("mounting: {error}"));
    let mut app = session(mounted);

    app.query().label(HEADLINE).assert_exists();
    app.query().label("3 unread").assert_exists();
    app.query().label("Dismiss").single().tap(&mut app);
    assert!(
        dismissed.get(),
        "tapping the button called the props callback"
    );
}

#[test]
fn ts_a_prop_binding_keeps_pushing_after_the_mount_returns() {
    // The mount's exports outlive `try_mount`: the cell pushing `unread` into
    // the module's signal belongs to the mount's scope, and the view carries
    // that scope. Without it this reads "3 unread" for ever.
    let unread = Binding::container(3_u32);
    let view = tsx!(
        "fixtures/promo.tsx",
        PromoProps {
            headline: String::from(HEADLINE),
            unread: unread.clone(),
            on_dismiss: Box::new(|| {}),
        }
    );

    let handle = runtime();
    let mounted = view
        .try_mount(&environment(&handle))
        .unwrap_or_else(|error| panic!("mounting: {error}"));
    let mut app = session(mounted);

    unread.set(11);
    app.query().label("11 unread").assert_exists();
    app.query().label("3 unread").assert_not_exists();
}

#[test]
fn ts_a_module_with_no_props_is_checked_against_the_empty_contract() {
    // A module that takes no props is not a module with no contract: it is
    // typed against `NoProps`, and the bundle has to declare that hash.
    let handle = runtime();
    let error = Mount::new::<NoProps>(MODULE, NoProps {})
        .try_mount(&environment(&handle))
        .expect_err("the bundle declares the PromoProps contract, not the empty one");

    let TsError::ContractMismatch {
        expected, declared, ..
    } = error
    else {
        panic!("expected a contract mismatch, got {error}")
    };
    assert_eq!(expected, NoProps::CONTRACT_HASH);
    assert_eq!(declared, PromoProps::CONTRACT_HASH);
}

#[test]
fn ts_a_bundle_from_another_build_is_refused_with_both_hashes() {
    let declared = !PromoProps::CONTRACT_HASH;
    let handle = RuntimeHandle::new(runtime_declaring(declared));
    let error = Mount::new::<PromoProps>(MODULE, props(Binding::container(0)))
        .try_mount(&environment(&handle))
        .expect_err("the bundle was built against another contract");

    let message = error.to_string();
    assert!(
        message.contains(&format!("{declared:#018x}")),
        "the message carries the hash the bundle declares: {message}"
    );
    assert!(
        message.contains(&format!("{:#018x}", PromoProps::CONTRACT_HASH)),
        "the message carries the hash the binary has: {message}"
    );
    assert!(
        message.contains("PromoProps"),
        "the message names the props type: {message}"
    );
}

#[test]
fn ts_a_module_the_bundle_does_not_carry_names_the_ids_it_does() {
    // The bundler is what checks a module has a default export; a module that
    // never reached the bundle is this error, which is why it lists what did.
    let handle = runtime();
    let error = Mount::new::<NoProps>("src/missing.tsx", NoProps {})
        .try_mount(&environment(&handle))
        .expect_err("the bundle carries no such module");

    let message = error.to_string();
    assert!(message.contains("src/missing.tsx"), "{message}");
    assert!(
        message.contains(MODULE),
        "the message lists what the bundle does carry: {message}"
    );
}

#[test]
fn ts_a_mount_with_no_runtime_installed_names_the_module() {
    let error = Mount::new::<NoProps>(MODULE, NoProps {})
        .try_mount(&Environment::new())
        .expect_err("no runtime is installed");

    let message = error.to_string();
    assert!(message.contains(MODULE), "{message}");
    assert!(message.contains("RuntimeHandle::install"), "{message}");
}

#[test]
fn ts_a_bundle_that_declares_no_contract_cannot_be_mounted_from() {
    // The entry publishes whatever `__waterui_test_contracts` holds, so an
    // unseeded runtime is a bundle with an empty manifest.
    let host = Environment::new()
        .store::<ColorScheme, Computed<ColorScheme>>(Computed::constant(ColorScheme::Light));
    let runtime = TsRuntime::new(host, Components).expect("the runtime constructs");
    runtime
        .load(BUNDLE)
        .unwrap_or_else(|error| panic!("loading the bundle: {error}"));
    let handle = RuntimeHandle::new(runtime);

    let error = Mount::new::<NoProps>(MODULE, NoProps {})
        .try_mount(&environment(&handle))
        .expect_err("the bundle declares no contract for the module");

    assert!(matches!(error, TsError::MissingContract { .. }), "{error}");
}

#[test]
fn ts_dropping_the_view_disposes_the_mounted_tree() {
    let handle = runtime();
    // Read as text: the assertion is about a count, and comparing it as a
    // float would be comparing floats.
    let disposals = || {
        let value = handle
            .runtime()
            .bridge()
            .engine()
            .eval(
                "String(globalThis.__waterui_test_disposals)",
                "test:disposals.js",
            )
            .expect("reading the module's disposal counter");
        let JsValue::String(text) = value else {
            panic!("the counter read back as {value:?}")
        };
        text
    };

    let mounted = Mount::new::<PromoProps>(MODULE, props(Binding::container(3)))
        .try_mount(&environment(&handle))
        .unwrap_or_else(|error| panic!("mounting: {error}"));

    assert_eq!(disposals(), "0", "the mounted tree is still up");
    drop(mounted);
    assert_eq!(
        disposals(),
        "1",
        "dropping the view ran the mount's dispose, which is what releases the \
         JavaScript tree and everything the mount exported with it"
    );
}

#[test]
fn ts_the_mount_metadata_reaches_the_test_executable() {
    // The channel `tsx!` writes on: a `#[used] static` inside the block the
    // macro expands to, read back from the symbol table of the binary this
    // test runs in — the path the `water` CLI takes through a user crate's
    // rlib.
    let bytes = artifact::meta_static("waterui_meta_tsx_tests_fixtures_promo_tsx");
    let mount = decode_mount(&bytes).expect("the mount payload decodes");

    assert_eq!(mount.module, MODULE);
    assert_eq!(
        mount.props, "PromoProps",
        "the props name comes from the type's resolved schema"
    );
    assert_eq!(mount.contract_hash, PromoProps::CONTRACT_HASH);
}
