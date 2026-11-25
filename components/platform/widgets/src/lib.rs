//! `WaterUI` home-screen widgets with shared state and timeline reload.
//!
//! A widget is declared by attributing a timeline-producing function with
//! `#[waterui::widget]`:
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//! use waterui::widgets::*;
//!
//! #[waterui::widget(name = "Steps", description = "Today's step count")]
//! fn steps(context: WidgetContext) -> impl WidgetContent {
//!     use waterui::widgets::prelude::*;
//!
//!     let steps: u32 = context.shared_store().get("steps").unwrap_or(0);
//!     vstack((
//!         text("Steps").headline(),
//!         spacer(),
//!         text(Date::now()).relative(),
//!         image_system("figure.walk").a11y_label("Walking"),
//!         text(format!("{steps}")).large_title(),
//!     ))
//!     .alignment(HorizontalAlignment::Leading)
//!     .padding()
//!     .background(Color::ACCENT)
//! }
//!
//! // From the application, write shared state and ask the host to reload:
//! let store = waterui::widgets::shared_store()?;
//! store.set("steps", &6_241_u32)?;
//! waterui::widgets::reload("steps");
//! ```
//!
//! The crate root (`waterui::widgets::*`) exports only widget-specific types
//! and host functions so it can be glob-imported alongside `waterui::prelude::*`
//! without name collisions. Inside a widget module or `#[waterui::widget]`
//! function body, import [`prelude`] (`use waterui::widgets::prelude::*;`) for
//! the aligned widget-authoring vocabulary (`vstack`, `text`, `spacer`,
//! `Color`, `Font`, …).
//!
//! The function's return value must implement [`IntoTimeline`]: any
//! [`WidgetContent`] produces a single-entry timeline dated "now"; a
//! [`Timeline`] sets dated entries and a reload policy explicitly.
//!
//! The authoring surface is deliberately closed: [`WidgetContent`] and
//! [`WidgetChildren`] are implemented only by the schema's node types, so a
//! `WaterUI` `View` in widget position is a compile-time error — `WidgetKit`
//! cannot host a `UIView`/`NSView` tree, and snapshots carry resolved
//! values only. `water` provisions a shared container (App Group on Apple,
//! app-private widget storage on Android) and packages Apple declarations as a
//! generated `WidgetKit` extension rendered by the crate's native `SwiftUI`
//! sources.

extern crate alloc;

mod authoring;
mod declaration;
#[doc(hidden)]
pub mod ffi;
pub mod schema;
mod store;

/// Complete widget-authoring vocabulary mirroring `waterui::prelude::*`.
///
/// Import `use waterui::widgets::prelude::*;` inside a widget module or
/// `#[waterui::widget]` function body to build snapshots with `vstack`,
/// `hstack`, `zstack`, `text`, `spacer`, `image_system`, `Color`, `Font`, and
/// [`WidgetExt`] modifiers.
pub mod prelude {
    pub use crate::authoring::{
        DateText, HStack, IntoLabelText, IntoText, IntoTimeline, VStack, WidgetChildren,
        WidgetContent, WidgetExt, ZStack, fill, hstack, image_data, image_system, intent, link,
        spacer, spacer_min, text, vstack, zstack,
    };
    pub use crate::schema::{
        AccessibilityLabel, Alignment, Axis, Background, Color, Date, DateStyle,
        DeclaredWidgetIntent, EdgeInsets, Font, FontStyle, FontWeight, Frame, HorizontalAlignment,
        Image, ImageSource, Intent, LabelText, Link, MaxLength, Node, Padding, ReloadPolicy,
        SemanticColor, Shape, ShapeKind, Size, Spacer, Stack, Text, TextContent, Timeline,
        TimelineEntry, VerticalAlignment, WidgetContext, WidgetFamily, WidgetIntent,
    };
    pub use crate::store::{WidgetStore, WidgetStoreError, reload, reload_all, shared_store};
}

// Root exports stay collision-free with `waterui::prelude::*` so application
// files can write `use waterui::prelude::*; use waterui::widgets::*;`.
pub use authoring::{
    DateText, IntoLabelText, IntoTimeline, WidgetChildren, WidgetContent, WidgetExt, fill,
    image_data, image_system, intent,
};
pub use declaration::{IntentDeclaration, WidgetDeclaration};
pub use schema::{
    Date, DateStyle, DeclaredWidgetIntent, Intent, LabelText, MaxLength, Node, ReloadPolicy,
    TextContent, Timeline, TimelineEntry, WidgetContext, WidgetFamily, WidgetIntent,
};
pub use store::{WidgetStore, WidgetStoreError, reload, reload_all, shared_store};

#[doc(hidden)]
pub use pastey;
/// Re-exported so `#[widget_intent]` can derive `Serialize`/`Deserialize` on
/// the declared action type without the calling crate having to depend on
/// `serde` itself.
#[doc(hidden)]
pub use serde;
#[doc(hidden)]
pub use serde_json;
#[doc(hidden)]
pub use waterui_meta as meta;

/// Where the extension's native `SwiftUI`/`WidgetKit` sources live.
///
/// The build script records `CARGO_MANIFEST_DIR/swift` so the `water` CLI —
/// which reads this record out of the composed application artifact — can
/// compile the extension from the same sources the crate ships, on any
/// machine that has them (path, git or registry dependency alike). The
/// record is not debug-gated, because packaging reads it from release
/// artifacts too; `dir_static!` keeps it in the archive and out of the
/// linked image. Only Apple packaging builds a `WidgetKit` extension, so
/// other targets carry no record.
#[cfg(target_vendor = "apple")]
const SWIFT_SOURCES_RECORD_NAME: &[u8] = b"waterui_meta_widgets_swift_sources";
#[cfg(target_vendor = "apple")]
const SWIFT_SOURCES_RECORD_LEN: usize =
    SWIFT_SOURCES_RECORD_NAME.len() + env!("WATERUI_WIDGETS_SWIFT_SOURCES").len() + 2;

#[cfg(target_vendor = "apple")]
meta::dir_static! {
    static WIDGETS_SWIFT_SOURCES: [u8; SWIFT_SOURCES_RECORD_LEN] =
        meta::dir_entry::<SWIFT_SOURCES_RECORD_LEN>(
            SWIFT_SOURCES_RECORD_NAME,
            env!("WATERUI_WIDGETS_SWIFT_SOURCES").as_bytes(),
        );
}

/// Emits the `waterui_widget_*` export and its `waterui_meta_widget_*`
/// declaration record for a `#[widget]`-attributed function.
///
/// `#waterui::widgets::__export_widget!` expands the exported symbol
/// `waterui_widget_<crate>_<fn>` — a `#[no_mangle]` C entry point the
/// generated extension resolves — plus the record `water` reads while it
/// assembles the extension. `$body` is the serialization glue the
/// `#[widget]` macro generates: decode the [`WidgetContext`], call the
/// function, serialize its [`Timeline`].
#[doc(hidden)]
#[macro_export]
macro_rules! __export_widget {
    ($fn_name:expr, $payload:expr, [$ctx_ptr:ident, $ctx_len:ident, $out:ident] => $body:block) => {
        $crate::pastey::paste! {
            // `dir_static!` marks the record `#[used]` on every target: in a
            // staticlib a `pub` static has internal linkage, and without the
            // attribute an unreferenced record is dropped from the ELF or
            // COFF object (water-rs/waterui#3905).
            #[doc(hidden)]
            const [<__WUI_WIDGET_META_LEN_ $fn_name>]: usize = concat!(
                "waterui_meta_widget_",
                env!("CARGO_CRATE_NAME"),
                "_",
                $fn_name
            )
            .as_bytes()
            .len() + $payload.as_bytes().len() + 2;
            $crate::meta::dir_static! {
                /// The widget declaration record `water` reads out of the rlib.
                #[doc(hidden)]
                #[expect(
                    non_upper_case_globals,
                    reason = "tooling names the record by its `waterui_meta_` item name"
                )]
                pub static [<waterui_meta_widget_ env!("CARGO_CRATE_NAME") _ $fn_name>]:
                    [u8; [<__WUI_WIDGET_META_LEN_ $fn_name>]] =
                    $crate::meta::dir_entry::<{ [<__WUI_WIDGET_META_LEN_ $fn_name>] }>(
                        concat!(
                            "waterui_meta_widget_",
                            env!("CARGO_CRATE_NAME"),
                            "_",
                            $fn_name
                        )
                        .as_bytes(),
                        $payload.as_bytes(),
                    );
            }
            #[doc(hidden)]
            #[allow(unsafe_code)]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_widget_ env!("CARGO_CRATE_NAME") _ $fn_name>](
                $ctx_ptr: *const u8,
                $ctx_len: usize,
                $out: *mut $crate::ffi::Buffer,
            ) {
                // `Buffer` travels through an out-parameter: `@_extern`
                // admits only C-representable types, and a `Swift` struct
                // is not one — but a pointer to its mirrored layout is.
                unsafe { $out.write($body) }
            }
        }
    };
}

/// Emits the `waterui_intent_*` export and its `waterui_meta_intent_*`
/// record for a `#[widget_intent]`-attributed action type.
///
/// `#waterui::widgets::__export_intent!` expands the exported symbol
/// `waterui_intent_<crate>_<hash>` — a `#[no_mangle]` C entry point the
/// generated extension calls through the `WUIIntent_<hash>` type — plus
/// the `waterui_meta_intent_<crate>_<hash>` record `water` reads while it
/// generates the extension. `$hash` is the declared kind's stable
/// `SHA-256` hex digest; `$body` decodes the action from the payload bytes
/// and calls `I::perform`.
#[doc(hidden)]
#[macro_export]
macro_rules! __export_intent {
    ($hash:expr, $payload:expr, [$ptr:ident, $len:ident, $store_ptr:ident, $store_len:ident] => $body:block) => {
        $crate::pastey::paste! {
            #[doc(hidden)]
            const [<__WUI_INTENT_META_LEN_ $hash>]: usize =
                concat!("waterui_meta_intent_", env!("CARGO_CRATE_NAME"), "_", $hash)
                    .as_bytes()
                    .len() + $payload.as_bytes().len() + 2;
            $crate::meta::dir_static! {
                /// The intent declaration record `water` reads out of the rlib.
                #[doc(hidden)]
                #[expect(
                    non_upper_case_globals,
                    reason = "tooling names the record by its `waterui_meta_` item name"
                )]
                pub static [<waterui_meta_intent_ env!("CARGO_CRATE_NAME") _ $hash>]:
                    [u8; [<__WUI_INTENT_META_LEN_ $hash>]] =
                    $crate::meta::dir_entry::<{ [<__WUI_INTENT_META_LEN_ $hash>] }>(
                        concat!("waterui_meta_intent_", env!("CARGO_CRATE_NAME"), "_", $hash)
                            .as_bytes(),
                        $payload.as_bytes(),
                    );
            }
            #[doc(hidden)]
            #[allow(unsafe_code)]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_intent_ env!("CARGO_CRATE_NAME") _ $hash>](
                $ptr: *const u8,
                $len: usize,
                $store_ptr: *const u8,
                $store_len: usize,
            ) $body
        }
    };
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec;
    use std::path::Path;
    use std::time::{Duration, UNIX_EPOCH};

    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::{from_str, to_string, to_value};
    use suiteki::Str;

    use crate::WidgetDeclaration;
    use crate::prelude::*;

    #[derive(Debug, Serialize, Deserialize)]
    struct DummyIntent;

    impl WidgetIntent for DummyIntent {
        fn perform(self, _store: &WidgetStore) {}
    }

    impl DeclaredWidgetIntent for DummyIntent {
        const KIND: &'static str = "mark-done";
    }

    #[test]
    fn timeline_serializes_to_the_host_contract() {
        let timeline = Timeline::new([TimelineEntry::new(
            Date::unix_timestamp(1_700_000_000.0),
            vstack((
                text("Steps").headline(),
                spacer(),
                text(Date::unix_timestamp(1_700_000_060.0)).relative(),
                image_system("figure.walk").a11y_label("Walking"),
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(8.0)
            .padding()
            .background(Color::ACCENT),
        )])
        .policy(ReloadPolicy::Never);

        let json = to_value(&timeline).unwrap();
        let entry = &json["entries"][0];
        assert_eq!(entry["date"], 1_700_000_000.0);
        assert_eq!(json["policy"]["type"], "never");
        assert_eq!(entry["content"]["type"], "background");
        assert_eq!(entry["content"]["background"]["type"], "color");
        assert_eq!(entry["content"]["background"]["kind"], "semantic");
        assert_eq!(entry["content"]["background"]["semantic"], "accent");

        let padded = &entry["content"]["content"];
        assert_eq!(padded["type"], "padding");
        assert!(padded.get("insets").is_none());

        let stack = &padded["content"];
        assert_eq!(stack["type"], "stack");
        assert_eq!(stack["axis"], "vertical");
        assert_eq!(stack["spacing"], 8.0);
        assert_eq!(stack["alignment"]["horizontal"], "leading");

        let children = &stack["children"];
        assert_eq!(children[0]["type"], "text");
        assert_eq!(children[0]["content"]["kind"], "localized");
        assert_eq!(children[0]["content"]["key"], "Steps");
        assert_eq!(children[0]["font"]["kind"], "named");
        assert_eq!(children[0]["font"]["style"], "headline");

        assert_eq!(children[1]["type"], "spacer");

        assert_eq!(children[2]["type"], "text");
        assert_eq!(children[2]["content"]["kind"], "date");
        assert_eq!(children[2]["content"]["date"], 1_700_000_060.0);
        assert_eq!(children[2]["content"]["style"], "relative");

        assert_eq!(children[3]["type"], "accessibilityLabel");
        assert_eq!(children[3]["label"]["kind"], "localized");
        assert_eq!(children[3]["label"]["key"], "Walking");
        assert_eq!(children[3]["content"]["type"], "image");
        assert_eq!(children[3]["content"]["source"]["kind"], "sfSymbol");
        assert_eq!(children[3]["content"]["source"]["name"], "figure.walk");
    }

    #[test]
    fn into_text_distinguishes_localized_verbatim_and_dates() {
        assert_eq!(
            text("Steps").content(),
            &TextContent::Localized {
                key: String::from("Steps")
            }
        );
        assert_eq!(
            text(String::from("6,241")).content(),
            &TextContent::Verbatim {
                text: String::from("6,241")
            }
        );
        assert_eq!(
            text(Str::from("6,241")).content(),
            &TextContent::Verbatim {
                text: String::from("6,241")
            }
        );
        let instant = Date::unix_timestamp(1_700_000_000.0);
        assert_eq!(
            text(instant).relative().content(),
            &TextContent::Date {
                date: instant,
                style: DateStyle::Relative,
            }
        );
    }

    #[test]
    fn widget_context_decodes_the_extensions_json() {
        let context: WidgetContext = from_str(
            r#"{"family":"accessoryRectangular","displaySize":{"width":170.0,"height":44.0},"storePath":"/tmp/waterui-widgets"}"#,
        )
        .unwrap();
        assert_eq!(context.family(), WidgetFamily::AccessoryRectangular);
        assert_eq!(context.display_size().width, 170.0);
        assert_eq!(
            context.shared_store().root(),
            Path::new("/tmp/waterui-widgets")
        );
    }

    #[test]
    fn every_node_kind_round_trips() {
        let nodes = vec![
            Node::Text(
                text("body")
                    .bold(true)
                    .foreground(Color::PRIMARY)
                    .line_limit(2)
                    .font(Font::system(17.0, FontWeight::Semibold)),
            ),
            Node::Text(text(String::from("verbatim")).caption()),
            Node::Text(text(Date::unix_timestamp(1_700_000_000.0)).relative()),
            Node::Image(image_system("figure.walk").foreground(Color::ACCENT)),
            Node::Image(image_data([1_u8, 2, 3])),
            Node::Stack(hstack((text("a"), spacer(), text("b"))).into()),
            Node::Shape(Shape::rounded_rectangle(12.0).fill(Color::SECONDARY)),
            Node::Color(Color::srgb_u8(0x33, 0x66, 0xcc)),
            Node::Padding(text("default").padding()),
            Node::Padding(
                text("custom").padding_with(EdgeInsets::vertical(8.0).with_horizontal(12.0)),
            ),
            Node::Spacer(spacer_min(10.0)),
            Node::Frame(
                text("framed")
                    .min_width(40.0)
                    .max_width(f32::INFINITY)
                    .height(24.0)
                    .alignment(Alignment::TOP_LEADING),
            ),
            Node::Background(
                text("bg")
                    .padding()
                    .background(Shape::rounded_rectangle(8.0).fill(Color::ACCENT)),
            ),
            Node::AccessibilityLabel(image_system("heart.fill").a11y_label("Favorite")),
            Node::Link(Link::new("waterui://open", text("open"))),
            Node::Intent(Intent::new(
                &DummyIntent,
                Shape::circle().fill(Color::ACCENT),
            )),
        ];
        for node in nodes {
            let json = to_string(&node).unwrap();
            let back: Node = from_str(&json).unwrap();
            assert_eq!(back, node, "round-trip mismatch for {json}");
        }
    }

    /// An instant before the Unix epoch is a negative offset, never a panic.
    #[test]
    fn a_date_before_the_epoch_is_negative() {
        let before = UNIX_EPOCH - Duration::from_secs(90);
        assert!((Date::from(before).unix_seconds() + 90.0).abs() < f64::EPSILON);
        let after = UNIX_EPOCH + Duration::from_secs(90);
        assert!((Date::from(after).unix_seconds() - 90.0).abs() < f64::EPSILON);
    }

    /// An action whose `Serialize` fails is a contract violation; the region
    /// then carries an empty payload.
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "the WidgetIntent action `unserializable` must serialize")
    )]
    fn an_unserializable_intent_action_carries_an_empty_payload() {
        struct Unserializable;
        impl Serialize for Unserializable {
            fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("no JSON form"))
            }
        }
        impl<'de> Deserialize<'de> for Unserializable {
            fn deserialize<D: Deserializer<'de>>(_: D) -> Result<Self, D::Error> {
                Ok(Self)
            }
        }
        impl WidgetIntent for Unserializable {
            fn perform(self, _store: &WidgetStore) {}
        }
        impl DeclaredWidgetIntent for Unserializable {
            const KIND: &'static str = "unserializable";
        }

        let region = Intent::new(&Unserializable, Shape::circle().fill(Color::ACCENT));
        assert_eq!(region.kind(), "unserializable");
        assert_eq!(region.payload(), "");
    }

    #[test]
    fn widget_declaration_decodes_a_record_payload() {
        let declaration: WidgetDeclaration = from_str(
            r#"{"fn":"steps","kind":"steps","displayName":"Steps","description":"Today","families":["systemSmall","systemMedium"]}"#,
        )
        .unwrap();
        assert_eq!(declaration.function(), "steps");
        assert_eq!(declaration.families(), ["systemSmall", "systemMedium"]);
    }
}
