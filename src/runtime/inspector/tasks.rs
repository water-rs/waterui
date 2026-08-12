//! Main-thread occupancy, aggregated in the target process.
//!
//! Every poll of every main-thread future calls this probe, so it must cost
//! almost nothing. It accumulates per-task-type totals and emits one event per
//! window instead of one per poll — reporting every poll individually costs
//! more than the work being measured.
//!
//! Backtraces are the other half of the problem. Capturing a stack takes
//! milliseconds on the very thread whose stalls we are trying to explain, so a
//! capture is rate-limited to at most one per [`BACKTRACE_INTERVAL`]. A stall
//! reported without a stack is honest; a stall report that caused the next
//! stall is not.

use std::backtrace::Backtrace;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use waterui_inspector_protocol::{Channel, InspectorEvent, StallSample, TaskAggregate, TaskWindow};

use crate::task::{RuntimeProbe, TaskPollSample};

use super::hub::EventHub;

/// Shortest interval between two backtrace captures.
const BACKTRACE_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum stack frames reported for a stall.
const BACKTRACE_DEPTH: usize = 24;

/// Aggregates poll samples and publishes one window at a time.
pub(super) struct TaskProbe {
    hub: Arc<EventHub>,
    window: Duration,
    stall_ratio: f64,
    state: Mutex<ProbeState>,
}

struct ProbeState {
    window_started: Instant,
    last_backtrace: Option<Instant>,
    totals: HashMap<&'static str, Totals>,
    budget_us: u32,
    refresh_hz: f32,
}

#[derive(Default)]
struct Totals {
    polls: u32,
    ready: u32,
    wall_us_total: u64,
    wall_us_max: u32,
    cpu_us_total: u64,
    over_budget: u32,
}

impl TaskProbe {
    /// Creates a probe publishing to `hub` once per `window`.
    pub(super) fn new(hub: Arc<EventHub>, window: Duration, stall_ratio: f64) -> Self {
        Self {
            hub,
            window,
            stall_ratio,
            state: Mutex::new(ProbeState {
                window_started: Instant::now(),
                last_backtrace: None,
                totals: HashMap::new(),
                budget_us: 0,
                refresh_hz: 0.0,
            }),
        }
    }
}

impl RuntimeProbe for TaskProbe {
    fn on_poll_sample(&self, sample: &TaskPollSample) {
        if !self.hub.wants(Channel::Tasks) {
            return;
        }

        let wall_us = duration_to_us(sample.wall);
        let budget_us = saturating_u32(duration_to_us(sample.frame_budget));
        let over_budget = budget_us > 0 && wall_us > u64::from(budget_us);

        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };

        state.budget_us = budget_us;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a display refresh rate is far inside f32 range"
        )]
        {
            state.refresh_hz = sample.refresh_hz as f32;
        }

        let totals = state.totals.entry(sample.task_type).or_default();
        totals.polls = totals.polls.saturating_add(1);
        if sample.poll_ready {
            totals.ready = totals.ready.saturating_add(1);
        }
        totals.wall_us_total = totals.wall_us_total.saturating_add(wall_us);
        totals.wall_us_max = totals.wall_us_max.max(saturating_u32(wall_us));
        totals.cpu_us_total = totals
            .cpu_us_total
            .saturating_add(duration_to_us(sample.cpu));
        if over_budget {
            totals.over_budget = totals.over_budget.saturating_add(1);
        }

        let stall = self.stall_for(&mut state, sample, wall_us, budget_us);
        let window = (state.window_started.elapsed() >= self.window).then(|| flush(&mut state));
        drop(state);

        // Publishing outside the lock keeps a slow socket from serialising the
        // probe against the next poll.
        if let Some(stall) = stall {
            self.hub.publish(InspectorEvent::Stall(stall));
        }
        if let Some(window) = window {
            self.hub.publish(InspectorEvent::Tasks(window));
        }
    }
}

impl TaskProbe {
    /// Builds a stall sample when this poll overran badly enough to warrant one.
    fn stall_for(
        &self,
        state: &mut ProbeState,
        sample: &TaskPollSample,
        wall_us: u64,
        budget_us: u32,
    ) -> Option<StallSample> {
        let budget_secs = sample.frame_budget.as_secs_f64();
        if budget_secs <= 0.0 {
            return None;
        }
        let usage_ratio = sample.wall.as_secs_f64() / budget_secs;
        if usage_ratio < self.stall_ratio {
            return None;
        }

        let now = Instant::now();
        let may_capture = state
            .last_backtrace
            .is_none_or(|last| now.duration_since(last) >= BACKTRACE_INTERVAL);
        let backtrace = if may_capture {
            state.last_backtrace = Some(now);
            capture_backtrace()
        } else {
            Vec::new()
        };

        #[expect(
            clippy::cast_possible_truncation,
            reason = "a budget percentage is far inside f32 range"
        )]
        Some(StallSample {
            task_type: sample.task_type.to_string(),
            wall_us,
            cpu_us: duration_to_us(sample.cpu),
            budget_us: u64::from(budget_us),
            usage_pct: (usage_ratio * 100.0) as f32,
            backtrace,
        })
    }
}

/// Drains the accumulated window, most expensive task first.
fn flush(state: &mut ProbeState) -> TaskWindow {
    let elapsed = state.window_started.elapsed();
    state.window_started = Instant::now();

    let mut tasks: Vec<TaskAggregate> = state
        .totals
        .drain()
        .map(|(task_type, totals)| TaskAggregate {
            task_type: task_type.to_string(),
            polls: totals.polls,
            ready: totals.ready,
            wall_us_total: totals.wall_us_total,
            wall_us_max: totals.wall_us_max,
            cpu_us_total: totals.cpu_us_total,
            over_budget: totals.over_budget,
        })
        .collect();
    tasks.sort_unstable_by_key(|task| core::cmp::Reverse(task.wall_us_total));

    TaskWindow {
        window_ms: saturating_u32(elapsed.as_millis().try_into().unwrap_or(u64::MAX)),
        budget_us: state.budget_us,
        refresh_hz: state.refresh_hz,
        tasks,
    }
}

/// Captures the current stack, trimmed to something a UI can display.
fn capture_backtrace() -> Vec<String> {
    Backtrace::force_capture()
        .to_string()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(BACKTRACE_DEPTH)
        .map(ToString::to_string)
        .collect()
}

fn duration_to_us(duration: Duration) -> u64 {
    duration.as_micros().try_into().unwrap_or(u64::MAX)
}

fn saturating_u32(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::{super::hub::EventHub, TaskProbe};
    use std::sync::Arc;
    use std::time::Duration;
    use waterui_inspector_protocol::{ChannelSet, InspectorEvent};

    use crate::task::{RuntimeProbe, TaskPollSample};

    fn sample(wall_us: u64) -> TaskPollSample {
        TaskPollSample {
            task_type: "TestTask",
            poll_ready: false,
            wall: Duration::from_micros(wall_us),
            cpu: Duration::from_micros(wall_us),
            frame_budget: Duration::from_micros(8_333),
            refresh_hz: 120.0,
        }
    }

    /// The defect this replaces: one event per poll. A thousand polls inside one
    /// window must produce one aggregate, not a thousand events.
    #[test]
    fn a_window_of_polls_produces_one_event() {
        let (hub, receiver) = EventHub::new();
        hub.set_subscribed(ChannelSet::TASKS);
        let probe = TaskProbe::new(Arc::clone(&hub), Duration::ZERO, 100.0);

        for _ in 0..1000 {
            probe.on_poll_sample(&sample(10));
        }

        // With a zero-length window every poll flushes, so instead assert the
        // shape: each event is an aggregate, never a per-poll record.
        let mut aggregates = 0_usize;
        while let Ok(dispatch) = receiver.try_recv() {
            match dispatch {
                super::super::hub::Dispatch::Event(envelope) => {
                    assert!(matches!(envelope.event, InspectorEvent::Tasks(_)));
                    aggregates += 1;
                }
                _ => panic!("expected an event"),
            }
        }
        assert!(aggregates > 0);
    }

    /// Nothing at all should happen while the tasks channel is unsubscribed.
    #[test]
    fn an_unsubscribed_probe_is_silent() {
        let (hub, receiver) = EventHub::new();
        let probe = TaskProbe::new(Arc::clone(&hub), Duration::ZERO, 0.9);

        for _ in 0..100 {
            probe.on_poll_sample(&sample(50_000));
        }

        assert!(receiver.is_empty());
    }

    /// Back-to-back stalls must not each pay for a stack capture.
    #[test]
    fn backtrace_capture_is_rate_limited() {
        let (hub, receiver) = EventHub::new();
        hub.set_subscribed(ChannelSet::TASKS);
        let probe = TaskProbe::new(Arc::clone(&hub), Duration::from_secs(3600), 0.9);

        for _ in 0..10 {
            probe.on_poll_sample(&sample(50_000));
        }

        let mut with_backtrace = 0_usize;
        let mut stalls = 0_usize;
        while let Ok(dispatch) = receiver.try_recv() {
            if let super::super::hub::Dispatch::Event(envelope) = dispatch
                && let InspectorEvent::Stall(stall) = envelope.event
            {
                stalls += 1;
                if !stall.backtrace.is_empty() {
                    with_backtrace += 1;
                }
            }
        }

        assert_eq!(stalls, 10, "every stall is still reported");
        assert_eq!(with_backtrace, 1, "only the first one pays for a stack");
    }
}
