use crate::*;
use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderChangeKind {
    New,
    Removed,
    Changed,
    Unavailable,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderChange {
    pub path: String,
    pub logical_delta_bytes: i64,
    pub allocated_delta_bytes: Option<i64>,
    pub kind: FolderChangeKind,
    pub comparable: bool,
}
pub fn compare_snapshots(previous: &ScanSnapshot, current: &ScanSnapshot) -> Vec<FolderChange> {
    if path_key(&previous.root) != path_key(&current.root) {
        return vec![];
    }
    let before: BTreeMap<_, _> = previous
        .directories
        .iter()
        .map(|d| (path_key(&d.path), d))
        .collect();
    let after: BTreeMap<_, _> = current
        .directories
        .iter()
        .map(|d| (path_key(&d.path), d))
        .collect();
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).cloned().collect();
    let mut changes = vec![];
    for key in keys {
        let old = before.get(&key).copied();
        let now = after.get(&key).copied();
        let path = normalize_path(&old.or(now).expect("Union key has a directory").path);
        let comparable = previous.traversal_completed
            && current.traversal_completed
            && old.map_or_else(|| covered_parent(&key, &before), |d| d.coverage_complete)
            && now.map_or_else(|| covered_parent(&key, &after), |d| d.coverage_complete);
        if !comparable {
            changes.push(FolderChange {
                path,
                logical_delta_bytes: 0,
                allocated_delta_bytes: None,
                kind: FolderChangeKind::Unavailable,
                comparable: false,
            });
            continue;
        }
        let delta = now
            .map_or(0, |d| d.logical_bytes)
            .saturating_sub(old.map_or(0, |d| d.logical_bytes));
        let allocated = if old.is_none_or(|d| d.allocated_bytes.is_some())
            && now.is_none_or(|d| d.allocated_bytes.is_some())
        {
            Some(
                now.and_then(|d| d.allocated_bytes)
                    .unwrap_or(0)
                    .saturating_sub(old.and_then(|d| d.allocated_bytes).unwrap_or(0)),
            )
        } else {
            None
        };
        if old.is_some() && now.is_some() && delta == 0 && allocated.is_none_or(|n| n == 0) {
            continue;
        }
        changes.push(FolderChange {
            path,
            logical_delta_bytes: delta,
            allocated_delta_bytes: allocated,
            kind: if old.is_none() {
                FolderChangeKind::New
            } else if now.is_none() {
                FolderChangeKind::Removed
            } else {
                FolderChangeKind::Changed
            },
            comparable: true,
        });
    }
    changes
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickActions {
    pub check_free_space: bool,
    pub request_full_scan: Option<String>,
    pub automatic_request_id: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomaticScanOutcome {
    Completed,
    Failed,
    Cancelled,
}
/// The host uses a monotonic clock for ticks; a wall-clock adjustment cannot spin checks.
pub struct MonitoringScheduler {
    schedule: SchedulePolicy,
    system_root: String,
    next_space_check: std::time::Duration,
    automatic_generation: u64,
    pending_automatic: Option<u64>,
    automatic_not_before: std::time::Duration,
}
impl MonitoringScheduler {
    pub fn new(
        settings: AppSettings,
        started_utc: DateTime<Utc>,
        started_monotonic: std::time::Duration,
        system_root: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            schedule: SchedulePolicy::new(settings, started_utc)?,
            system_root: normalize_path(&system_root.into()),
            next_space_check: started_monotonic + std::time::Duration::from_secs(300),
            automatic_generation: 0,
            pending_automatic: None,
            automatic_not_before: started_monotonic,
        })
    }
    pub fn tick(
        &mut self,
        now_utc: DateTime<Utc>,
        monotonic: std::time::Duration,
        on_battery: bool,
        operation_running: bool,
        local_root_confirmed: bool,
    ) -> TickActions {
        if !local_root_confirmed {
            return TickActions::default();
        }
        let check_free_space = monotonic >= self.next_space_check;
        if check_free_space {
            self.next_space_check = monotonic.saturating_add(std::time::Duration::from_secs(300));
        }
        let automatic_request_id = if self.pending_automatic.is_none()
            && monotonic >= self.automatic_not_before
            && self
                .schedule
                .is_full_scan_due(now_utc, on_battery, operation_running)
        {
            self.automatic_generation = self
                .automatic_generation
                .checked_add(1)
                .expect("Automatic scan generation exhausted");
            self.pending_automatic = Some(self.automatic_generation);
            self.pending_automatic
        } else {
            None
        };
        TickActions {
            check_free_space,
            request_full_scan: automatic_request_id.map(|_| self.system_root.clone()),
            automatic_request_id,
        }
    }
    /// Only the reserved request for the system volume can release the automatic slot.
    /// Attempt outcomes do not create or modify completed scan history.
    pub fn finish_automatic(
        &mut self,
        request_id: u64,
        root: &str,
        outcome: AutomaticScanOutcome,
        monotonic: std::time::Duration,
    ) -> bool {
        if self.pending_automatic != Some(request_id)
            || path_key(root) != path_key(&self.system_root)
        {
            return false;
        }
        self.pending_automatic = None;
        self.record_attempt_outcome(outcome, monotonic);
        true
    }
    /// Explicit system-volume scans affect the next automatic attempt without
    /// taking its reservation. Other roots and outstanding requests are untouched.
    pub fn record_manual_outcome(
        &mut self,
        root: &str,
        outcome: AutomaticScanOutcome,
        monotonic: std::time::Duration,
    ) -> bool {
        if self.pending_automatic.is_some() || path_key(root) != path_key(&self.system_root) {
            return false;
        }
        self.record_attempt_outcome(outcome, monotonic);
        true
    }
    fn record_attempt_outcome(
        &mut self,
        outcome: AutomaticScanOutcome,
        monotonic: std::time::Duration,
    ) {
        let delay = match outcome {
            AutomaticScanOutcome::Completed => std::time::Duration::ZERO,
            AutomaticScanOutcome::Failed => std::time::Duration::from_secs(15 * 60),
            AutomaticScanOutcome::Cancelled => {
                std::time::Duration::from_secs(self.schedule.settings.interval_hours as u64 * 3600)
            }
        };
        self.automatic_not_before = if outcome == AutomaticScanOutcome::Completed {
            monotonic
        } else {
            self.automatic_not_before
                .max(monotonic.saturating_add(delay))
        };
    }
    pub fn apply_settings(&mut self, settings: AppSettings) -> Result<()> {
        self.schedule.apply_settings(settings)
    }
    pub fn record_completed(&mut self, snapshot: &ScanSnapshot, now_utc: DateTime<Utc>) {
        if snapshot.traversal_completed && path_key(&snapshot.root) == path_key(&self.system_root) {
            self.schedule.record_full_scan(now_utc);
        }
    }
}
fn covered_parent(path: &str, directories: &BTreeMap<String, &DirectoryObservation>) -> bool {
    let mut path = path.to_owned();
    loop {
        let Some(separator) = path.rfind('\\') else {
            return false;
        };
        path.truncate(if separator == 2 && path.as_bytes().get(1) == Some(&b':') {
            3
        } else {
            separator
        });
        if let Some(directory) = directories.get(&path) {
            return directory.coverage_complete;
        }
        if path.len() <= 3 {
            return false;
        }
    }
}
pub struct SchedulePolicy {
    settings: AppSettings,
    started_utc: DateTime<Utc>,
    last_full_scan: Option<DateTime<Utc>>,
}
impl SchedulePolicy {
    pub fn new(settings: AppSettings, started_utc: DateTime<Utc>) -> Result<Self> {
        settings.validate()?;
        Ok(Self {
            settings,
            started_utc,
            last_full_scan: None,
        })
    }
    pub fn is_full_scan_due(
        &self,
        now: DateTime<Utc>,
        on_battery: bool,
        operation_running: bool,
    ) -> bool {
        !self.settings.paused
            && (!on_battery || self.settings.allow_on_battery)
            && !operation_running
            && now
                >= self
                    .last_full_scan
                    .map_or(self.started_utc + Duration::minutes(5), |last| {
                        last + Duration::hours(i64::from(self.settings.interval_hours))
                    })
    }
    pub fn record_full_scan(&mut self, now: DateTime<Utc>) {
        self.last_full_scan = Some(self.last_full_scan.map_or(now, |last| last.max(now)));
    }
    pub fn apply_settings(&mut self, settings: AppSettings) -> Result<()> {
        settings.validate()?;
        self.settings = settings;
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    LowSpace,
    FolderGrowth,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertEvent {
    pub kind: AlertKind,
    pub destination_path: String,
    pub bytes: i64,
    pub snapshot_id: Option<uuid::Uuid>,
    pub root: Option<String>,
    pub previous_snapshot_id: Option<uuid::Uuid>,
}
pub struct AlertPolicy {
    settings: AppSettings,
    system_root: String,
    pub state: AlertSuppressionState,
}
impl AlertPolicy {
    pub fn new(
        settings: AppSettings,
        root: impl Into<String>,
        mut state: AlertSuppressionState,
    ) -> Result<Self> {
        settings.validate()?;
        state.validate()?;
        state.low_space_roots = state
            .low_space_roots
            .into_iter()
            .map(|p| path_key(&p))
            .collect();
        state.last_growth_notified_utc = state
            .last_growth_notified_utc
            .into_iter()
            .map(|(p, t)| (path_key(&p), t))
            .collect();
        state.last_growth_snapshot = state
            .last_growth_snapshot
            .into_iter()
            .map(|(p, t)| (path_key(&p), t))
            .collect();
        state.evaluated_growth_snapshots = state
            .evaluated_growth_snapshots
            .into_iter()
            .map(|(p, t)| (path_key(&p), t))
            .collect();
        Ok(Self {
            settings,
            system_root: normalize_path(&root.into()),
            state,
        })
    }
    pub fn evaluate(
        &mut self,
        previous: Option<&ScanSnapshot>,
        current: Option<&ScanSnapshot>,
        free: i64,
        now: DateTime<Utc>,
    ) -> Result<Vec<AlertEvent>> {
        if free < 0 {
            bail!("Free space must be nonnegative.");
        }
        let mut alerts = vec![];
        let root = current.map_or_else(|| self.system_root.clone(), |s| normalize_path(&s.root));
        let root_key = path_key(&root);
        if free >= self.settings.low_space_bytes {
            self.state.low_space_roots.remove(&root_key);
        } else if self.state.low_space_roots.insert(root_key.clone()) {
            alerts.push(AlertEvent {
                kind: AlertKind::LowSpace,
                destination_path: root.clone(),
                bytes: free,
                snapshot_id: current.map(|s| s.id),
                root: Some(root.clone()),
                previous_snapshot_id: None,
            });
        }
        let (Some(previous), Some(current)) = (previous, current) else {
            return Ok(alerts);
        };
        if !previous.traversal_completed || !current.traversal_completed {
            return Ok(alerts);
        }
        let evaluated = self
            .state
            .evaluated_growth_snapshots
            .entry(root_key)
            .or_default();
        if evaluated.contains(&current.id) {
            return Ok(alerts);
        }
        evaluated.push(current.id);
        if evaluated.len() > 30 {
            evaluated.remove(0);
        }
        for change in compare_snapshots(previous, current) {
            let key = path_key(&change.path);
            if !change.comparable
                || change.logical_delta_bytes < self.settings.growth_bytes
                || self.state.last_growth_snapshot.get(&key) == Some(&current.id)
                || self
                    .state
                    .last_growth_notified_utc
                    .get(&key)
                    .is_some_and(|last| now - *last < Duration::days(1))
            {
                continue;
            }
            self.state.last_growth_notified_utc.insert(key.clone(), now);
            self.state.last_growth_snapshot.insert(key, current.id);
            alerts.push(AlertEvent {
                kind: AlertKind::FolderGrowth,
                destination_path: change.path,
                bytes: change.logical_delta_bytes,
                snapshot_id: Some(current.id),
                root: Some(root.clone()),
                previous_snapshot_id: Some(previous.id),
            });
        }
        Ok(alerts)
    }
    pub fn apply_settings(&mut self, settings: AppSettings) -> Result<()> {
        settings.validate()?;
        self.settings = settings;
        Ok(())
    }
}
