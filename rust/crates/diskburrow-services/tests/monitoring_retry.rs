#[test]
fn requested_scan_cannot_repeat_before_admission() {
    use chrono::{DateTime, Duration as WallDuration};
    use diskburrow_services::{AppSettings, MonitoringScheduler};
    use std::time::Duration;
    let now = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
    let mut scheduler =
        MonitoringScheduler::new(AppSettings::default(), now, Duration::ZERO, r"C:\").unwrap();
    assert!(
        scheduler
            .tick(
                now + WallDuration::minutes(5),
                Duration::from_secs(300),
                false,
                false,
                true
            )
            .request_full_scan
            .is_some()
    );
    assert!(
        scheduler
            .tick(
                now + WallDuration::minutes(5),
                Duration::from_secs(301),
                false,
                false,
                true
            )
            .request_full_scan
            .is_none()
    );
}
use chrono::{DateTime, Duration as WallDuration, Utc};
use diskburrow_services::{AppSettings, AutomaticScanOutcome, MonitoringScheduler};
use std::time::Duration;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}
fn scheduler() -> MonitoringScheduler {
    MonitoringScheduler::new(AppSettings::default(), now(), Duration::ZERO, r"C:\").unwrap()
}
fn due(scheduler: &mut MonitoringScheduler, seconds: u64) -> Option<u64> {
    scheduler
        .tick(
            now() + WallDuration::days(2),
            Duration::from_secs(seconds),
            false,
            false,
            true,
        )
        .automatic_request_id
}

#[test]
fn outstanding_automatic_request_is_reserved_even_before_worker_admission() {
    let mut scheduler = scheduler();
    let id = due(&mut scheduler, 300).unwrap();
    assert!(due(&mut scheduler, 301).is_none());
    assert!(due(&mut scheduler, 86_400).is_none());
    assert!(scheduler.finish_automatic(
        id,
        r"C:\",
        AutomaticScanOutcome::Failed,
        Duration::from_secs(86_400)
    ));
}

#[test]
fn failure_retries_at_fifteen_monotonic_minutes_despite_wall_clock_changes() {
    let mut scheduler = scheduler();
    let id = due(&mut scheduler, 300).unwrap();
    assert!(scheduler.finish_automatic(
        id,
        r"C:\",
        AutomaticScanOutcome::Failed,
        Duration::from_secs(310)
    ));
    let fast_clock = scheduler.tick(
        now() + WallDuration::days(900),
        Duration::from_secs(1209),
        false,
        false,
        true,
    );
    assert!(fast_clock.request_full_scan.is_none());
    assert!(fast_clock.check_free_space);
    let rewound_clock = scheduler.tick(
        now() - WallDuration::days(900),
        Duration::from_secs(1210),
        false,
        false,
        true,
    );
    // Clock rewind may postpone the regular schedule, but never shortens the retry bound.
    assert!(rewound_clock.request_full_scan.is_none());
    assert!(due(&mut scheduler, 1210).is_some());
}

#[test]
fn explicit_cancellation_defers_automatic_scan_by_the_configured_interval() {
    let mut scheduler = scheduler();
    scheduler
        .apply_settings(AppSettings {
            interval_hours: 1,
            ..Default::default()
        })
        .unwrap();
    let id = due(&mut scheduler, 300).unwrap();
    assert!(scheduler.finish_automatic(
        id,
        r"c:\",
        AutomaticScanOutcome::Cancelled,
        Duration::from_secs(310)
    ));
    assert!(due(&mut scheduler, 3909).is_none());
    assert!(due(&mut scheduler, 3910).is_some());
}

#[test]
fn stale_attempts_and_other_roots_cannot_release_a_new_request() {
    let mut scheduler = scheduler();
    let first = due(&mut scheduler, 300).unwrap();
    assert!(!scheduler.finish_automatic(
        first,
        r"E:\",
        AutomaticScanOutcome::Failed,
        Duration::from_secs(310)
    ));
    assert!(due(&mut scheduler, 311).is_none());
    assert!(scheduler.finish_automatic(
        first,
        r"C:\",
        AutomaticScanOutcome::Failed,
        Duration::from_secs(310)
    ));
    let second = due(&mut scheduler, 1210).unwrap();
    assert_ne!(first, second);
    assert!(!scheduler.finish_automatic(
        first,
        r"C:\",
        AutomaticScanOutcome::Cancelled,
        Duration::from_secs(1211)
    ));
    assert!(due(&mut scheduler, 1212).is_none());
    assert!(scheduler.finish_automatic(
        second,
        r"C:\",
        AutomaticScanOutcome::Completed,
        Duration::from_secs(1212)
    ));
    assert!(due(&mut scheduler, 1213).is_some());
}
#[test]
fn manual_system_attempts_delay_auto_but_other_roots_and_successful_history_are_preserved() {
    let mut scheduler = scheduler();
    assert!(!scheduler.record_manual_outcome(
        r"E:\",
        AutomaticScanOutcome::Cancelled,
        Duration::from_secs(300)
    ));
    assert!(scheduler.record_manual_outcome(
        r"C:\",
        AutomaticScanOutcome::Failed,
        Duration::from_secs(300)
    ));
    assert!(due(&mut scheduler, 1199).is_none());
    assert!(scheduler.record_manual_outcome(
        r"C:\",
        AutomaticScanOutcome::Cancelled,
        Duration::from_secs(400)
    ));
    assert!(due(&mut scheduler, 21_999).is_none());
    // A successful explicit scan replaces retry delay with the normal last-success schedule.
    assert!(scheduler.record_manual_outcome(
        r"C:\",
        AutomaticScanOutcome::Completed,
        Duration::from_secs(500)
    ));
    let snapshot = diskburrow_services::ScanSnapshot {
        id: uuid::Uuid::new_v4(),
        root: r"C:\".into(),
        started_utc: now(),
        completed_utc: now(),
        traversal_completed: true,
        directories: vec![],
        largest_files: vec![],
        issues: vec![],
    };
    scheduler.record_completed(&snapshot, now());
    assert!(
        scheduler
            .tick(
                now() + WallDuration::hours(6) - WallDuration::seconds(1),
                Duration::from_secs(30_000),
                false,
                false,
                true
            )
            .request_full_scan
            .is_none()
    );
    let id = scheduler
        .tick(
            now() + WallDuration::hours(6),
            Duration::from_secs(30_001),
            false,
            false,
            true,
        )
        .automatic_request_id
        .unwrap();
    assert!(!scheduler.record_manual_outcome(
        r"C:\",
        AutomaticScanOutcome::Failed,
        Duration::from_secs(30_002)
    ));
    assert!(due(&mut scheduler, 40_000).is_none());
    assert!(scheduler.finish_automatic(
        id,
        r"C:\",
        AutomaticScanOutcome::Completed,
        Duration::from_secs(40_000)
    ));
}
