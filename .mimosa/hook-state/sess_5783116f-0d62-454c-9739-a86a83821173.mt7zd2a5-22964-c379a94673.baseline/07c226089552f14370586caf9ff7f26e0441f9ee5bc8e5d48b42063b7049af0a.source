//! Fine-grained startup phase timing stats (issue #3851).
//!
//! Mirrors Obsidian 1.13.5's startup stats: a named phase timer records
//! when each startup phase (config load, storage open, agent init, ...)
//! completes and can render a human-readable report with per-phase
//! durations plus the total startup time.
//!
//! The timer is intentionally tiny — a single [`Instant`] plus one `Vec`
//! push per checkpoint — and can be fully disabled via
//! [`PhaseTimer::disabled`], in which case every checkpoint is a no-op.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

/// A single recorded startup phase.
///
/// `elapsed` is the **cumulative** time since the timer was created at the
/// moment the phase completed, so consecutive entries are non-negative and
/// monotonically non-decreasing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupPhase {
    /// Phase name, e.g. `"config_load"`.
    pub name: String,
    /// Cumulative elapsed time from the start of startup.
    pub elapsed: Duration,
}

/// Collected startup phase timings.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StartupStats {
    /// Phases in the order they were recorded.
    pub phases: Vec<StartupPhase>,
}

impl StartupStats {
    /// Total startup time — the elapsed time of the last recorded phase,
    /// or [`Duration::ZERO`] when no phase was recorded.
    pub fn total(&self) -> Duration {
        self.phases
            .last()
            .map(|phase| phase.elapsed)
            .unwrap_or(Duration::ZERO)
    }

    /// Build a human-readable report: one line per phase with its own
    /// duration in milliseconds, followed by a `total` line.
    pub fn report(&self) -> String {
        let mut out = String::new();
        let mut previous = Duration::ZERO;
        for phase in &self.phases {
            let own = phase.elapsed.saturating_sub(previous);
            let _ = writeln!(out, "{:<28} {:>12.3} ms", phase.name, ms(&own));
            previous = phase.elapsed;
        }
        let _ = writeln!(out, "{:<28} {:>12.3} ms", "total", ms(&self.total()));
        out
    }
}

/// Order-preserving startup phase timer.
///
/// Cheap when disabled: [`PhaseTimer::disabled`] makes every `checkpoint`
/// a no-op, so callers can leave the timer wired in unconditionally.
#[derive(Debug)]
pub struct PhaseTimer {
    enabled: bool,
    start: Instant,
    stats: StartupStats,
}

impl PhaseTimer {
    /// Create an enabled timer that records checkpoints.
    pub fn new() -> Self {
        Self {
            enabled: true,
            start: Instant::now(),
            stats: StartupStats::default(),
        }
    }

    /// Create a disabled timer; all checkpoints are no-ops.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            start: Instant::now(),
            stats: StartupStats::default(),
        }
    }

    /// Record the end of a named startup phase.
    pub fn checkpoint(&mut self, name: impl Into<String>) {
        if !self.enabled {
            return;
        }
        self.stats.phases.push(StartupPhase {
            name: name.into(),
            elapsed: self.start.elapsed(),
        });
    }

    /// Borrow the collected stats.
    pub fn stats(&self) -> &StartupStats {
        &self.stats
    }

    /// Consume the timer and return the collected stats.
    pub fn finish(self) -> StartupStats {
        self.stats
    }

    /// Build the formatted report (an empty phase list yields a `total` line only).
    pub fn report(&self) -> String {
        self.stats.report()
    }
}

impl Default for PhaseTimer {
    fn default() -> Self {
        Self::new()
    }
}

/// Record a named startup phase on the given timer.
///
/// Convenience wrapper used by the CLI startup path so checkpoints read
/// as a single call.
pub fn record_startup_phase(timer: &mut PhaseTimer, name: impl Into<String>) {
    timer.checkpoint(name);
}

/// Convert a duration to fractional milliseconds.
fn ms(duration: &Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_are_recorded_in_order() {
        let mut timer = PhaseTimer::new();
        timer.checkpoint("config_load");
        timer.checkpoint("storage_open");
        timer.checkpoint("agent_init");
        let stats = timer.finish();
        let names: Vec<&str> = stats.phases.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["config_load", "storage_open", "agent_init"]);
    }

    #[test]
    fn elapsed_values_are_non_negative_and_monotonic() {
        let mut timer = PhaseTimer::new();
        timer.checkpoint("first");
        std::thread::sleep(Duration::from_millis(2));
        timer.checkpoint("second");
        std::thread::sleep(Duration::from_millis(2));
        timer.checkpoint("third");
        let stats = timer.finish();
        let mut previous = Duration::ZERO;
        for phase in &stats.phases {
            assert!(
                phase.elapsed >= Duration::ZERO,
                "elapsed must be non-negative"
            );
            assert!(
                phase.elapsed >= previous,
                "elapsed must be monotonic: {} < {}",
                ms(&phase.elapsed),
                ms(&previous)
            );
            previous = phase.elapsed;
        }
        assert!(stats.total() >= previous);
    }

    #[test]
    fn report_contains_all_phase_names_and_total() {
        let mut timer = PhaseTimer::new();
        timer.checkpoint("config_load");
        timer.checkpoint("storage_open");
        timer.checkpoint("agent_init");
        let report = timer.report();
        for needle in ["config_load", "storage_open", "agent_init", "total"] {
            assert!(
                report.contains(needle),
                "report missing {needle}:\n{report}"
            );
        }
    }

    #[test]
    fn empty_timer_reports_zero_total() {
        let timer = PhaseTimer::new();
        let stats = timer.finish();
        assert!(stats.phases.is_empty());
        assert_eq!(stats.total(), Duration::ZERO);
        let report = stats.report();
        assert!(report.contains("total"));
    }

    #[test]
    fn disabled_timer_ignores_checkpoints() {
        let mut timer = PhaseTimer::disabled();
        timer.checkpoint("config_load");
        timer.checkpoint("storage_open");
        assert!(timer.finish().phases.is_empty());
    }
}
