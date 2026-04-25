use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, File},
    hash::{Hash, Hasher},
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use futures::StreamExt;
use tracing::info;
use tracing_subscriber::EnvFilter;
use waterkit_codec::{CodecType, Decoder};
use waterkit_video::VideoReader;
use zenwave::{Client, Method, redirect::FollowRedirect};

const ANALYSIS_TARGET: &str = "waterui.video.analysis";
const REALTIME_DROP_THRESHOLD: Duration = Duration::from_millis(120);
const PRESENT_TOLERANCE: Duration = Duration::from_millis(3);

#[derive(Clone, Copy, Debug)]
struct SampleCase {
    name: &'static str,
    url: &'static str,
    expected_codec: CodecType,
    min_decoded_frames: usize,
    decode_frame_budget: usize,
    max_first_frame_ms: u64,
    min_decode_fps: f64,
}

#[derive(Debug)]
struct CaseReport {
    name: &'static str,
    codec: CodecType,
    width: u32,
    height: u32,
    sample_count: u32,
    decoded_frames: usize,
    pts_regressions: usize,
    first_frame_ms: f64,
    decode_fps: f64,
    mean_frame_delta_ms: f64,
    p95_frame_delta_ms: f64,
    download_ms: f64,
    open_ms: f64,
    decode_ms: f64,
    seek_recovery_mean_ms: f64,
    seek_recovery_p95_ms: f64,
    seek_recovery_max_ms: f64,
}

#[derive(Debug, Clone, Copy)]
struct TimelineFrame {
    pts: Duration,
}

#[derive(Debug)]
struct VodSyncReport {
    startup_ms: f64,
    buffering_events: usize,
    buffering_total_ms: f64,
    mean_abs_drift_ms: f64,
    p95_abs_drift_ms: f64,
    max_abs_drift_ms: f64,
}

#[derive(Debug)]
struct LiveSyncReport {
    startup_ms: f64,
    rendered_frames: usize,
    dropped_frames: usize,
    late_frames: usize,
    mean_abs_drift_ms: f64,
    p95_abs_drift_ms: f64,
    max_abs_drift_ms: f64,
}

#[derive(Debug)]
struct SeekRecoveryReport {
    mean_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

fn init_test_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,waterui_video=info,waterkit_video=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_test_writer()
        .try_init();
}

fn pts_to_duration(pts: u64, timescale: u32) -> Duration {
    if timescale == 0 {
        return Duration::ZERO;
    }
    Duration::from_nanos((pts.saturating_mul(1_000_000_000)) / u64::from(timescale))
}

fn detect_codec(config: &[u8]) -> Result<CodecType, String> {
    if config.len() >= 8 && &config[4..8] == b"avcC" {
        return Ok(CodecType::H264);
    }

    if config.len() >= 7 && config[0] == 0x01 {
        let profile = config[1];
        if matches!(profile, 66 | 77 | 88 | 100 | 110 | 122 | 144) {
            return Ok(CodecType::H264);
        }
    }

    if config.len() >= 8 && &config[4..8] == b"hvcC" {
        return Ok(CodecType::H265);
    }

    if config.len() >= 23 && config[0] == 0x01 {
        let level = config[12];
        if level > 0 && level <= 186 {
            return Ok(CodecType::H265);
        }
    }

    Err(String::from(
        "Unknown codec config in real sample analysis (expected avcC/hvcC)",
    ))
}

fn cache_path_for(url: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    let hash = hasher.finish();
    std::env::temp_dir()
        .join("waterui_real_sample_analysis")
        .join(format!("{hash:016x}.mp4"))
}

fn percentile(values: &[f64], ratio: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * ratio.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn signed_delta_ms(lhs: Duration, rhs: Duration) -> f64 {
    if lhs >= rhs {
        lhs.checked_sub(rhs).unwrap().as_secs_f64() * 1000.0
    } else {
        -(rhs.checked_sub(lhs).unwrap().as_secs_f64() * 1000.0)
    }
}

fn shift_duration_ms(base: Duration, delta_ms: i64) -> Duration {
    if delta_ms >= 0 {
        return base.saturating_add(Duration::from_millis(delta_ms as u64));
    }

    let magnitude = Duration::from_millis(delta_ms.unsigned_abs());
    base.checked_sub(magnitude)
        .unwrap_or_else(|| panic!("network timeline underflow: base={base:?}, delta_ms={delta_ms}"))
}

const fn deterministic_jitter_ms(index: usize, amplitude_ms: u64) -> i64 {
    let cycle = (index as u64 * 37 + 11) % 13;
    ((cycle * amplitude_ms) / 12) as i64
}

fn build_arrival_timeline(
    timeline: &[TimelineFrame],
    throughput_ratio: f64,
    base_latency_ms: u64,
    jitter_amplitude_ms: u64,
    spike_every: usize,
    spike_ms: u64,
) -> Vec<Duration> {
    if timeline.is_empty() {
        return Vec::new();
    }
    assert!(
        (throughput_ratio.is_finite() && throughput_ratio > 0.0),
        "throughput_ratio must be finite and positive"
    );

    let mut arrivals = Vec::with_capacity(timeline.len());
    let mut network_clock = Duration::from_millis(base_latency_ms);
    arrivals.push(network_clock);

    for index in 1..timeline.len() {
        let prev = timeline[index - 1].pts;
        let current = timeline[index].pts;
        assert!(
            (current >= prev),
            "timeline PTS must be monotonic: frame={index}, prev={prev:?}, current={current:?}"
        );

        let delta_pts = current.checked_sub(prev).unwrap();
        let transfer_time = delta_pts.mul_f64(1.0 / throughput_ratio);
        network_clock = network_clock.saturating_add(transfer_time);

        let jitter = deterministic_jitter_ms(index, jitter_amplitude_ms);
        network_clock = shift_duration_ms(network_clock, jitter);

        if spike_every > 0 && index % spike_every == 0 {
            network_clock = network_clock.saturating_add(Duration::from_millis(spike_ms));
        }

        assert!(
            (network_clock >= arrivals[index - 1]),
            "network arrival timeline regressed at frame={index}: previous={:?}, current={:?}",
            arrivals[index - 1],
            network_clock
        );

        arrivals.push(network_clock);
    }

    arrivals
}

fn start_wall_with_buffer(
    timeline: &[TimelineFrame],
    arrivals: &[Duration],
    startup_buffer: Duration,
) -> Duration {
    assert!(
        (timeline.len() == arrivals.len()),
        "timeline/arrival length mismatch: timeline={}, arrivals={}",
        timeline.len(),
        arrivals.len()
    );
    if timeline.is_empty() {
        return Duration::ZERO;
    }

    let base_pts = timeline[0].pts;
    for index in 0..timeline.len() {
        let buffered = timeline[index].pts.saturating_sub(base_pts);
        if buffered >= startup_buffer {
            return arrivals[index];
        }
    }
    arrivals[0]
}

fn analyze_vod_sync(
    timeline: &[TimelineFrame],
    arrivals: &[Duration],
    startup_buffer: Duration,
) -> VodSyncReport {
    let start_wall = start_wall_with_buffer(timeline, arrivals, startup_buffer);
    let base_pts = timeline[0].pts;
    let mut anchor_wall = start_wall;

    let mut buffering_events = 0usize;
    let mut buffering_total = Duration::ZERO;
    let mut late_streak = false;
    let mut abs_drift_ms = Vec::with_capacity(timeline.len());

    for (frame, arrival) in timeline.iter().zip(arrivals.iter().copied()) {
        let delta = frame.pts.saturating_sub(base_pts);
        let expected = anchor_wall.saturating_add(delta);

        if arrival > expected.saturating_add(PRESENT_TOLERANCE) {
            let stall = arrival.checked_sub(expected).unwrap();
            buffering_total = buffering_total.saturating_add(stall);
            anchor_wall = anchor_wall.saturating_add(stall);
            if !late_streak {
                buffering_events = buffering_events.saturating_add(1);
                late_streak = true;
            }
        } else {
            late_streak = false;
        }

        let expected_after = anchor_wall.saturating_add(delta);
        let present_wall = if arrival > expected_after {
            arrival
        } else {
            expected_after
        };
        let playback_clock = base_pts.saturating_add(present_wall.saturating_sub(anchor_wall));
        abs_drift_ms.push(signed_delta_ms(frame.pts, playback_clock).abs());
    }

    VodSyncReport {
        startup_ms: start_wall.as_secs_f64() * 1000.0,
        buffering_events,
        buffering_total_ms: buffering_total.as_secs_f64() * 1000.0,
        mean_abs_drift_ms: mean(&abs_drift_ms),
        p95_abs_drift_ms: percentile(&abs_drift_ms, 0.95),
        max_abs_drift_ms: percentile(&abs_drift_ms, 1.0),
    }
}

fn analyze_live_sync(
    timeline: &[TimelineFrame],
    arrivals: &[Duration],
    startup_buffer: Duration,
) -> LiveSyncReport {
    let start_wall = start_wall_with_buffer(timeline, arrivals, startup_buffer);
    let base_pts = timeline[0].pts;

    let mut rendered_frames = 0usize;
    let mut dropped_frames = 0usize;
    let mut late_frames = 0usize;
    let mut abs_drift_ms = Vec::with_capacity(timeline.len());

    for (frame, arrival) in timeline.iter().zip(arrivals.iter().copied()) {
        let delta = frame.pts.saturating_sub(base_pts);
        let expected = start_wall.saturating_add(delta);

        if arrival > expected.saturating_add(REALTIME_DROP_THRESHOLD) {
            dropped_frames = dropped_frames.saturating_add(1);
            continue;
        }

        let present_wall = if arrival > expected {
            arrival
        } else {
            expected
        };
        let audio_clock = base_pts.saturating_add(present_wall.saturating_sub(start_wall));
        let drift_ms = signed_delta_ms(frame.pts, audio_clock);

        if drift_ms < 0.0 {
            late_frames = late_frames.saturating_add(1);
        }

        rendered_frames = rendered_frames.saturating_add(1);
        abs_drift_ms.push(drift_ms.abs());
    }

    LiveSyncReport {
        startup_ms: start_wall.as_secs_f64() * 1000.0,
        rendered_frames,
        dropped_frames,
        late_frames,
        mean_abs_drift_ms: mean(&abs_drift_ms),
        p95_abs_drift_ms: percentile(&abs_drift_ms, 0.95),
        max_abs_drift_ms: percentile(&abs_drift_ms, 1.0),
    }
}

fn download_sample(url: &str) -> Result<(PathBuf, f64), String> {
    let path = cache_path_for(url);
    if path.exists() {
        return Ok((path, 0.0));
    }

    let Some(parent) = path.parent() else {
        return Err(String::from("Invalid cache path"));
    };
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;

    let started_at = Instant::now();
    futures::executor::block_on(async {
        let mut client = FollowRedirect::new(zenwave::client());
        let response = client
            .method(Method::GET, url)
            .await
            .map_err(|error| error.to_string())?;

        if !response.status().is_success() {
            return Err(format!(
                "HTTP error while downloading sample: {}",
                response.status()
            ));
        }

        let mut body = response.into_body();
        let mut file = File::create(&path).map_err(|error| error.to_string())?;
        let mut bytes_written = 0usize;

        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|error| error.to_string())?;
            file.write_all(&chunk).map_err(|error| error.to_string())?;
            bytes_written = bytes_written.saturating_add(chunk.len());
        }

        file.flush().map_err(|error| error.to_string())?;
        info!(target: ANALYSIS_TARGET, bytes_written, "downloaded real sample");
        Ok::<(), String>(())
    })?;

    Ok((path, started_at.elapsed().as_secs_f64() * 1000.0))
}

fn collect_timeline(path: &Path, frame_budget: usize) -> Result<Vec<TimelineFrame>, String> {
    let mut reader = VideoReader::open(path).map_err(|error| error.to_string())?;
    let timescale = reader.timescale();
    let mut timeline = Vec::with_capacity(frame_budget);
    let mut last_pts = None::<Duration>;

    while timeline.len() < frame_budget {
        let Some((_sample, sample_pts, _keyframe)) =
            reader.read_sample().map_err(|error| error.to_string())?
        else {
            break;
        };

        let pts = pts_to_duration(sample_pts, timescale);
        if let Some(previous) = last_pts
            && pts < previous
        {
            return Err(format!(
                "PTS regression while collecting timeline: prev={previous:?} current={pts:?}"
            ));
        }

        timeline.push(TimelineFrame { pts });
        last_pts = Some(pts);
    }

    if timeline.len() < frame_budget {
        return Err(format!(
            "timeline too short: got {} frames, need {}",
            timeline.len(),
            frame_budget
        ));
    }

    Ok(timeline)
}

fn measure_seek_recovery(
    path: &Path,
    codec: CodecType,
    width: u32,
    height: u32,
) -> Result<SeekRecoveryReport, String> {
    let reader = VideoReader::open(path).map_err(|error| error.to_string())?;
    let sample_count = reader.sample_count();
    if sample_count < 16 {
        return Err(format!(
            "not enough samples for seek recovery analysis: {sample_count}"
        ));
    }

    let timescale = reader.timescale();
    let config = reader
        .codec_config()
        .ok_or_else(|| String::from("No codec config in real sample"))?;

    let seek_points = [0.12_f64, 0.33_f64, 0.57_f64, 0.81_f64];
    let mut latencies_ms = Vec::with_capacity(seek_points.len());

    for progress in seek_points {
        let target_index = (progress * f64::from(sample_count.saturating_sub(1))).round() as usize;
        let mut scan_reader = VideoReader::open(path).map_err(|error| error.to_string())?;
        let mut keyframe_index = 0usize;
        let mut target_pts = Duration::ZERO;
        let mut found_target = false;

        for index in 0..=target_index {
            let Some((_sample_data, sample_pts, is_keyframe)) = scan_reader
                .read_sample()
                .map_err(|error| error.to_string())?
            else {
                break;
            };
            if is_keyframe {
                keyframe_index = index;
            }
            if index == target_index {
                target_pts = pts_to_duration(sample_pts, timescale);
                found_target = true;
                break;
            }
        }

        if !found_target {
            return Err(format!(
                "target sample missing during seek recovery: progress={progress:.3} target_index={target_index}"
            ));
        }

        let mut decoder = Decoder::new(codec, Some(config), width, height)
            .map_err(|error| format!("decoder init failed during seek analysis: {error}"))?;
        let mut decode_reader = VideoReader::open(path).map_err(|error| error.to_string())?;

        let started = Instant::now();
        let mut recovered = false;
        for sample_index in 0..sample_count as usize {
            let Some((sample_data, sample_pts, _is_keyframe)) = decode_reader
                .read_sample()
                .map_err(|error| error.to_string())?
            else {
                break;
            };
            if sample_index < keyframe_index {
                continue;
            }

            let sample_pts_duration = pts_to_duration(sample_pts, timescale);
            let mut stream = decoder.decode(&sample_data);
            for frame_result in stream {
                let frame = frame_result.map_err(|error| error.to_string())?;
                let frame_pts = if frame.timestamp_ns() > 0 {
                    Duration::from_nanos(frame.timestamp_ns())
                } else {
                    sample_pts_duration
                };

                if frame_pts.saturating_add(Duration::from_millis(5)) >= target_pts {
                    recovered = true;
                    break;
                }
            }

            if recovered {
                break;
            }
        }

        if !recovered {
            return Err(format!(
                "seek recovery failed at progress={progress:.3} target_index={target_index}"
            ));
        }

        latencies_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }

    Ok(SeekRecoveryReport {
        mean_ms: mean(&latencies_ms),
        p95_ms: percentile(&latencies_ms, 0.95),
        max_ms: percentile(&latencies_ms, 1.0),
    })
}

fn analyze_case(case: SampleCase) -> Result<CaseReport, String> {
    let _span = tracing::info_span!(
        "real_sample_analysis_case",
        name = case.name,
        expected_codec = ?case.expected_codec,
        frame_budget = case.decode_frame_budget
    )
    .entered();

    let (sample_path, download_ms) = download_sample(case.url)?;
    info!(
        target: ANALYSIS_TARGET,
        path = %sample_path.display(),
        download_ms = download_ms,
        "sample ready"
    );

    let open_started = Instant::now();
    let mut reader = VideoReader::open(&sample_path).map_err(|error| error.to_string())?;
    let open_ms = open_started.elapsed().as_secs_f64() * 1000.0;

    let (width, height) = reader.dimensions();
    let sample_count = reader.sample_count();
    let timescale = reader.timescale();

    let config = reader
        .codec_config()
        .ok_or_else(|| String::from("No codec config in real sample"))?;
    let codec = detect_codec(config)?;
    if codec != case.expected_codec {
        return Err(format!(
            "Codec mismatch for {}: expected {:?}, got {:?}",
            case.name, case.expected_codec, codec
        ));
    }

    let mut decoder = Decoder::new(codec, Some(config), width, height)
        .map_err(|error| format!("decoder init failed: {error}"))?;

    let mut nv12_buffer = vec![0_u8; (width * height * 3 / 2) as usize];
    let decode_started = Instant::now();
    let mut first_frame_latency = None::<Duration>;
    let mut decoded_frames = 0usize;
    let mut pts_regressions = 0usize;
    let mut prev_pts = None::<Duration>;
    let mut frame_delta_ms = Vec::<f64>::new();

    while decoded_frames < case.decode_frame_budget {
        let Some((sample_data, sample_pts, _is_keyframe)) =
            reader.read_sample().map_err(|error| error.to_string())?
        else {
            break;
        };

        let sample_pts_duration = pts_to_duration(sample_pts, timescale);
        let mut decode_stream = decoder.decode(&sample_data);
        for frame_result in decode_stream {
            let frame = frame_result.map_err(|error| error.to_string())?;

            if first_frame_latency.is_none() {
                first_frame_latency = Some(decode_started.elapsed());
            }

            if frame.width() != width || frame.height() != height {
                return Err(format!(
                    "Frame dimensions mismatch for {}: got {}x{}, expected {}x{}",
                    case.name,
                    frame.width(),
                    frame.height(),
                    width,
                    height
                ));
            }

            let written = frame.copy_to_buffer(&mut nv12_buffer);
            if written != nv12_buffer.len() {
                return Err(format!(
                    "NV12 write size mismatch for {}: got {}, expected {}",
                    case.name,
                    written,
                    nv12_buffer.len()
                ));
            }

            let frame_pts = if frame.timestamp_ns() > 0 {
                Duration::from_nanos(frame.timestamp_ns())
            } else {
                sample_pts_duration
            };

            if let Some(previous_pts) = prev_pts {
                if frame_pts < previous_pts {
                    pts_regressions = pts_regressions.saturating_add(1);
                } else {
                    frame_delta_ms
                        .push(frame_pts.checked_sub(previous_pts).unwrap().as_secs_f64() * 1000.0);
                }
            }
            prev_pts = Some(frame_pts);

            decoded_frames = decoded_frames.saturating_add(1);
            if decoded_frames >= case.decode_frame_budget {
                break;
            }
        }
    }

    let decode_elapsed = decode_started.elapsed();
    if decoded_frames < case.min_decoded_frames {
        return Err(format!(
            "Too few decoded frames for {}: got {}, expected at least {}",
            case.name, decoded_frames, case.min_decoded_frames
        ));
    }

    let first_frame = first_frame_latency
        .ok_or_else(|| format!("No decoded frame produced for {}", case.name))?
        .as_secs_f64()
        * 1000.0;
    let decode_fps = decoded_frames as f64 / decode_elapsed.as_secs_f64();
    let mean_frame_delta_ms = mean(&frame_delta_ms);
    let p95_frame_delta_ms = percentile(&frame_delta_ms, 0.95);

    let seek_recovery = measure_seek_recovery(&sample_path, codec, width, height)?;

    Ok(CaseReport {
        name: case.name,
        codec,
        width,
        height,
        sample_count,
        decoded_frames,
        pts_regressions,
        first_frame_ms: first_frame,
        decode_fps,
        mean_frame_delta_ms,
        p95_frame_delta_ms,
        download_ms,
        open_ms,
        decode_ms: decode_elapsed.as_secs_f64() * 1000.0,
        seek_recovery_mean_ms: seek_recovery.mean_ms,
        seek_recovery_p95_ms: seek_recovery.p95_ms,
        seek_recovery_max_ms: seek_recovery.max_ms,
    })
}

#[test]
#[ignore = "network + real sample performance analysis"]
fn trace_real_samples_performance_and_correctness() {
    init_test_tracing();

    let cases = [
        SampleCase {
            name: "sdr_avc_1080p_3m",
            url: "https://repo.jellyfin.org/test-videos/SDR/AVC/Test%20Jellyfin%201080p%20AVC%203M.mp4",
            expected_codec: CodecType::H264,
            min_decoded_frames: 150,
            decode_frame_budget: 180,
            max_first_frame_ms: 2000,
            min_decode_fps: 12.0,
        },
        SampleCase {
            name: "sdr_hevc_1080p_3m",
            url: "https://repo.jellyfin.org/test-videos/SDR/HEVC%208bit/Test%20Jellyfin%201080p%20HEVC%208bit%203M.mp4",
            expected_codec: CodecType::H265,
            min_decoded_frames: 150,
            decode_frame_budget: 180,
            max_first_frame_ms: 2500,
            min_decode_fps: 10.0,
        },
        SampleCase {
            name: "hdr10_hevc_1080p_3m",
            url: "https://repo.jellyfin.org/test-videos/HDR/HDR10/HEVC/Test%20Jellyfin%201080p%20HEVC%20HDR10%203M.mp4",
            expected_codec: CodecType::H265,
            min_decoded_frames: 120,
            decode_frame_budget: 150,
            max_first_frame_ms: 3000,
            min_decode_fps: 8.0,
        },
    ];

    for case in cases {
        let report = analyze_case(case).unwrap_or_else(|error| {
            panic!("real sample analysis failed for {}: {error}", case.name);
        });

        info!(
            target: ANALYSIS_TARGET,
            sample = report.name,
            codec = ?report.codec,
            width = report.width,
            height = report.height,
            sample_count = report.sample_count,
            decoded_frames = report.decoded_frames,
            pts_regressions = report.pts_regressions,
            first_frame_ms = report.first_frame_ms,
            decode_fps = report.decode_fps,
            mean_frame_delta_ms = report.mean_frame_delta_ms,
            p95_frame_delta_ms = report.p95_frame_delta_ms,
            download_ms = report.download_ms,
            open_ms = report.open_ms,
            decode_ms = report.decode_ms,
            seek_recovery_mean_ms = report.seek_recovery_mean_ms,
            seek_recovery_p95_ms = report.seek_recovery_p95_ms,
            seek_recovery_max_ms = report.seek_recovery_max_ms,
            "real sample analysis report"
        );

        assert_eq!(
            report.pts_regressions, 0,
            "PTS regression detected in {}",
            case.name
        );
        assert!(
            report.first_frame_ms <= case.max_first_frame_ms as f64,
            "first frame too slow in {}: {:.2}ms > {}ms",
            case.name,
            report.first_frame_ms,
            case.max_first_frame_ms
        );
        assert!(
            report.decode_fps >= case.min_decode_fps,
            "decode fps too low in {}: {:.2} < {:.2}",
            case.name,
            report.decode_fps,
            case.min_decode_fps
        );
        assert!(
            report.seek_recovery_p95_ms <= 150.0,
            "seek recovery too slow in {}: p95 {:.2}ms > 150ms",
            case.name,
            report.seek_recovery_p95_ms
        );
    }
}

#[test]
#[ignore = "network + real sample timeline sync analysis"]
fn trace_real_sample_network_sync_behavior() {
    init_test_tracing();

    let sample = SampleCase {
        name: "sdr_avc_1080p_3m",
        url: "https://repo.jellyfin.org/test-videos/SDR/AVC/Test%20Jellyfin%201080p%20AVC%203M.mp4",
        expected_codec: CodecType::H264,
        min_decoded_frames: 0,
        decode_frame_budget: 0,
        max_first_frame_ms: 0,
        min_decode_fps: 0.0,
    };

    let (sample_path, download_ms) =
        download_sample(sample.url).expect("sample download should succeed for sync analysis");
    info!(
        target: ANALYSIS_TARGET,
        sample = sample.name,
        path = %sample_path.display(),
        download_ms,
        "network sync sample ready"
    );

    let timeline = collect_timeline(&sample_path, 360)
        .expect("timeline collection must produce enough frames");

    let vod_arrivals = build_arrival_timeline(&timeline, 1.20, 80, 5, 120, 550);
    let vod = analyze_vod_sync(&timeline, &vod_arrivals, Duration::from_millis(150));

    info!(
        target: ANALYSIS_TARGET,
        sample = sample.name,
        mode = "vod",
        startup_ms = vod.startup_ms,
        buffering_events = vod.buffering_events,
        buffering_total_ms = vod.buffering_total_ms,
        mean_abs_drift_ms = vod.mean_abs_drift_ms,
        p95_abs_drift_ms = vod.p95_abs_drift_ms,
        max_abs_drift_ms = vod.max_abs_drift_ms,
        "network sync report"
    );

    assert!(
        vod.buffering_events >= 1,
        "vod scenario should observe buffering events under injected stall"
    );
    assert!(
        vod.max_abs_drift_ms <= 35.0,
        "vod drift too high: {:.2}ms",
        vod.max_abs_drift_ms
    );

    let live_arrivals = build_arrival_timeline(&timeline, 1.0, 45, 18, 90, 240);
    let live = analyze_live_sync(&timeline, &live_arrivals, Duration::from_millis(120));

    info!(
        target: ANALYSIS_TARGET,
        sample = sample.name,
        mode = "live",
        startup_ms = live.startup_ms,
        rendered_frames = live.rendered_frames,
        dropped_frames = live.dropped_frames,
        late_frames = live.late_frames,
        mean_abs_drift_ms = live.mean_abs_drift_ms,
        p95_abs_drift_ms = live.p95_abs_drift_ms,
        max_abs_drift_ms = live.max_abs_drift_ms,
        "network sync report"
    );

    assert!(
        live.dropped_frames > 0,
        "live scenario must drop frames when injected latency exceeds threshold"
    );
    assert!(
        live.max_abs_drift_ms <= 120.0,
        "live drift too high: {:.2}ms",
        live.max_abs_drift_ms
    );
}
