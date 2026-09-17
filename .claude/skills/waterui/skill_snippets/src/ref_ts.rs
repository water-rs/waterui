//! Snippets from SKILL.md's "## TypeScript views" section — their own module
//! because they compile against the `ts` feature's surface (`tsx!`, `TsProps`,
//! a `Binding` as a props field), which nothing else in the crate names.
//!
//! `tsx!("./promo.tsx", …)` resolves its path against this file, so the module
//! it names is the real `src/promo.tsx` beside it — the macro stats the file
//! at expansion, and a transcription cannot point at a file that does not
//! exist.

// ---------------------------------------------------------------------------
// SKILL.md § "## TypeScript views" — rust block 17/18
// ---------------------------------------------------------------------------
use waterui::Binding;
use waterui::ts::schema::TsProps;
use waterui::tsx;

#[derive(TsProps)]
struct PromoProps {
    headline: String,
    unread: Binding<u32>,
    #[ts(rename = "onDismiss")]
    on_dismiss: Box<dyn Fn()>,
}

// ---------------------------------------------------------------------------
// SKILL.md § "## TypeScript views" — rust block 18/18
// ---------------------------------------------------------------------------
pub fn mount_promo() -> impl waterui::View {
    let unread = Binding::container(3_u32);
    let view = tsx!(
        "./promo.tsx",
        PromoProps {
            headline: String::from("Welcome back"),
            unread,
            on_dismiss: Box::new(|| {}),
        }
    );
    view
}
