//! `#[js_api]` exposes an object's methods and state to the page.
#![expect(
    clippy::future_not_send,
    reason = "a web view and the API it serves live on the UI thread"
)]
// `clippy::unused_async_trait_impl` is nightly-only: naming it is what silences
// nightly, and allowing `unknown_lints` is what keeps stable — which has never
// heard of it — from rejecting the name under `-D warnings`.
#![allow(unknown_lints)]
#![expect(
    clippy::unused_async,
    clippy::unused_async_trait_impl,
    reason = "`async` is how `#[js_api]` is told a method is a handler rather than \
              state, whether or not a particular body has something to await"
)]

use waterui::webview::{Json, serde};
use waterui::{Binding, Computed, js_api};

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
#[serde(crate = "waterui::webview::serde")]
struct Greeting {
    text: String,
}

struct App {
    theme: Binding<String>,
    count: Binding<u32>,
}

#[js_api(namespace = "app")]
impl App {
    /// Mirrored state: a binding is writable from the page.
    fn theme(&self) -> Binding<String> {
        self.theme.clone()
    }

    /// A derived value the page may read but not assign to.
    fn doubled(&self) -> Computed<u32> {
        use waterui::SignalExt as _;
        self.count.clone().map(|count| count * 2).computed()
    }

    /// A handler the page calls as `await app.greet({ name: "Lexo" })`.
    async fn greet(&self, name: String) -> Json<Greeting> {
        Json(Greeting {
            text: format!("Hi {name}"),
        })
    }

    /// Renaming changes only what the page sees.
    #[js(rename = "reset")]
    async fn reset_counter(&self) {
        self.count.set(0);
    }

    /// Internal helpers stay internal.
    #[js(skip)]
    fn internal(&self, factor: u32) -> u32 {
        self.count.get() * factor
    }
}

/// The declarations are what a page's TypeScript compiles against, so the
/// names, the argument shapes and the read-only marks have to be exact. The
/// application's own `Greeting` is `unknown`: this boundary does not know what
/// is in it, and a declaration TypeScript believes is worse than one that says
/// so.
#[test]
fn the_surface_describes_itself_in_typescript() {
    let declarations = waterui::webview::typescript_declarations::<App>();

    assert!(
        declarations
            .contains(r#"invoke(name: "app.greet", args: { name: string }): Promise<unknown>;"#),
        "{declarations}"
    );
    // Renamed, and with no arguments there is no argument object to send.
    assert!(
        declarations.contains(r#"invoke(name: "app.reset"): Promise<void>;"#),
        "{declarations}"
    );
    // A binding is assignable; a computed is not, and `readonly` is how
    // TypeScript refuses the assignment the page would otherwise be told throws.
    assert!(
        declarations.contains(r#""app.theme": string;"#),
        "{declarations}"
    );
    assert!(
        declarations.contains(r#"readonly "app.doubled": number;"#),
        "{declarations}"
    );
    assert!(
        !declarations.contains("internal"),
        "#[js(skip)] must not appear in the declarations either"
    );
}

/// `#[js(skip)]` takes a method off the page's surface and changes nothing else
/// about it: it keeps its arguments, its return type and its callers in Rust.
#[test]
fn a_skipped_method_stays_an_ordinary_rust_method() {
    let app = App {
        theme: Binding::container(String::from("light")),
        count: Binding::container(2),
    };

    assert_eq!(app.internal(3), 6);
}
