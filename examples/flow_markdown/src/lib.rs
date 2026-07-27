//! Flow Markdown playground.
use std::time::Duration;

use waterui::animation::Animation;
use waterui::app::App;
use waterui::prelude::flow_markdown::FlowMarkdownConfig;
use waterui::prelude::*;
use waterui::preview;
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
const PRIMARY_CONTROL_WIDTH: f32 = 132.0;
const SECONDARY_CONTROL_WIDTH: f32 = 92.0;
const WIDE_CONTROL_WIDTH: f32 = 152.0;
const TOKEN_CONTROL_WIDTH: f32 = 172.0;

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
        (count != 0),
        "flow-markdown example requires at least one markdown document"
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
        "flow-markdown example markdown document must not be empty"
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

fn configured_flow_config(
    preset_index: i32,
    cps: i32,
    stream_cps: i32,
    token_fade_enabled: bool,
) -> FlowMarkdownConfig {
    let cps = cps.clamp(8, 256) as u32;
    let batch_ms = stream_interval_ms(stream_cps).clamp(8, 40);
    let token_fade = token_fade_animation(stream_cps, token_fade_enabled);
    let text_policy = FlowAnimationPolicy::Typewriter {
        cps,
        batch_ms,
        fade_in: token_fade.clone(),
    };

    let mut configured = FlowMarkdownConfig::default()
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

/// Aggregates the bindings that the document-control buttons share, so each
/// button can inject a single `State<StreamControl>` instead of stacking many
/// `State<Binding<T>>` parameters and matching `.state(...)` calls. See
/// `docs/api-style.md` for when to prefer this idiom.
#[derive(Clone)]
struct StreamControl {
    markdown: Binding<Str>,
    document_index: Binding<i32>,
    char_progress: Binding<i32>,
    streaming: Binding<bool>,
    stream_revision: Binding<i32>,
    stream_cps: Binding<i32>,
}

#[preview]
pub fn demo() -> impl View {
    let markdown: Binding<Str> = Binding::container(Str::from_static(""));
    let document_index = Binding::i32(0);
    let char_progress = Binding::i32(0);
    let streaming = Binding::bool(false);
    let stream_revision = Binding::i32(0);
    let stream_cps = Binding::i32(30);
    let animation_preset = Binding::i32(0);
    let animation_cps = Binding::i32(64);
    let token_fade_enabled = Binding::bool(true);

    let control = StreamControl {
        markdown: markdown.clone(),
        document_index: document_index.clone(),
        char_progress: char_progress.clone(),
        streaming: streaming.clone(),
        stream_revision: stream_revision.clone(),
        stream_cps: stream_cps.clone(),
    };
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
    let flow_config = animation_preset
        .zip(&animation_cps)
        .zip(&stream_cps)
        .zip(&token_fade_enabled)
        .map(|(((preset, cps), stream_cps), token_fade_enabled)| {
            configured_flow_config(preset, cps, stream_cps, token_fade_enabled)
        })
        .computed();
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

    vstack((
        text("Flow Markdown").title(),
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
            button("Prev doc")
                .action(|State(c): State<StreamControl>| {
                    cancel_stream(&c.streaming, &c.stream_revision);
                    c.document_index.set(c.document_index.get() - 1);
                    reset_stream(&c.markdown, &c.char_progress);
                })
                .state(&control)
                .width(PRIMARY_CONTROL_WIDTH),
            button("Next doc")
                .action(|State(c): State<StreamControl>| {
                    cancel_stream(&c.streaming, &c.stream_revision);
                    c.document_index.set(c.document_index.get() + 1);
                    reset_stream(&c.markdown, &c.char_progress);
                })
                .state(&control)
                .width(WIDE_CONTROL_WIDTH),
            button("Start stream")
                .bordered_prominent()
                .action(|State(c): State<StreamControl>| {
                    start_character_stream(
                        c.markdown.clone(),
                        c.char_progress.clone(),
                        c.stream_revision.clone(),
                        c.streaming.clone(),
                        c.document_index.clone(),
                        c.stream_cps.clone(),
                    );
                })
                .state(&control)
                .width(PRIMARY_CONTROL_WIDTH),
            button("Load full")
                .action(|State(c): State<StreamControl>| {
                    load_full_document(
                        &c.markdown,
                        &c.char_progress,
                        &c.stream_revision,
                        &c.streaming,
                        c.document_index.get(),
                    );
                })
                .state(&control)
                .width(PRIMARY_CONTROL_WIDTH),
            button("Reset")
                .action(|State(c): State<StreamControl>| {
                    cancel_stream(&c.streaming, &c.stream_revision);
                    reset_stream(&c.markdown, &c.char_progress);
                })
                .state(&control)
                .width(PRIMARY_CONTROL_WIDTH),
        ))
        .spacing(8.0),
        text!(
            "Flow animation preset: {preset} | token reveal CPS: {cps} | token fade: {fade_label}",
            preset = flow_preset_text,
            cps = flow_cps_text,
            fade_label = flow_fade_label_text
        )
        .caption(),
        hstack((
            button("LLM CPS -")
                .action(|State(cps): State<Binding<i32>>| {
                    cps.set((cps.get() - 4).clamp(STREAM_CPS_MIN, STREAM_CPS_MAX));
                })
                .state(&stream_cps)
                .width(WIDE_CONTROL_WIDTH),
            button("LLM CPS +")
                .action(|State(cps): State<Binding<i32>>| {
                    cps.set((cps.get() + 4).clamp(STREAM_CPS_MIN, STREAM_CPS_MAX));
                })
                .state(&stream_cps)
                .width(WIDE_CONTROL_WIDTH),
            button("Preset")
                .action(|State(preset): State<Binding<i32>>| {
                    preset.set((preset.get() + 1).rem_euclid(3));
                })
                .state(&animation_preset)
                .width(SECONDARY_CONTROL_WIDTH),
            button("CPS -")
                .action(|State(cps): State<Binding<i32>>| {
                    cps.set((cps.get() - 8).clamp(8, 256));
                })
                .state(&animation_cps)
                .width(SECONDARY_CONTROL_WIDTH),
            button("CPS +")
                .action(|State(cps): State<Binding<i32>>| {
                    cps.set((cps.get() + 8).clamp(8, 256));
                })
                .state(&animation_cps)
                .width(SECONDARY_CONTROL_WIDTH),
            button(text!("{token_fade_label}"))
                .action(|State(enabled): State<Binding<bool>>| {
                    enabled.set(!enabled.get());
                })
                .state(&token_fade_enabled)
                .width(TOKEN_CONTROL_WIDTH),
        ))
        .spacing(8.0),
        Divider,
        scroll(flow_markdown(markdown).configuration(flow_config).padding()).border(Grey, 1.0),
    ))
    .padding()
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
