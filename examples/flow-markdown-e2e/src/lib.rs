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
    let document_title = document_index.clone().map(current_document_title);
    let document_number = document_index
        .clone()
        .map(normalized_document_index)
        .map(|index| index + 1);
    let document_char_count = document_index.clone().map(current_document_char_count);
    let document_total = MARKDOWN_DOCUMENTS.len() as i32;
    let stream_speed = stream_cps
        .clone()
        .map(|cps| cps.clamp(STREAM_CPS_MIN, STREAM_CPS_MAX));
    let stream_status = streaming
        .clone()
        .map(|streaming| if streaming { "running" } else { "idle" });
    let flow_summary = animation_preset
        .zip(&animation_cps)
        .zip(&stream_cps)
        .zip(&token_fade_enabled)
        .map(|(((preset, cps), stream_cps), token_fade_enabled)| {
            let fade_label = if token_fade_enabled {
                format!("on ({} ms)", stream_interval_ms(stream_cps).clamp(8, 64))
            } else {
                "off".to_string()
            };
            (preset_label(preset), cps.clamp(8, 256), fade_label)
        });
    let flow_preset = flow_summary.clone().map(|(preset, _, _)| preset);
    let flow_cps = flow_summary.clone().map(|(_, cps, _)| cps);
    let flow_fade_label = flow_summary.clone().map(|(_, _, fade_label)| fade_label);
    let token_fade_label = token_fade_enabled.clone().map(|enabled| {
        if enabled {
            "Token fade on"
        } else {
            "Token fade off"
        }
    });
    let document_title_text = document_title.clone();
    let document_number_text = document_number.clone();
    let char_progress_text = char_progress.clone();
    let document_char_count_text = document_char_count.clone();
    let stream_speed_text = stream_speed.clone();
    let stream_status_text = stream_status.clone();
    let flow_preset_text = flow_preset.clone();
    let flow_cps_text = flow_cps.clone();
    let flow_fade_label_text = flow_fade_label.clone();

    scroll(
        vstack((
            text("FlowMarkdown E2E").title(),
            text!(
                "Document: {document_title} ({document_number}/{document_total})",
                document_title = document_title_text,
                document_number = document_number_text,
                document_total = document_total
            )
            .sub_headline(),
            text!(
                "LLM output progress: {char_progress}/{document_char_count} chars",
                char_progress = char_progress_text,
                document_char_count = document_char_count_text
            )
            .caption(),
            text!(
                "LLM stream speed: {stream_speed} chars/s | stream: {stream_status}",
                stream_status = stream_status_text,
                stream_speed = stream_speed_text
            )
            .caption(),
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
            text!(
                "Flow animation preset: {preset} | token reveal CPS: {cps} | token fade: {fade_label}",
                preset = flow_preset_text,
                cps = flow_cps_text,
                fade_label = flow_fade_label_text
            )
            .caption(),
            hstack((
                button("LLM CPS -").with_state(&stream_cps).action(|cps| {
                    cps.set((cps.get() - 4).clamp(STREAM_CPS_MIN, STREAM_CPS_MAX));
                }),
                button("LLM CPS +").with_state(&stream_cps).action(|cps| {
                    cps.set((cps.get() + 4).clamp(STREAM_CPS_MIN, STREAM_CPS_MAX));
                }),
                button("Preset").with_state(&animation_preset).action(|preset| {
                    preset.set((preset.get() + 1).rem_euclid(3));
                }),
                button("CPS -").with_state(&animation_cps).action(|cps| {
                    cps.set((cps.get() - 8).clamp(8, 256));
                }),
                button("CPS +").with_state(&animation_cps).action(|cps| {
                    cps.set((cps.get() + 8).clamp(8, 256));
                }),
                button(text!("{token_fade_label}"))
                    .with_state(&token_fade_enabled)
                    .action(|enabled| {
                        enabled.set(!enabled.get());
                    }),
            ))
            .spacing(10.0),
            Divider,
            Dynamic::watch(
                animation_preset
                    .zip(&animation_cps)
                    .zip(&stream_cps)
                    .zip(&token_fade_enabled),
                {
                    let markdown = markdown.clone();
                    move |(((preset, cps), stream_cps), token_fade_enabled)| {
                        configured_flow(
                            flow_markdown(markdown.clone()),
                            preset,
                            cps,
                            stream_cps,
                            token_fade_enabled,
                        )
                        .padding()
                        .border(Grey, 1.0)
                    }
                },
            ),
        ))
        .padding(),
    )
}

pub fn app(env: Environment) -> App {
    App::new(main, env)
}

waterui_ffi::export!();
