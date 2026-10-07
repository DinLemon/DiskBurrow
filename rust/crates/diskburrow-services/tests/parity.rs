use chrono::{DateTime, Duration, Utc};
use diskburrow_services::*;
use std::{fs, path::PathBuf};
use uuid::Uuid;

fn fixture() -> tempfile::TempDir {
    let root = PathBuf::from(
        "E:/CodexWork/DiskBurrow-20261005-01a10bc8/work/rust-scratch/service-fixtures",
    );
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}
fn now() -> DateTime<Utc> {
    "2026-10-05T00:00:00.1234567Z".parse().unwrap()
}
fn snapshot(bytes: i64) -> ScanSnapshot {
    ScanSnapshot {
        id: Uuid::new_v4(),
        root: r"C:\".into(),
        started_utc: now(),
        completed_utc: now() + Duration::seconds(bytes),
        traversal_completed: true,
        directories: vec![DirectoryObservation {
            path: r"C:\cache".into(),
            logical_bytes: bytes,
            allocated_bytes: Some(bytes / 2),
            coverage_complete: true,
        }],
        largest_files: vec![],
        issues: vec![ScanIssue {
            path: r"C:\Windows".into(),
            kind: ScanIssueKind::AccessDenied,
        }],
    }
}

#[test]
fn legacy_omissions_keep_binary_defaults_but_new_settings_use_decimal() {
    let dir = fixture();
    let mut store = SettingsStore::new(dir.path());
    assert_eq!(store.load().low_space_bytes, 15_000_000_000);
    fs::write(dir.path().join("settings.json"), r#"{"Language":"en"}"#).unwrap();
    let settings = store.load();
    assert_eq!(settings.low_space_bytes, 15_i64 << 30);
    assert_eq!(settings.growth_bytes, 5_i64 << 30);
    store.save(&settings).unwrap();
    assert_eq!(store.load(), settings);
    let exact = i64::MAX - 3;
    fs::write(
        dir.path().join("settings.json"),
        format!("{{\"LowSpaceBytes\":{exact},\"GrowthBytes\":17}}"),
    )
    .unwrap();
    assert_eq!(store.load().low_space_bytes, exact);
    assert_eq!(store.load().growth_bytes, 17);
}
#[test]
fn corrupt_settings_are_visible_and_not_overwritten() {
    let dir = fixture();
    let mut store = SettingsStore::new(dir.path());
    for text in [
        "broken",
        r#"{"IntervalHours":2}"#,
        r#"{"ExcludedPaths":null}"#,
        r#"{"Language":"fr"}"#,
    ] {
        fs::write(dir.path().join("settings.json"), text).unwrap();
        assert_eq!(store.load(), AppSettings::default());
        assert!(store.last_user_message.is_some());
        assert_eq!(
            fs::read_to_string(dir.path().join("settings.json")).unwrap(),
            text
        );
    }
    store.save(&AppSettings::default()).unwrap();
    assert!(store.last_user_message.is_none());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn invalid_settings_never_create_a_document() {
    let dir = fixture();
    let mut store = SettingsStore::new(dir.path());
    for field in 0..5 {
        let mut s = AppSettings::default();
        match field {
            0 => s.growth_bytes = 0,
            1 => s.initial_delay = "00:00:00".into(),
            2 => s.excluded_paths.push("relative".into()),
            3 => s.theme = "blue".into(),
            _ => s.low_space_bytes = -1,
        }
        assert!(store.save(&s).is_err());
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn suppression_restart_never_replays_a_quiet_snapshot() {
    let dir = fixture();
    let mut store = SettingsStore::new(dir.path());
    let mut policy = AlertPolicy::new(
        AppSettings::default(),
        r"C:\",
        AlertSuppressionState::default(),
    )
    .unwrap();
    let a = snapshot(0);
    let b = snapshot(5_000_000_000);
    assert_eq!(
        policy
            .evaluate(Some(&a), Some(&b), 20_000_000_000, now())
            .unwrap()
            .len(),
        1
    );
    let c = snapshot(10_000_000_000);
    assert!(
        policy
            .evaluate(
                Some(&b),
                Some(&c),
                20_000_000_000,
                now() + Duration::hours(23)
            )
            .unwrap()
            .is_empty()
    );
    store.save_alert_state(&policy.state).unwrap();
    let mut restart =
        AlertPolicy::new(AppSettings::default(), r"C:\", store.load_alert_state()).unwrap();
    assert!(
        restart
            .evaluate(
                Some(&b),
                Some(&c),
                20_000_000_000,
                now() + Duration::days(1)
            )
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        restart
            .evaluate(
                Some(&c),
                Some(&snapshot(15_000_000_000)),
                20_000_000_000,
                now() + Duration::days(1)
            )
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn low_space_recovers_and_clock_backwards_does_not_repeat() {
    let mut policy = AlertPolicy::new(
        AppSettings::default(),
        r"C:\",
        AlertSuppressionState::default(),
    )
    .unwrap();
    assert_eq!(policy.evaluate(None, None, 1, now()).unwrap().len(), 1);
    assert!(
        policy
            .evaluate(None, None, 1, now() - Duration::days(1))
            .unwrap()
            .is_empty()
    );
    assert!(
        policy
            .evaluate(None, None, 15_000_000_000, now())
            .unwrap()
            .is_empty()
    );
    assert_eq!(policy.evaluate(None, None, 1, now()).unwrap().len(), 1);
    assert!(policy.evaluate(None, None, -1, now()).is_err());
}
#[test]
fn scan_schedule_coalesces_and_blocks_pause_battery_busy() {
    let mut s = SchedulePolicy::new(AppSettings::default(), now()).unwrap();
    assert!(!s.is_full_scan_due(now() + Duration::minutes(4), false, false));
    assert!(s.is_full_scan_due(now() + Duration::minutes(5), false, false));
    assert!(!s.is_full_scan_due(now() + Duration::hours(1), true, false));
    assert!(!s.is_full_scan_due(now() + Duration::hours(1), false, true));
    s.record_full_scan(now() + Duration::days(2));
    s.record_full_scan(now());
    assert!(!s.is_full_scan_due(now() + Duration::days(2) + Duration::hours(5), false, false));
    assert!(s.is_full_scan_due(now() + Duration::days(2) + Duration::hours(6), false, false));
    let settings = AppSettings {
        paused: true,
        ..Default::default()
    };
    s.apply_settings(settings).unwrap();
    assert!(!s.is_full_scan_due(now() + Duration::days(3), false, false));
}
#[test]
fn inaccessible_descendant_is_unavailable_not_deleted() {
    let mut a = snapshot(20);
    a.directories.push(DirectoryObservation {
        path: r"C:\cache\child".into(),
        logical_bytes: 20,
        allocated_bytes: None,
        coverage_complete: true,
    });
    let mut b = snapshot(0);
    b.directories[0].coverage_complete = false;
    let changes = compare_snapshots(&a, &b);
    assert!(
        changes
            .iter()
            .any(|c| c.path.eq_ignore_ascii_case(r"C:\cache\child")
                && c.kind == FolderChangeKind::Unavailable)
    );
    assert!(changes.iter().all(|c| c.kind != FolderChangeKind::Removed));
}
#[test]
fn history_retains_thirty_completed_scans_and_largest_hundred() {
    let dir = fixture();
    let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    for i in 0..35 {
        let mut s = snapshot(i);
        s.largest_files = (0..130)
            .map(|j| FileObservation {
                path: format!(r"C:\cache\{j}"),
                identity: Some(FileIdentity {
                    volume: u64::MAX,
                    file_id: j.to_string(),
                }),
                logical_bytes: j,
                allocated_bytes: None,
                modified_utc: now(),
                link_count: 1,
                attributes: 128,
            })
            .collect();
        assert!(store.save(&s).saved);
    }
    let loaded = store.load_recent("c:/", 100);
    assert_eq!(loaded.len(), 30);
    assert_eq!(loaded[0].directories[0].logical_bytes, 34);
    assert_eq!(loaded[0].largest_files.len(), 100);
    assert_eq!(loaded[0].largest_files[0].logical_bytes, 129);
    let mut incomplete = snapshot(100);
    incomplete.traversal_completed = false;
    assert!(!store.save(&incomplete).saved);
    assert_eq!(store.load_recent(r"C:\", 30).len(), 30);
}
#[test]
fn pruning_and_failed_insert_roll_back_old_rows() {
    let dir = fixture();
    let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    for i in 0..30 {
        assert!(store.save(&snapshot(i)).saved);
    }
    let before = store.load_recent(r"C:\", 30);
    let db = rusqlite::Connection::open(&store.database_path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_insert AFTER INSERT ON snapshots WHEN (SELECT COUNT(*) FROM snapshots)=30 BEGIN SELECT RAISE(ABORT,'fixture after pruning'); END;").unwrap();
    drop(db);
    assert!(!store.save(&snapshot(31)).saved);
    assert_eq!(store.load_recent(r"C:\", 30), before);
}
#[test]
fn corrupt_or_unknown_history_stays_byte_for_byte_intact() {
    let dir = fixture();
    let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    let path = store.database_path.clone();
    fs::write(&path, [1, 2, 3, 4, 5]).unwrap();
    assert!(store.load_recent(r"C:\", 30).is_empty());
    assert!(!store.save(&snapshot(1)).saved);
    assert_eq!(fs::read(&path).unwrap(), [1, 2, 3, 4, 5]);
    assert!(
        store
            .last_user_message
            .as_ref()
            .unwrap()
            .to_lowercase()
            .contains("reset")
    );
}
#[test]
fn all_sidecars_count_and_over_budget_writes_preserve_good_history() {
    let dir = fixture();
    let budget = StorageBudget::with_limits(dir.path(), 256 * 1024, 32 * 1024).unwrap();
    let mut store = SqliteHistoryStore::new(budget.clone());
    let good = snapshot(1);
    assert!(store.save(&good).saved);
    for (name, size) in [
        ("history.db-wal", 80 * 1024),
        ("history.db-shm", 80 * 1024),
        ("history.db-journal", 24 * 1024),
        ("sqlite-temp", 4 * 1024),
        ("old.log", 32 * 1024),
    ] {
        fs::write(dir.path().join(name), vec![0; size]).unwrap();
    }
    let before = budget.used_bytes().unwrap();
    assert!(!store.save(&snapshot(2)).saved);
    assert!(!budget.append_log("Cannot add"));
    assert_eq!(budget.used_bytes().unwrap(), before);
    for name in [
        "history.db-wal",
        "history.db-shm",
        "history.db-journal",
        "sqlite-temp",
        "old.log",
    ] {
        fs::remove_file(dir.path().join(name)).unwrap();
    }
    assert_eq!(store.load_recent(r"C:\", 30)[0].id, good.id);
}
#[test]
fn logs_rotate_in_place_under_shared_budget() {
    let dir = fixture();
    let budget = StorageBudget::with_limits(dir.path(), 64 * 1024, 1024).unwrap();
    for _ in 0..12 {
        assert!(budget.append_log(&"a".repeat(600)));
    }
    assert!(budget.append_log("latest"));
    assert!(budget.used_bytes().unwrap() <= 1024);
    assert!(
        fs::read_to_string(dir.path().join("diskburrow.log"))
            .unwrap()
            .contains("latest")
    );
    assert!(!budget.append_log(&"x".repeat(1024)));
}
#[test]
fn legacy_pascalcase_numeric_enums_and_utc_ticks_roundtrip_exactly() {
    let s = snapshot(1);
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["Issues"][0]["Kind"], 0);
    assert!(json.get("CompletedUtc").is_some());
    assert_eq!(serde_json::from_value::<ScanSnapshot>(json).unwrap(), s);
    assert_eq!(normalize_path(r"c:/CACHE/./child/.."), r"c:\CACHE");
    assert_eq!(path_key(r"C:\cache\"), path_key(r"c:/cache"));
}
#[test]
fn schema_one_migration_keeps_legacy_snapshot_and_tick_ordering() {
    let dir = fixture();
    let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    let s = snapshot(4);
    let db = rusqlite::Connection::open(&store.database_path).unwrap();
    db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;CREATE TABLE snapshots(id TEXT PRIMARY KEY,root TEXT NOT NULL,completed INTEGER NOT NULL,payload BLOB NOT NULL);PRAGMA user_version=1;").unwrap();
    db.execute(
        "INSERT INTO snapshots VALUES(?1,?2,?3,?4)",
        rusqlite::params![
            s.id.to_string(),
            r"C:\",
            dotnet_utc_ticks(s.completed_utc).unwrap(),
            serde_json::to_vec(&s).unwrap()
        ],
    )
    .unwrap();
    drop(db);
    assert_eq!(store.load_recent(r"C:\", 30), vec![s]);
    let db = rusqlite::Connection::open(&store.database_path).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(dotnet_utc_ticks(now()).unwrap() % 10_000_000, 1_234_567);
}
#[test]
fn incompatible_versions_and_bad_payloads_never_reset_history() {
    for bad in 0..6 {
        let dir = fixture();
        let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
        assert!(store.save(&snapshot(1)).saved);
        let db = rusqlite::Connection::open(&store.database_path).unwrap();
        match bad {
            0 => db.execute_batch("PRAGMA user_version=99").unwrap(),
            1 => db.execute_batch("PRAGMA user_version=0").unwrap(),
            _ => {
                let mut payload = serde_json::to_value(snapshot(2)).unwrap();
                match bad {
                    2 => payload = serde_json::Value::Null,
                    3 => payload["Directories"] = serde_json::Value::Null,
                    4 => payload["TraversalCompleted"] = false.into(),
                    _ => payload["Issues"][0]["Kind"] = 99.into(),
                };
                db.execute(
                    "UPDATE snapshots SET payload=?1",
                    [serde_json::to_vec(&payload).unwrap()],
                )
                .unwrap();
            }
        }
        drop(db);
        let before = fs::read(&store.database_path).unwrap();
        assert!(store.load_recent(r"C:\", 30).is_empty());
        assert!(
            store
                .last_user_message
                .as_ref()
                .unwrap()
                .to_lowercase()
                .contains("reset")
        );
        assert_eq!(fs::read(&store.database_path).unwrap(), before);
        if bad < 2 {
            assert!(!store.save(&snapshot(3)).saved);
            assert_eq!(fs::read(&store.database_path).unwrap(), before);
        }
    }
}
#[test]
fn oversized_payloads_repeatedly_leave_previous_live_and_persisted_results() {
    let dir = fixture();
    let budget = StorageBudget::with_limits(dir.path(), 256 * 1024, 16 * 1024).unwrap();
    let mut store = SqliteHistoryStore::new(budget.clone());
    let good = snapshot(1);
    assert!(store.save(&good).saved);
    let mut huge = snapshot(2);
    huge.directories = (0..4000)
        .map(|i| DirectoryObservation {
            path: format!(r"C:\{}\{i}", "x".repeat(200)),
            logical_bytes: 1,
            allocated_bytes: Some(1),
            coverage_complete: true,
        })
        .collect();
    let before = budget.used_bytes().unwrap();
    for _ in 0..5 {
        assert!(!store.save(&huge).saved);
    }
    assert_eq!(budget.used_bytes().unwrap(), before);
    assert_eq!(store.last_successful_snapshot.as_ref().unwrap().id, good.id);
    assert_eq!(store.load_recent(r"C:\", 30), vec![good]);
    assert_eq!(huge.directories.len(), 4000);
}
#[test]
fn cleanup_reports_keep_numeric_outcomes_audit_and_hundred_rows() {
    let dir = fixture();
    let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    let mut report = CleanupReport {
        plan_id: Uuid::new_v4(),
        items: vec![CleanupItemResult {
            candidate_id: Uuid::new_v4(),
            outcome: CleanupOutcome::SkippedChanged,
            reason_key: Some("changed".into()),
            audit: Some(CleanupAudit {
                path: r"C:\cache\a".into(),
                rule_id: "temp".into(),
                rule_root_path: r"C:\cache".into(),
                reviewed_modified_utc: now(),
                logical_bytes: i64::MAX,
                allocated_bytes: None,
                link_count: 2,
                identity: None,
            }),
        }],
        free_space_delta_bytes: -12,
        was_cancelled: true,
        free_space_delta_available: false,
    };
    for _ in 0..105 {
        report.plan_id = Uuid::new_v4();
        assert!(store.append_cleanup(&report).saved);
    }
    let db = rusqlite::Connection::open(&store.database_path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM cleanup_reports", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        100
    );
    let payload: Vec<u8> = db
        .query_row(
            "SELECT payload FROM cleanup_reports WHERE id=?1",
            [report.plan_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    assert_eq!(json["Items"][0]["Outcome"], 2);
    assert_eq!(
        serde_json::from_slice::<CleanupReport>(&payload).unwrap(),
        report
    );
}
#[test]
fn concurrent_stores_share_directory_budget_gate() {
    let dir = fixture();
    let threads: Vec<_> = (0..4)
        .map(|worker| {
            let root = dir.path().to_path_buf();
            std::thread::spawn(move || {
                let mut store = SqliteHistoryStore::new(StorageBudget::new(root));
                for i in 0..10 {
                    assert!(store.save(&snapshot(worker * 10 + i)).saved);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    assert_eq!(store.load_recent(r"C:\", 30).len(), 30);
}
#[test]
fn monotonic_volume_checks_keep_running_while_paused_busy_or_wall_clock_rewinds() {
    let settings = AppSettings {
        paused: true,
        ..Default::default()
    };
    let mut scheduler =
        MonitoringScheduler::new(settings, now(), std::time::Duration::ZERO, r"C:\").unwrap();
    assert_eq!(
        scheduler.tick(now(), std::time::Duration::from_secs(299), true, true, true),
        TickActions::default()
    );
    let tick = scheduler.tick(
        now() - Duration::days(2),
        std::time::Duration::from_secs(300),
        true,
        true,
        true,
    );
    assert!(tick.check_free_space);
    assert!(tick.request_full_scan.is_none());
    assert!(
        !scheduler
            .tick(
                now(),
                std::time::Duration::from_secs(301),
                false,
                false,
                true
            )
            .check_free_space
    );
    scheduler.apply_settings(AppSettings::default()).unwrap();
    let tick = scheduler.tick(
        now() + Duration::days(2),
        std::time::Duration::from_secs(3600),
        false,
        false,
        true,
    );
    assert!(tick.check_free_space);
    assert_eq!(tick.request_full_scan.as_deref(), Some(r"C:\"));
    let tick = scheduler.tick(
        now() + Duration::days(2),
        std::time::Duration::from_secs(3601),
        false,
        false,
        false,
    );
    assert!(!tick.check_free_space);
    assert!(tick.request_full_scan.is_none());
    scheduler.record_completed(&snapshot(2), now() + Duration::days(2));
    assert!(
        scheduler
            .tick(
                now() + Duration::days(2),
                std::time::Duration::from_secs(3602),
                false,
                false,
                true
            )
            .request_full_scan
            .is_none()
    );
}
#[test]
fn cancelled_history_write_never_opens_storage_or_changes_old_rows() {
    let dir = fixture();
    let mut store = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    let token = CancellationToken::new();
    token.cancel();
    assert!(
        store
            .save_cancellable(&snapshot(1), &token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    assert!(!store.database_path.exists());
    assert!(store.save(&snapshot(1)).saved);
    let before = fs::read(&store.database_path).unwrap();
    assert!(
        store
            .save_cancellable(&snapshot(2), &token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    assert_eq!(fs::read(&store.database_path).unwrap(), before);
}
#[test]
fn cancelled_settings_alerts_cleanup_and_reads_do_not_touch_fixture() {
    let dir = fixture();
    let token = CancellationToken::new();
    token.cancel();
    let mut settings = SettingsStore::new(dir.path());
    assert!(
        settings
            .load_cancellable(&token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    assert!(
        settings
            .save_cancellable(&AppSettings::default(), &token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    assert!(
        settings
            .load_alert_state_cancellable(&token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    assert!(
        settings
            .save_alert_state_cancellable(&AlertSuppressionState::default(), &token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    let mut history = SqliteHistoryStore::new(StorageBudget::new(dir.path()));
    assert!(
        history
            .load_recent_cancellable(r"C:\", 30, &token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    let report = CleanupReport {
        plan_id: Uuid::new_v4(),
        items: vec![],
        free_space_delta_bytes: 0,
        was_cancelled: false,
        free_space_delta_available: true,
    };
    assert!(
        history
            .append_cleanup_cancellable(&report, &token)
            .unwrap_err()
            .is::<Cancelled>()
    );
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}
#[test]
fn repeated_low_limit_writes_bound_actual_database_and_live_journal_peak() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };
    let dir = fixture();
    let budget = StorageBudget::with_limits(dir.path(), 384 * 1024, 16 * 1024).unwrap();
    let mut store = SqliteHistoryStore::new(budget.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let maximum = Arc::new(AtomicU64::new(0));
    let journal_samples = Arc::new(AtomicU64::new(0));
    let watcher = {
        let root = dir.path().to_path_buf();
        let stop = stop.clone();
        let maximum = maximum.clone();
        let samples = journal_samples.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                let total = fs::read_dir(&root)
                    .unwrap()
                    .flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum::<u64>();
                maximum.fetch_max(total, Ordering::Relaxed);
                if root.join("history.db-journal").exists() {
                    samples.fetch_add(1, Ordering::Relaxed);
                }
                std::thread::park_timeout(std::time::Duration::from_millis(1));
            }
        })
    };
    let mut saved = 0;
    for i in 0..40 {
        let mut s = snapshot(i);
        s.largest_files = (0..100)
            .map(|j| FileObservation {
                path: format!(r"C:\cache\{j}"),
                identity: Some(FileIdentity {
                    volume: 1,
                    file_id: j.to_string(),
                }),
                logical_bytes: j,
                allocated_bytes: Some(j),
                modified_utc: now(),
                link_count: 1,
                attributes: 128,
            })
            .collect();
        if store.save(&s).saved {
            saved += 1;
        }
        assert!(budget.used_bytes().unwrap() <= 384 * 1024);
    }
    stop.store(true, Ordering::Release);
    watcher.join().unwrap();
    assert!(saved > 0);
    assert!(maximum.load(Ordering::Relaxed) <= 384 * 1024);
    assert!(journal_samples.load(Ordering::Relaxed) > 0);
    assert!(!store.load_recent(r"C:\", 30).is_empty());
    eprintln!(
        "Saved={saved}/40 peak={} journal_samples={} limit=393216",
        maximum.load(Ordering::Relaxed),
        journal_samples.load(Ordering::Relaxed)
    );
}
