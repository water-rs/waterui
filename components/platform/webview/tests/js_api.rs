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

use std::collections::{BTreeSet, VecDeque};
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::path::PathBuf;

use waterui::webview::{Bytes, Json, TsType, serde};
use waterui::{Binding, Computed, Signal, js_api};

type Id = u64;
type ThemeSignal = Binding<String>;

#[derive(serde::Serialize, serde::Deserialize, TsType, PartialEq, Debug)]
#[serde(crate = "waterui::webview::serde")]
enum Role {
    Admin,
    Guest,
}

#[derive(serde::Serialize, serde::Deserialize, TsType, PartialEq, Debug)]
#[serde(crate = "waterui::webview::serde")]
enum Notice {
    Empty,
    Message { body: String },
}

#[derive(serde::Serialize, serde::Deserialize, TsType, PartialEq, Debug)]
#[serde(crate = "waterui::webview::serde")]
struct Greeting {
    text: String,
    role: Role,
}

#[derive(Debug)]
struct ApiError;

impl Display for ApiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str("api error")
    }
}

struct App {
    theme: ThemeSignal,
    count: Binding<u32>,
    last_id: Binding<Id>,
}

#[js_api(namespace = "app")]
impl App {
    /// Mirrored state through a type alias: `TypeSchema::Signal` marks it writable.
    fn theme(&self) -> ThemeSignal {
        self.theme.clone()
    }

    /// A derived value the page may read but not assign to.
    fn doubled(&self) -> Computed<u32> {
        use waterui::SignalExt as _;
        self.count.clone().map(|count| count * 2).computed()
    }

    /// Aliased 64-bit integer mirrored state.
    fn last_id(&self) -> Computed<Id> {
        use waterui::SignalExt as _;
        self.last_id.clone().computed()
    }

    /// A handler the page calls as `await app.greet({ name: "Lexo" })`.
    async fn greet(&self, name: String) -> Json<Greeting> {
        Json(Greeting {
            text: format!("Hi {name}"),
            role: Role::Guest,
        })
    }

    /// Aliased parameter and return type resolve through `TsType::SCHEMA`.
    async fn echo_id(
        &self,
        id: Id,
        tags: BTreeSet<String>,
        queue: VecDeque<Id>,
        path: PathBuf,
    ) -> Result<Json<Id>, ApiError> {
        let _ = (tags, queue, path);
        Ok(Json(id))
    }

    /// Tagged enum payload renders with its real discriminant shape.
    async fn latest_notice(&self) -> Json<Notice> {
        Json(Notice::Empty)
    }

    /// Raw binary return projects as `Uint8Array`.
    async fn download(&self) -> Bytes {
        Bytes(Vec::new())
    }

    /// Renaming changes only what the page sees.
    #[js(rename = "reset")]
    async fn reset_counter(&self) {
        self.count.set(0);
    }

    /// Internal helpers stay internal.
    #[js(skip)]
    fn internal(&self, factor: u32) -> u32 {
        self.count.snapshot() * factor
    }
}

/// The declarations come straight from `TsType::SCHEMA`, so type aliases,
/// writability and user-defined structs and enums all project with their real
/// TypeScript shapes.
#[test]
fn the_surface_describes_itself_in_typescript() {
    let declarations = waterui::webview::typescript_declarations::<App>();

    assert!(
        declarations.contains("interface Greeting {\n  text: string;\n  role: Role;\n}"),
        "{declarations}"
    );
    assert!(
        declarations.contains(r#"type Role = "admin" | "guest";"#),
        "{declarations}"
    );
    assert!(
        declarations.contains(
            r#"type Notice = { type: "Empty" } | { type: "Message"; value: { body: string } };"#
        ),
        "{declarations}"
    );
    assert!(
        declarations
            .contains(r#"invoke(name: "app.greet", args: { name: string }): Promise<Greeting>;"#),
        "{declarations}"
    );
    assert!(
        declarations.contains(
            r#"invoke(name: "app.echo_id", args: { id: number | bigint; tags: string[]; queue: (number | bigint)[]; path: string }): Promise<number | bigint>;"#
        ),
        "{declarations}"
    );
    assert!(
        declarations.contains(r#"invoke(name: "app.latest_notice"): Promise<Notice>;"#),
        "{declarations}"
    );
    assert!(
        declarations.contains(r#"invoke(name: "app.download"): Promise<Uint8Array>;"#),
        "{declarations}"
    );
    // Renamed, and with no arguments there is no argument object to send.
    assert!(
        declarations.contains(r#"invoke(name: "app.reset"): Promise<void>;"#),
        "{declarations}"
    );
    // A binding (even behind a type alias) is assignable; a computed is not.
    assert!(
        declarations.contains(r#"    "app.theme": string;"#),
        "{declarations}"
    );
    assert!(
        declarations.contains(r#"    readonly "app.doubled": number;"#),
        "{declarations}"
    );
    assert!(
        declarations.contains(r#"    readonly "app.last_id": number | bigint;"#),
        "{declarations}"
    );
    assert!(
        !declarations.contains("internal"),
        "#[js(skip)] must not appear in the declarations either"
    );
    // Mirrored values arrive with the page's first pull; the boundary it
    // awaits before reading them is part of the surface.
    assert!(
        declarations.contains("readonly ready: Promise<void>;"),
        "{declarations}"
    );
}

/// `#[js(skip)]` takes a method off the page's surface and changes nothing else
/// about it: it keeps its arguments, its return type and its callers in Rust.
#[test]
fn a_skipped_method_stays_an_ordinary_rust_method() {
    let app = App {
        theme: Binding::new(String::from("light")),
        count: Binding::new(2),
        last_id: Binding::new(7),
    };

    assert_eq!(app.internal(3), 6);
}
