//! Regression test for #3667: update-check spawned task is aborted on fast commands.
//!
//! Bug: The runtime was dropped (via `process::exit` in `exit_ok`) before the
//! spawned update-check task could complete its network fetch and write the
//! cache. The fix keeps the `JoinHandle` and awaits it with a bounded timeout
//! (≤150 ms) so the task gets a grace period to finish.
//!
//! This test verifies the core pattern: a spawned task that writes to disk
//! completes successfully when its `JoinHandle` is awaited with a `tokio::time::timeout`.

use std::time::Duration;

/// Simulates the update-check pattern: spawn a task, then await it with a
/// bounded timeout so its side effect (cache write) actually occurs.
///
/// This is the exact pattern used in the fix for #3667:
///   1. `runtime.spawn(task)` returns a `JoinHandle`
///   2. `runtime.block_on(handle_command())` runs the command
///   3. `runtime.block_on(timeout(150ms, handle))` gives the task a grace period
#[test]
fn test_spawned_task_completes_with_grace_period() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build runtime");

    let cache_path = std::env::temp_dir().join(format!(
        "vaultpilot-regression-3667-{}-{}.json",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let _ = std::fs::remove_file(&cache_path);

    // Simulate the update-check task: write a cache file after a brief delay
    // (representing the network fetch).
    let path_clone = cache_path.clone();
    let handle: tokio::task::JoinHandle<()> = runtime.spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = std::fs::write(&path_clone, r#"{"checked_at":0,"latest_tag":"v9.9.9"}"#);
    });

    // Simulate a fast command (near-instant), then give the spawned task a
    // grace period — exactly what the fix does.
    let _result: () = runtime.block_on(async {});

    // #3667 fix: await the handle with a bounded timeout.
    let outcome =
        runtime.block_on(async { tokio::time::timeout(Duration::from_millis(150), handle).await });

    // The task should have completed within the grace period.
    assert!(
        outcome.is_ok(),
        "spawned task should complete within 150 ms grace period"
    );

    // The cache file should have been written.
    assert!(
        cache_path.exists(),
        "cache file should exist after grace period"
    );
    let content = std::fs::read_to_string(&cache_path).expect("cache file should be readable");
    assert!(
        content.contains("v9.9.9"),
        "cache file should contain the latest tag"
    );

    // Cleanup
    let _ = std::fs::remove_file(&cache_path);
}

/// Verifies that a spawned task with a longer delay is properly bounded by
/// the timeout (it should NOT complete, and the caller should get an `Err`
/// from `tokio::time::timeout`).
#[test]
fn test_grace_period_bounded_by_timeout() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to build runtime");

    let marker_path = std::env::temp_dir().join(format!(
        "vaultpilot-regression-3667-timeout-{}-{}.json",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let _ = std::fs::remove_file(&marker_path);

    let path_clone = marker_path.clone();
    // Spawn a task that takes 500ms — much longer than our 50ms grace period.
    let handle: tokio::task::JoinHandle<()> = runtime.spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let _ = std::fs::write(&path_clone, "late");
    });

    let _result: () = runtime.block_on(async {});

    // Grace period of 50ms — the task needs 500ms, so this should time out.
    let outcome =
        runtime.block_on(async { tokio::time::timeout(Duration::from_millis(50), handle).await });

    // The timeout should have fired (task not yet complete).
    assert!(
        outcome.is_err(),
        "timeout should fire when task takes longer than the grace period"
    );

    // The marker file should NOT have been written yet.
    assert!(
        !marker_path.exists(),
        "task output should NOT exist when timeout fires before completion"
    );

    // Cleanup
    let _ = std::fs::remove_file(&marker_path);
}
