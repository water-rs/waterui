//! `WaterUI` widgets through `WidgetKit`.
//!
//! A widget is declared by attributing a timeline-producing function with
//! `#[waterui::widget]`:
//!
//! ```rust,ignore
//! use waterui::widgets::*;
//!
//! #[waterui::widget(name = "Steps", description = "Today's step count")]
//! fn steps(context: WidgetContext) -> impl WidgetContent {
//!     vstack((
//!         text("Steps").font(Font::style(FontStyle::Headline)),
//!         text("6,241").font(Font::style(FontStyle::LargeTitle)),
//!     ))
//!     .alignment(HorizontalAlignment::Leading)
//! }
//! ```
//!
//! The function's return value must implement [`IntoTimeline`]: any
//! [`WidgetContent`] produces a single-entry timeline dated "now"; a
//! [`Timeline`] sets dated entries and a reload policy explicitly.
//!
//! The authoring surface is deliberately closed: [`WidgetContent`] and
//! [`WidgetChildren`] are implemented only by the schema's node types, so a
//! `WaterUI` `View` in widget position is a compile-time error — `WidgetKit`
//! cannot host a `UIView`/`NSView` tree, and snapshots carry resolved
//! values only. `water` packages the declared widgets as a generated
//! `WidgetKit` extension rendered by the crate's native `SwiftUI` sources.

extern crate alloc;

mod authoring;
mod declaration;
#[doc(hidden)]
pub mod ffi;
mod schema;

pub use authoring::{
    HStack, IntoTimeline, VStack, WidgetChildren, WidgetContent, ZStack, fill, hstack, intent,
    link, text, vstack, zstack,
};
pub use declaration::{IntentDeclaration, WidgetDeclaration};
pub use schema::*;

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
        const _: () = {
            const NAME: &[u8] = concat!(
                "waterui_meta_widget_",
                env!("CARGO_CRATE_NAME"),
                "_",
                $fn_name
            )
            .as_bytes();
            const PAYLOAD: &[u8] = $payload.as_bytes();
            const LEN: usize = NAME.len() + PAYLOAD.len() + 2;
            $crate::meta::dir_static! {
                static META: [u8; LEN] = $crate::meta::dir_entry::<LEN>(NAME, PAYLOAD);
            }
        };
        $crate::pastey::paste! {
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
    ($hash:expr, $payload:expr, [$ptr:ident, $len:ident] => $body:block) => {
        const _: () = {
            const NAME: &[u8] =
                concat!("waterui_meta_intent_", env!("CARGO_CRATE_NAME"), "_", $hash).as_bytes();
            const PAYLOAD: &[u8] = $payload.as_bytes();
            const LEN: usize = NAME.len() + PAYLOAD.len() + 2;
            $crate::meta::dir_static! {
                static META: [u8; LEN] = $crate::meta::dir_entry::<LEN>(NAME, PAYLOAD);
            }
        };
        $crate::pastey::paste! {
            #[doc(hidden)]
            #[allow(unsafe_code)]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_intent_ env!("CARGO_CRATE_NAME") _ $hash>](
                $ptr: *const u8,
                $len: usize,
            ) $body
        }
    };
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct DummyIntent;

    impl WidgetIntent for DummyIntent {
        fn perform(self) {}
    }

    impl DeclaredWidgetIntent for DummyIntent {
        const KIND: &'static str = "mark-done";
    }

    #[test]
    fn timeline_serializes_to_the_swift_contract() {
        let timeline = Timeline::new([TimelineEntry::new(
            Date::unix_timestamp(1_700_000_000.0),
            vstack((
                text("Hello").font(Font::style(FontStyle::Headline)),
                fill(Color::srgb(1.0, 0.5, 0.0)),
            ))
            .alignment(HorizontalAlignment::Leading)
            .spacing(8.0),
        )])
        .policy(ReloadPolicy::Never);

        let json = serde_json::to_value(&timeline).unwrap();
        let entry = &json["entries"][0];
        assert_eq!(entry["date"], 1_700_000_000.0);
        assert_eq!(json["policy"]["type"], "never");
        assert_eq!(entry["content"]["type"], "stack");
        assert_eq!(entry["content"]["axis"], "vertical");
        assert_eq!(entry["content"]["spacing"], 8.0);
        assert_eq!(entry["content"]["alignment"]["horizontal"], "leading");
        let children = &entry["content"]["children"];
        assert_eq!(children[0]["type"], "text");
        assert_eq!(children[0]["content"], "Hello");
        assert_eq!(children[0]["font"]["kind"], "named");
        assert_eq!(children[0]["font"]["style"], "headline");
        assert_eq!(children[1]["type"], "color");
        assert_eq!(children[1]["kind"], "srgb");
        assert_eq!(children[1]["red"], 1.0);
    }

    #[test]
    fn widget_context_decodes_the_extensions_json() {
        let context: WidgetContext = serde_json::from_str(
            r#"{"family":"accessoryRectangular","displaySize":{"width":170.0,"height":44.0}}"#,
        )
        .unwrap();
        assert_eq!(context.family(), WidgetFamily::AccessoryRectangular);
        assert_eq!(context.display_size().width, 170.0);
    }

    #[test]
    fn every_node_kind_round_trips() {
        let nodes = vec![
            Node::Text(
                text("body")
                    .color(Color::PRIMARY)
                    .line_limit(2)
                    .font(Font::system(17.0, FontWeight::Semibold)),
            ),
            Node::Image(Image::system("figure.walk").color(Color::ACCENT)),
            Node::Image(Image::data([1_u8, 2, 3])),
            Node::Stack(hstack((text("a"), text("b"))).into()),
            Node::Shape(Shape::rounded_rectangle(12.0).fill(Color::SECONDARY)),
            Node::Color(Color::srgb_u8(0x33, 0x66, 0xcc)),
            Node::Link(Link::new("waterui://open", text("open"))),
            Node::Intent(Intent::new(
                &DummyIntent,
                Shape::circle().fill(Color::ACCENT),
            )),
        ];
        for node in nodes {
            let json = serde_json::to_string(&node).unwrap();
            let back: Node = serde_json::from_str(&json).unwrap();
            assert_eq!(back, node, "round-trip mismatch for {json}");
        }
    }

    #[test]
    fn widget_declaration_decodes_a_record_payload() {
        let declaration: WidgetDeclaration = serde_json::from_str(
            r#"{"fn":"steps","kind":"steps","displayName":"Steps","description":"Today","families":["systemSmall","systemMedium"]}"#,
        )
        .unwrap();
        assert_eq!(declaration.function(), "steps");
        assert_eq!(declaration.families(), ["systemSmall", "systemMedium"]);
    }
}
