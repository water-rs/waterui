//! FlowMarkdown end-to-end playground.
use std::time::Duration;

use waterui::animation::Animation;
use waterui::app::App;
use waterui::prelude::*;
use waterui::task::{sleep, spawn_local};

struct MarkdownDocument {
    title: &'static str,
    body: &'static str,
}

const MARKDOWN_DOCUMENTS: [MarkdownDocument; 3] = [
    MarkdownDocument {
        title: "Ops digest",
        body: include_str!("markdown/llm_ops_digest.md"),
    },
    MarkdownDocument {
        title: "Release brief",
        body: include_str!("markdown/llm_release_brief.md"),
    },
    MarkdownDocument {
        title: "Incident report",
        body: include_str!("markdown/llm_incident_report.md"),
    },
];

const STREAM_CPS_MIN: i32 = 4;
const STREAM_CPS_MAX: i32 = 128;

fn stream_interval_ms(stream_cps: i32) -> u64 {
    let cps = stream_cps.clamp(STREAM_CPS_MIN, STREAM_CPS_MAX) as u64;
    let nanos = 1_000_000_000u64 / cps;
    (nanos / 1_000_000).max(8)
}

fn token_fade_animation(stream_cps: i32, enabled: bool) -> Option<Animation> {
    if !enabled {
        return None;
    }

    let fade_ms = stream_interval_ms(stream_cps).clamp(8, 64);
    Some(Animation::ease_in_out(Duration::from_millis(fade_ms)))
}

fn normalized_document_index(index: i32) -> usize {
    let count = MARKDOWN_DOCUMENTS.len();
    assert!(
        !(count == 0),
        "flow-markdown-e2e requires at least one markdown document"
    );

    index.rem_euclid(count as i32) as usize
}

fn current_document(index: i32) -> &'static MarkdownDocument {
    &MARKDOWN_DOCUMENTS[normalized_document_index(index)]
}

fn current_document_body(index: i32) -> &'static str {
    current_document(index).body
}

fn current_document_title(index: i32) -> &'static str {
    current_document(index).title
}

fn current_document_char_count(index: i32) -> i32 {
    current_document_body(index).chars().count() as i32
}

fn cancel_stream(streaming: &Binding<bool>, stream_revision: &Binding<i32>) {
    stream_revision.set(stream_revision.get().wrapping_add(1));
    streaming.set(false);
}

fn reset_stream(markdown: &Binding<Str>, char_progress: &Binding<i32>) {
    markdown.set(Str::from_static(""));
    char_progress.set(0);
}

fn load_full_document(
    markdown: &Binding<Str>,
    char_progress: &Binding<i32>,
    stream_revision: &Binding<i32>,
    streaming: &Binding<bool>,
    document_index: i32,
) {
    cancel_stream(streaming, stream_revision);
    let doc = current_document_body(document_index);
    markdown.set(Str::from_static(doc));
    char_progress.set(current_document_char_count(document_index));
}

fn start_character_stream(
    markdown: Binding<Str>,
    char_progress: Binding<i32>,
    stream_revision: Binding<i32>,
    streaming: Binding<bool>,
    document_index: Binding<i32>,
    stream_cps: Binding<i32>,
) {
    let revision = stream_revision.get().wrapping_add(1);
    stream_revision.set(revision);
    streaming.set(true);
    reset_stream(&markdown, &char_progress);

    let document = current_document_body(document_index.get());
    let char_end_offsets: Vec<usize> = document
        .char_indices()
        .map(|(start, ch)| start + ch.len_utf8())
        .collect();
    assert!(
        !(char_end_offsets.is_empty()),
        "flow-markdown-e2e markdown document must not be empty"
    );

    spawn_local(async move {
        for (index, end_offset) in char_end_offsets.into_iter().enumerate() {
            if stream_revision.get() != revision {
                return;
            }

            markdown.set(Str::from_static(&document[..end_offset]));
            char_progress.set((index + 1) as i32);

            let cps = stream_cps.get().clamp(STREAM_CPS_MIN, STREAM_CPS_MAX) as u64;
            let interval = Duration::from_nanos(1_000_000_000u64 / cps);
            sleep(interval).await;
        }

        if stream_revision.get() == revision {
            streaming.set(false);
        }
    })
    .detach();
}

fn preset_from_index(index: i32) -> FlowAnimationPreset {
    match index.rem_euclid(3) {
        0 => FlowAnimationPreset::AssistantDefault,
        1 => FlowAnimationPreset::Minimal,
        _ => FlowAnimationPreset::None,
    }
}

fn preset_label(index: i32) -> &'static str {
    match preset_from_index(index) {
        FlowAnimationPreset::AssistantDefault => "assistant",
        FlowAnimationPreset::Minimal => "minimal",
        FlowAnimationPreset::None => "none",
    }
}

fn configured_flow(
    view: FlowMarkdown,
    preset_index: i32,
    cps: i32,
    stream_cps: i32,
    token_fade_enabled: bool,
) -> FlowMarkdown {
    let cps = cps.clamp(8, 256) as u32;
    let batch_ms = stream_interval_ms(stream_cps).clamp(8, 40);
    let token_fade = token_fade_animation(stream_cps, token_fade_enabled);
    let text_policy = FlowAnimationPolicy::Typewriter {
        cps,
        batch_ms,
        fade_in: token_fade.clone(),
    };

    let mut configured = view
        .stream(FlowStreamMode::AppendOnly)
        .preset(preset_from_index(preset_index))
        .table_policy(FlowTablePolicy::NoAnimationReadablePending)
        .token_fade_in(token_fade);

    for kind in [
        FlowElementKind::Text,
        FlowElementKind::Heading,
        FlowElementKind::ListItem,
        FlowElementKind::Quote,
        FlowElementKind::Link,
    ] {
        configured = configured.override_animation(kind, text_policy.clone());
    }

    configured
}

fn main() -> impl View {
    let markdown: Binding<Str> = Binding::container(Str::from_static(""));
    let document_index = Binding::i32(0);
    let char_progress = Binding::i32(0);
    let streaming = Binding::bool(false);
    let stream_revision = Binding::i32(0);
    let stream_cps = Binding::i32(30);
    let animation_preset = Binding::i32(0);
    let animation_cps = Binding::i32(64);
    let token_fade_enabled = Binding::bool(true);
    let animation_revision = Binding::i32(0);

    scroll(
        vstack((
            text("FlowMarkdown E2E").title(),
            Dynamic::watch(document_index.clone(), |index| {
                AnyView::new(
                    text(format!(
                        "Document: {} ({}/{})",
                        current_document_title(index),
                        normalized_document_index(index) + 1,
                        MARKDOWN_DOCUMENTS.len(),
                    ))
                    .sub_headline(),
                )
            }),
            Dynamic::watch(char_progress.clone(), {
                let document_index = document_index.clone();
                move |progress| {
                    AnyView::new(
                        text(format!(
                            "LLM output progress: {progress}/{} chars",
                            current_document_char_count(document_index.get()),
                        ))
                        .caption(),
                    )
                }
            }),
            Dynamic::watch(stream_cps.clone(), {
                let streaming = streaming.clone();
                move |cps| {
                    let status = if streaming.get() { "running" } else { "idle" };
                    AnyView::new(
                        text(format!(
                            "LLM stream speed: {} chars/s | stream: {status}",
                            cps.clamp(STREAM_CPS_MIN, STREAM_CPS_MAX),
                        ))
                        .caption(),
                    )
                }
            }),
            hstack((
                button("Prev doc").action({
                    let document_index = document_index.clone();
                    let markdown = markdown.clone();
                    let char_progress = char_progress.clone();
                    let stream_revision = stream_revision.clone();
                    let streaming = streaming.clone();
                    move || {
                        cancel_stream(&streaming, &stream_revision);
                        document_index.set(document_index.get() - 1);
                        reset_stream(&markdown, &char_progress);
                    }
                }),
                button("Next doc").action({
                    let document_index = document_index.clone();
                    let markdown = markdown.clone();
                    let char_progress = char_progress.clone();
                    let stream_revision = stream_revision.clone();
                    let streaming = streaming.clone();
                    move || {
                        cancel_stream(&streaming, &stream_revision);
                        document_index.set(document_index.get() + 1);
                        reset_stream(&markdown, &char_progress);
                    }
                }),
                button("Start stream")
                    .action({
                        let markdown = markdown.clone();
                        let char_progress = char_progress.clone();
                        let stream_revision = stream_revision.clone();
                        let streaming = streaming.clone();
                        let document_index = document_index.clone();
                        let stream_cps = stream_cps.clone();
                        move || {
                            start_character_stream(
                                markdown.clone(),
                                char_progress.clone(),
                                stream_revision.clone(),
                                streaming.clone(),
                                document_index.clone(),
                                stream_cps.clone(),
                            );
                        }
                    })
                    .bordered_prominent(),
                button("Load full").action({
                    let markdown = markdown.clone();
                    let char_progress = char_progress.clone();
                    let stream_revision = stream_revision.clone();
                    let streaming = streaming.clone();
                    let document_index = document_index.clone();
                    move || {
                        load_full_document(
                            &markdown,
                            &char_progress,
                            &stream_revision,
                            &streaming,
                            document_index.get(),
                        );
                    }
                }),
                button("Reset").action({
                    let markdown = markdown.clone();
                    let char_progress = char_progress.clone();
                    let stream_revision = stream_revision.clone();
                    let streaming = streaming.clone();
                    move || {
                        cancel_stream(&streaming, &stream_revision);
                        reset_stream(&markdown, &char_progress);
                    }
                }),
            ))
            .spacing(10.0),
            Dynamic::watch(animation_revision.clone(), {
                let animation_preset = animation_preset.clone();
                let animation_cps = animation_cps.clone();
                let token_fade_enabled = token_fade_enabled.clone();
                let stream_cps = stream_cps.clone();
                move |_| {
                    let fade_label = if token_fade_enabled.get() {
                        format!(
                            "on ({} ms)",
                            stream_interval_ms(stream_cps.get()).clamp(8, 64)
                        )
                    } else {
                        "off".to_string()
                    };
                    AnyView::new(
                        text(format!(
                            "Flow animation preset: {} | token reveal CPS: {} | token fade: {}",
                            preset_label(animation_preset.get()),
                            animation_cps.get().clamp(8, 256),
                            fade_label,
                        ))
                        .caption(),
                    )
                }
            }),
            hstack((
                button("LLM CPS -")
                    .with_state(&stream_cps)
                    .with_state(&animation_revision)
                    .action(|(cps, revision)| {
                        cps.set((cps.get() - 4).clamp(STREAM_CPS_MIN, STREAM_CPS_MAX));
                        revision.set(revision.get() + 1);
                    }),
                button("LLM CPS +")
                    .with_state(&stream_cps)
                    .with_state(&animation_revision)
                    .action(|(cps, revision)| {
                        cps.set((cps.get() + 4).clamp(STREAM_CPS_MIN, STREAM_CPS_MAX));
                        revision.set(revision.get() + 1);
                    }),
                button("Preset")
                    .with_state(&animation_preset)
                    .with_state(&animation_revision)
                    .action(|(preset, revision)| {
                        preset.set((preset.get() + 1).rem_euclid(3));
                        revision.set(revision.get() + 1);
                    }),
                button("CPS -")
                    .with_state(&animation_cps)
                    .with_state(&animation_revision)
                    .action(|(cps, revision)| {
                        cps.set((cps.get() - 8).clamp(8, 256));
                        revision.set(revision.get() + 1);
                    }),
                button("CPS +")
                    .with_state(&animation_cps)
                    .with_state(&animation_revision)
                    .action(|(cps, revision)| {
                        cps.set((cps.get() + 8).clamp(8, 256));
                        revision.set(revision.get() + 1);
                    }),
                button(Dynamic::watch(token_fade_enabled.clone(), |enabled| {
                    if enabled {
                        AnyView::new(text("Token fade on"))
                    } else {
                        AnyView::new(text("Token fade off"))
                    }
                }))
                .with_state(&token_fade_enabled)
                .with_state(&animation_revision)
                .action(|(enabled, revision)| {
                    enabled.set(!enabled.get());
                    revision.set(revision.get() + 1);
                }),
            ))
            .spacing(10.0),
            Divider,
            Dynamic::watch(animation_revision.clone(), {
                let markdown = markdown.clone();
                let animation_preset = animation_preset.clone();
                let animation_cps = animation_cps.clone();
                let stream_cps = stream_cps.clone();
                let token_fade_enabled = token_fade_enabled.clone();
                move |_| {
                    AnyView::new(
                        configured_flow(
                            flow_markdown(markdown.clone()),
                            animation_preset.get(),
                            animation_cps.get(),
                            stream_cps.get(),
                            token_fade_enabled.get(),
                        )
                        .padding()
                        .border(Grey, 1.0),
                    )
                }
            }),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
