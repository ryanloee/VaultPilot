//! Regression test for #3910 — agent startup phase timing (`startupStats`).
//!
//! The `vaultpilot-agent` sidecar records fine-grained startup phases via
//! `vaultpilot_lib::startup_stats::PhaseTimer` and serves them to the WinUI
//! client through the `startupStats` JSON-RPC method in this shape:
//!
//! ```json
//! {
//!   "phases": [
//!     { "name": "search_rules_load", "elapsed_ms": 3.21 },
//!     { "name": "ipc_ready",         "elapsed_ms": 564.68 }
//!   ],
//!   "total_ms": 567.89
//! }
//! ```
//!
//! `elapsed_ms` is each phase's **own** fractional-millisecond duration —
//! the increment over the previous phase's cumulative elapsed
//! (`elapsed.saturating_sub(previous)`, matching `StartupStats::report()`) —
//! not the cumulative elapsed time, and `total_ms` equals the last phase's
//! cumulative elapsed (`StartupStats::total()`), i.e. the sum of the phase
//! increments. Consecutive `elapsed_ms` values are therefore *not*
//! monotonic; only `total_ms` accumulates.
//!
//! The serialization helper lives in `src/bin/vaultpilot-agent.rs` and is
//! private to that binary crate, so this test mirrors its shape logic —
//! same convention as `issue_3103_agent_health_detection.rs`.

#[cfg(test)]
mod tests {
    use crate::startup_stats::{PhaseTimer, StartupStats};
    use std::time::Duration;

    /// Mirror of the agent binary's `startup_stats_to_json` shape (#3910).
    fn startup_stats_to_json(stats: &StartupStats) -> serde_json::Value {
        let mut previous = Duration::ZERO;
        let phases: Vec<serde_json::Value> = stats
            .phases
            .iter()
            .map(|phase| {
                let own = phase.elapsed.saturating_sub(previous);
                previous = phase.elapsed;
                serde_json::json!({
                    "name": phase.name,
                    "elapsed_ms": own.as_secs_f64() * 1000.0,
                })
            })
            .collect();
        serde_json::json!({
            "phases": phases,
            "total_ms": stats.total().as_secs_f64() * 1000.0,
        })
    }

    #[test]
    fn phase_timer_stats_serialize_to_startup_stats_shape() {
        let mut timer = PhaseTimer::new();
        timer.checkpoint("search_rules_load");
        timer.checkpoint("runtime_build");
        timer.checkpoint("storage_open");
        timer.checkpoint("ipc_ready");
        let stats = timer.finish();
        assert_eq!(stats.phases.len(), 4, "all checkpoints must be recorded");

        let json = startup_stats_to_json(&stats);
        assert!(json.is_object(), "result must be a JSON object");

        let phases = json["phases"].as_array().expect("phases must be an array");
        assert_eq!(phases.len(), 4, "all recorded phases must be serialized");

        // Names in recording order.
        let names: Vec<&str> = phases
            .iter()
            .map(|p| p["name"].as_str().expect("phase name must be a string"))
            .collect();
        assert_eq!(
            names,
            [
                "search_rules_load",
                "runtime_build",
                "storage_open",
                "ipc_ready"
            ],
            "phases must be serialized in recording order"
        );

        // Each elapsed_ms is the phase's OWN duration (increment over the
        // previous phase): a finite f64 >= 0, but NOT cumulative/monotonic.
        let mut sum_ms = 0.0f64;
        for phase in phases.iter() {
            let elapsed_ms = phase["elapsed_ms"]
                .as_f64()
                .expect("elapsed_ms must be an f64");
            assert!(elapsed_ms.is_finite(), "elapsed_ms must be finite");
            assert!(elapsed_ms >= 0.0, "elapsed_ms must be non-negative");
            sum_ms += elapsed_ms;
        }

        let total_ms = json["total_ms"].as_f64().expect("total_ms must be an f64");
        assert!(total_ms.is_finite(), "total_ms must be finite");
        assert!(total_ms >= 0.0);
        assert!(
            (sum_ms - total_ms).abs() <= 0.01,
            "sum of per-phase elapsed_ms ({sum_ms:.3}) must equal total_ms ({total_ms:.3})"
        );
        assert_eq!(
            total_ms,
            stats.total().as_secs_f64() * 1000.0,
            "total_ms must equal StartupStats::total() in ms"
        );
        assert!(total_ms > 0.0, "real checkpoints must take some time");
    }

    #[test]
    fn empty_timer_serializes_to_empty_shape() {
        let stats = PhaseTimer::new().finish();
        let json = startup_stats_to_json(&stats);
        assert_eq!(
            json["phases"]
                .as_array()
                .expect("phases must be an array")
                .len(),
            0,
            "empty stats must yield an empty phases array"
        );
        assert_eq!(json["total_ms"], 0.0, "empty stats must yield total_ms 0");
    }

    #[test]
    fn elapsed_values_are_fractional_millisecond_numbers() {
        // elapsed_ms must serialize as a JSON number (not a string) with
        // fractional-millisecond precision matching the Duration exactly.
        let mut timer = PhaseTimer::new();
        timer.checkpoint("instant_phase");
        let stats = timer.finish();
        let json = startup_stats_to_json(&stats);
        let phase = &json["phases"][0];
        assert!(
            phase["elapsed_ms"].is_number(),
            "elapsed_ms must be a JSON number"
        );
        let elapsed_ms = phase["elapsed_ms"]
            .as_f64()
            .expect("elapsed_ms must be an f64");
        assert!(elapsed_ms.is_finite(), "elapsed_ms must be finite");
        assert!(elapsed_ms >= 0.0);
        assert_eq!(
            elapsed_ms,
            stats.phases[0].elapsed.as_secs_f64() * 1000.0,
            "elapsed_ms must be the exact fractional-millisecond duration"
        );
    }
}
