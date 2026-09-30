//! Markdown example for WaterUI.
//!
//! The document sits under a search overlay that demonstrates key bubbling:
//! `on_key_press` on the bar owns Escape while the field keeps focus, and the
//! field's `on_submit` advances to the next match.
use waterui::app::App;
use waterui::key::{Key, KeyHandling, KeyPress, NamedKey};
use waterui::prelude::theme_color::Surface;
use waterui::prelude::*;
use waterui::preview;
use waterui::widget::condition::when;

const DOCUMENT: &str = include_str!("example.md");

/// Byte offsets where `needle` occurs in `DOCUMENT`, case-insensitive.
fn match_offsets(needle: Str) -> Vec<usize> {
    if needle.is_empty() {
        return Vec::new();
    }
    let haystack = DOCUMENT.to_lowercase();
    let needle = needle.to_lowercase();
    haystack
        .match_indices(&needle)
        .map(|(offset, _)| offset)
        .collect()
}

/// A short excerpt of `DOCUMENT` around `offset`, for the match preview.
fn excerpt(offset: usize) -> String {
    let start = offset.saturating_sub(40);
    let start = DOCUMENT[..=start.min(DOCUMENT.len() - 1)]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let end = DOCUMENT[offset..]
        .find('\n')
        .map_or(DOCUMENT.len(), |rest| offset + rest);
    DOCUMENT[start..end].to_string()
}

#[preview]
pub fn demo() -> impl View {
    let open: Binding<bool> = binding(false);
    let query = Binding::container(Str::from(""));
    let current: Binding<usize> = Binding::container(0);
    let focus: Binding<Option<&'static str>> = Binding::default();

    let matches = query.map(match_offsets).computed();
    let status = matches
        .zip(&current)
        .map(|(matches, current)| {
            if matches.is_empty() {
                "No matches".to_string()
            } else {
                format!("{} of {}", current % matches.len() + 1, matches.len())
            }
        })
        .computed();
    let preview = matches
        .zip(&current)
        .map(|(matches, current)| {
            matches
                .get(current % matches.len().max(1))
                .map_or(String::new(), |offset| excerpt(*offset))
        })
        .computed();

    let bar_focus = focus.clone();
    let search_bar = move || {
        let (query, focus, matches) = (query.clone(), bar_focus.clone(), matches.clone());
        vstack((
            hstack((
                field("Search", &query)
                    .on_submit(move |State(current): State<Binding<usize>>| {
                        let total = matches.snapshot().len();
                        if total != 0 {
                            current.set((current.snapshot() + 1) % total);
                        }
                    })
                    .focused(&focus, "search"),
                text(status.clone()).caption(),
                button("Done").action(|State(open): State<Binding<bool>>| {
                    open.set(false);
                }),
            )),
            text(preview.clone()).caption().muted(),
        ))
        .padding()
        .background(Surface)
        .on_key_press(
            |Use(press): Use<KeyPress>, State(open): State<Binding<bool>>| {
                if press.key == Key::Named(NamedKey::Escape) {
                    open.set(false);
                    KeyHandling::Handled
                } else {
                    KeyHandling::Ignored
                }
            },
        )
    };

    zstack((
        // The Find row floats over the document's top-leading corner; the
        // deeper top inset reserves its height so the overlay cannot cover
        // the heading.
        scroll(include_markdown!("example.md").padding_with([52.0, 14.0, 14.0, 14.0])),
        vstack((
            hstack((
                button("Find").action(
                    |State(open): State<Binding<bool>>,
                     State(focus): State<Binding<Option<&'static str>>>| {
                        open.set(true);
                        focus.set(Some("search"));
                    },
                ),
                spacer(),
            )),
            when(open.clone(), search_bar),
            spacer(),
        ))
        .padding(),
    ))
    .state(&open)
    .state(&current)
    .state(&focus)
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
