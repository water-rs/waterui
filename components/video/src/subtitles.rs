use std::{path::Path, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubtitleCue {
    pub(crate) start: Duration,
    pub(crate) end: Duration,
    pub(crate) text: String,
}

pub(crate) fn parse_subtitles_from_path(path: &Path) -> Result<Vec<SubtitleCue>, String> {
    let document = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read subtitle file {}: {error}", path.display()))?;
    parse_subtitle_document(path, &document)
}

pub(crate) fn active_subtitle_text(cues: &[SubtitleCue], position: Duration) -> Option<&str> {
    cues.iter()
        .rev()
        .find(|cue| cue.start <= position && position < cue.end)
        .map(|cue| cue.text.as_str())
}

fn parse_subtitle_document(path: &Path, document: &str) -> Result<Vec<SubtitleCue>, String> {
    let normalized = document
        .strip_prefix('\u{feff}')
        .unwrap_or(document)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);

    match extension.as_deref() {
        Some("vtt") => parse_webvtt(&normalized),
        Some("srt") => parse_srt(&normalized),
        _ if normalized
            .lines()
            .find(|line| !line.trim().is_empty())
            .is_some_and(|line| line.trim() == "WEBVTT") =>
        {
            parse_webvtt(&normalized)
        }
        _ if normalized.contains("-->") && normalized.contains(',') => parse_srt(&normalized),
        _ => Err(format!(
            "unsupported subtitle format for {}: expected .vtt or .srt",
            path.display()
        )),
    }
}

fn parse_webvtt(document: &str) -> Result<Vec<SubtitleCue>, String> {
    let mut cues = Vec::new();

    for block in document.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() || trimmed == "WEBVTT" || trimmed.starts_with("NOTE") {
            continue;
        }

        let mut lines = trimmed.lines();
        let Some(first_line) = lines.next() else {
            continue;
        };
        let timing_line = if first_line.contains("-->") {
            first_line
        } else {
            lines
                .next()
                .ok_or_else(|| format!("invalid WebVTT cue without timing line: {trimmed}"))?
        };
        let (start, end) = parse_timing_line(timing_line)?;
        let text = lines
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            continue;
        }
        cues.push(SubtitleCue { start, end, text });
    }

    Ok(cues)
}

fn parse_srt(document: &str) -> Result<Vec<SubtitleCue>, String> {
    let mut cues = Vec::new();

    for block in document.split("\n\n") {
        let trimmed = block.trim();
        if trimmed.is_empty() {
            continue;
        }

        let mut lines = trimmed.lines();
        let Some(first_line) = lines.next() else {
            continue;
        };
        let timing_line = if first_line.contains("-->") {
            first_line
        } else {
            lines
                .next()
                .ok_or_else(|| format!("invalid SRT cue without timing line: {trimmed}"))?
        };
        let (start, end) = parse_timing_line(timing_line)?;
        let text = lines
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            continue;
        }
        cues.push(SubtitleCue { start, end, text });
    }

    Ok(cues)
}

fn parse_timing_line(line: &str) -> Result<(Duration, Duration), String> {
    let (start, end_and_settings) = line
        .split_once("-->")
        .ok_or_else(|| format!("invalid subtitle timing line: {line}"))?;
    let start = parse_timestamp(start.trim())?;
    let end_token = end_and_settings
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("invalid subtitle timing line without end timestamp: {line}"))?;
    let end = parse_timestamp(end_token.trim())?;

    if end <= start {
        return Err(format!("subtitle cue end must be after start: {line}"));
    }

    Ok((start, end))
}

fn parse_timestamp(value: &str) -> Result<Duration, String> {
    let normalized = value.replace(',', ".");
    let (whole, fraction) = normalized
        .split_once('.')
        .ok_or_else(|| format!("subtitle timestamp must include milliseconds: {value}"))?;
    let fraction = match fraction.len() {
        1 => format!("{fraction}00"),
        2 => format!("{fraction}0"),
        _ => fraction.chars().take(3).collect::<String>(),
    };
    let milliseconds = fraction
        .parse::<u64>()
        .map_err(|error| format!("invalid subtitle milliseconds in {value}: {error}"))?;

    let parts = whole
        .split(':')
        .map(|part| {
            part.parse::<u64>().map_err(|error| {
                format!("invalid subtitle timestamp component in {value}: {error}")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => {
            return Err(format!("unsupported subtitle timestamp shape: {value}"));
        }
    };

    Ok(Duration::from_millis(
        (((hours * 60 + minutes) * 60 + seconds) * 1000) + milliseconds,
    ))
}

#[cfg(test)]
mod tests {
    use super::{active_subtitle_text, parse_subtitle_document};
    use std::{path::Path, time::Duration};

    #[test]
    fn parses_webvtt_cues() {
        let cues = parse_subtitle_document(
            Path::new("captions.vtt"),
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello\nWorld\n",
        )
        .expect("webvtt parse must succeed");

        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start, Duration::from_secs(1));
        assert_eq!(cues[0].end, Duration::from_millis(2500));
        assert_eq!(cues[0].text, "Hello\nWorld");
    }

    #[test]
    fn parses_srt_cues() {
        let cues = parse_subtitle_document(
            Path::new("captions.srt"),
            "1\n00:00:03,000 --> 00:00:05,000\nLine one\n\n2\n00:00:06,000 --> 00:00:07,000\nLine two\n",
        )
        .expect("srt parse must succeed");

        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].text, "Line one");
        assert_eq!(cues[1].start, Duration::from_secs(6));
    }

    #[test]
    fn finds_active_cue_for_playback_position() {
        let cues = parse_subtitle_document(
            Path::new("captions.vtt"),
            "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\nHello\n\n00:00:03.000 --> 00:00:04.000\nWorld\n",
        )
        .expect("subtitle parse must succeed");

        assert_eq!(
            active_subtitle_text(&cues, Duration::from_millis(1500)),
            Some("Hello")
        );
        assert_eq!(
            active_subtitle_text(&cues, Duration::from_millis(3500)),
            Some("World")
        );
        assert_eq!(active_subtitle_text(&cues, Duration::from_secs(5)), None);
    }
}
