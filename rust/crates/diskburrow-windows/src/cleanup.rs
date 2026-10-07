use crate::{
    KnownDirectories, NativeFileApi, NativeHandle, OwnerProcessState, RuleEnvironment,
    WindowsNativeFileApi, equals_path, is_within, normalize_local_path,
};
use crate::{
    native::{
        AncestorLease, DIRECTORY, READONLY, REPARSE, SYSTEM, is_cloud, safe_directory,
        validate_directory,
    },
    paths::{canonical, parent},
};
use chrono::{DateTime, Duration, Utc};
use diskburrow_services::{
    CleanupAudit, CleanupItemResult, CleanupOutcome, CleanupReport, FileIdentity, FileObservation,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn check(&self) -> anyhow::Result<()> {
        if self.is_cancelled() {
            anyhow::bail!("Cleanup.Cancelled");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleRoot {
    pub rule_id: String,
    pub path: String,
    /// Minimum age in seconds; strict greater-than comparison.
    pub minimum_age: Option<i64>,
    pub owner_process_name: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupCandidate {
    pub id: Uuid,
    pub rule: RuleRoot,
    pub file: FileObservation,
    pub reason_key: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupWarning {
    pub rule_id: String,
    pub path: Option<String>,
    pub reason_key: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupPlan {
    pub id: Uuid,
    pub created_utc: DateTime<Utc>,
    pub candidates: Vec<CleanupCandidate>,
    pub warnings: Vec<CleanupWarning>,
    pub selected_ids: HashSet<Uuid>,
}
impl CleanupPlan {
    pub fn estimated_data_bytes(&self) -> i64 {
        self.candidates
            .iter()
            .fold(0i64, |n, c| n.saturating_add(c.file.logical_bytes))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualDeleteEntry {
    pub id: Uuid,
    pub root_path: String,
    pub file: FileObservation,
}
impl ManualDeleteEntry {
    pub fn is_directory(&self) -> bool {
        self.file.attributes & DIRECTORY != 0
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualDeleteWarning {
    pub path: Option<String>,
    pub reason_key: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualDeletePlan {
    pub id: Uuid,
    pub created_utc: DateTime<Utc>,
    pub roots: Vec<String>,
    pub entries: Vec<ManualDeleteEntry>,
    pub warnings: Vec<ManualDeleteWarning>,
}
impl ManualDeletePlan {
    pub fn can_execute(&self) -> bool {
        !self.roots.is_empty() && !self.entries.is_empty() && self.warnings.is_empty()
    }
    pub fn file_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_directory()).count()
    }
    pub fn directory_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_directory()).count()
    }
    pub fn estimated_data_bytes(&self) -> i64 {
        self.entries
            .iter()
            .filter(|e| !e.is_directory())
            .fold(0i64, |n, e| n.saturating_add(e.file.logical_bytes))
    }
}
#[derive(Debug, Clone)]
pub struct ApprovedTempRoot {
    path: String,
}
impl ApprovedTempRoot {
    pub fn path(&self) -> &str {
        &self.path
    }
}
struct ManualIssued {
    plan: ManualDeletePlan,
    ancestors: HashMap<String, FileIdentity>,
}
struct CleanupIssued {
    plan: CleanupPlan,
    approved: Vec<String>,
}
#[derive(Default)]
struct Authority {
    manual_generation: u64,
    cleanup_generation: u64,
    manual: Option<ManualIssued>,
    cleanup: Option<CleanupIssued>,
}

/// Call previews and execution on a background thread. Execution accepts only a retained preview ID,
/// not a mutable caller-provided plan. Confirmation is separate and attempts consume the ID.
pub struct CleanupService {
    environment: Arc<dyn RuleEnvironment>,
    known: KnownDirectories,
    application_directory: String,
    data_directory: String,
    api: Arc<dyn NativeFileApi>,
    authority: Mutex<Authority>,
    execution: Mutex<()>,
}
impl CleanupService {
    pub fn new(
        environment: Arc<dyn RuleEnvironment>,
        application_directory: String,
        data_directory: String,
    ) -> Self {
        Self::with_native_api(
            environment,
            application_directory,
            data_directory,
            Arc::new(WindowsNativeFileApi),
        )
    }
    pub fn with_native_api(
        environment: Arc<dyn RuleEnvironment>,
        application_directory: String,
        data_directory: String,
        api: Arc<dyn NativeFileApi>,
    ) -> Self {
        let known = environment.known_directories();
        Self {
            environment,
            known,
            application_directory,
            data_directory,
            api,
            authority: Mutex::new(Authority::default()),
            execution: Mutex::new(()),
        }
    }
    pub fn approve_custom_temp_root(&self, path: &str) -> Option<ApprovedTempRoot> {
        let p = normalize_local_path(path)?;
        self.custom_temp_allowed(&p, &self.environment.known_directories())
            .then_some(ApprovedTempRoot { path: p })
    }
    fn custom_temp_allowed(&self, path: &str, k: &KnownDirectories) -> bool {
        if !k.user_library_roots_verified
            || normalize_local_path(&k.temp_hint).is_none_or(|hint| !equals_path(&hint, path))
            || !self.environment.is_local_path(path)
            || path[3..].split('\\').count() < 2
        {
            return false;
        }
        for blocked in [
            &k.windows,
            &k.program_files,
            &k.program_files_x86,
            &k.program_data,
            &self.application_directory,
            &self.data_directory,
        ] {
            let Some(boundary) = canonical(blocked) else {
                return false;
            };
            if is_within(path, &boundary) || is_within(&boundary, path) {
                return false;
            }
        }
        let Some(appdata) = parent(&k.local_app_data) else {
            return false;
        };
        for broad in [&k.user_profile, &k.local_app_data, &appdata]
            .into_iter()
            .chain(k.user_library_roots.iter())
        {
            let Some(boundary) = canonical(broad) else {
                return false;
            };
            if is_within(&boundary, path) {
                return false;
            }
        }
        safe_directory(self.api.as_ref(), path)
    }
    pub fn discover_rules(&self, approved: &[String]) -> (Vec<RuleRoot>, Vec<CleanupWarning>) {
        let k = self.environment.known_directories();
        let mut roots = Vec::new();
        let mut warnings = Vec::new();
        let Some(local) = normalize_local_path(&k.local_app_data) else {
            return (roots, vec![warning("UserTemp", None, "Cleanup.UnsafeRoot")]);
        };
        self.add_rule(
            &mut roots,
            &mut warnings,
            "UserTemp",
            &format!("{local}\\Temp"),
            Some(604800),
            None,
        );
        let standard = format!("{local}\\Temp");
        let hint = normalize_local_path(&k.temp_hint);
        if hint.as_ref().is_none_or(|p| !equals_path(p, &standard)) {
            if let Some(p) = hint.as_ref().filter(|p| {
                approved.iter().any(|a| equals_path(a, p)) && self.custom_temp_allowed(p, &k)
            }) {
                self.add_rule(&mut roots, &mut warnings, "UserTemp", p, Some(604800), None);
            } else {
                warnings.push(warning(
                    "UserTemp",
                    Some(k.temp_hint.clone()),
                    if !k.user_library_roots_verified {
                        "Cleanup.KnownFoldersUnavailable"
                    } else if hint
                        .as_ref()
                        .is_some_and(|p| !self.environment.is_local_path(p))
                    {
                        "Cleanup.NonLocalVolume"
                    } else {
                        "Cleanup.TempApprovalRequired"
                    },
                ));
            }
        }
        self.add_rule(
            &mut roots,
            &mut warnings,
            "CrashDumps",
            &format!("{local}\\CrashDumps"),
            Some(604800),
            None,
        );
        for (id, owner, user_data) in [
            (
                "ChromeCache",
                "chrome",
                format!("{local}\\Google\\Chrome\\User Data"),
            ),
            (
                "EdgeCache",
                "msedge",
                format!("{local}\\Microsoft\\Edge\\User Data"),
            ),
        ] {
            if !self.environment.is_local_path(&user_data) {
                warnings.push(warning(id, Some(user_data), "Cleanup.NonLocalVolume"));
                continue;
            }
            if !safe_directory(self.api.as_ref(), &user_data) {
                continue;
            }
            match self.environment.check_owner_process(owner) {
                OwnerProcessState::Running => {
                    warnings.push(warning(id, Some(user_data), "Cleanup.OwnerRunning"));
                    continue;
                }
                OwnerProcessState::Unavailable => {
                    warnings.push(warning(id, Some(user_data), "Cleanup.OwnerUnavailable"));
                    continue;
                }
                OwnerProcessState::Closed => {}
            }
            // Hold the full parent chain and User Data directory while profile names are resolved.
            let profiles = (|| -> io::Result<Vec<String>> {
                let lease = AncestorLease::open(self.api.as_ref(), &user_data)?;
                let h = self.api.open_directory(&user_data)?;
                let o = self.api.inspect_handle(&h, &user_data)?;
                validate_directory(self.api.as_ref(), &user_data, &h, &o)?;
                let mut profiles = Vec::new();
                for child in fs::read_dir(&user_data)? {
                    let child = child?;
                    let name = child.file_name().to_string_lossy().into_owned();
                    if name == "Default" || (name.starts_with("Profile ") && name.len() > 8) {
                        profiles.push(child.path().to_string_lossy().into_owned());
                    }
                }
                lease.verify()?;
                Ok(profiles)
            })();
            match profiles {
                Ok(profiles) => {
                    for p in profiles {
                        self.add_rule(
                            &mut roots,
                            &mut warnings,
                            id,
                            &format!("{p}\\Cache\\Cache_Data"),
                            None,
                            Some(owner),
                        );
                    }
                }
                Err(_) => warnings.push(warning(id, Some(user_data), "Cleanup.RootUnavailable")),
            }
        }
        (roots, warnings)
    }
    fn add_rule(
        &self,
        roots: &mut Vec<RuleRoot>,
        warnings: &mut Vec<CleanupWarning>,
        id: &str,
        path: &str,
        age: Option<i64>,
        owner: Option<&str>,
    ) {
        let Some(p) = normalize_local_path(path) else {
            warnings.push(warning(id, Some(path.into()), "Cleanup.UnsafeRoot"));
            return;
        };
        if !self.environment.is_local_path(&p) {
            warnings.push(warning(id, Some(p), "Cleanup.NonLocalVolume"));
            return;
        }
        if !safe_directory(self.api.as_ref(), &p) {
            warnings.push(warning(id, Some(p), "Cleanup.UnsafeRoot"));
            return;
        }
        if !roots.iter().any(|r| equals_path(&r.path, &p)) {
            roots.push(RuleRoot {
                rule_id: id.into(),
                path: p,
                minimum_age: age,
                owner_process_name: owner.map(str::to_owned),
            });
        }
    }
    pub fn preview_cleanup(
        &self,
        excluded: &[String],
        approved: &[String],
        cancel: &Cancellation,
    ) -> anyhow::Result<CleanupPlan> {
        self.preview_cleanup_at(excluded, approved, Utc::now(), cancel)
    }
    pub fn preview_cleanup_at(
        &self,
        excluded: &[String],
        approved: &[String],
        now: DateTime<Utc>,
        cancel: &Cancellation,
    ) -> anyhow::Result<CleanupPlan> {
        let generation = {
            let mut a = self.authority.lock().unwrap();
            a.cleanup = None;
            a.cleanup_generation = a.cleanup_generation.wrapping_add(1);
            a.cleanup_generation
        };
        cancel.check()?;
        let exclusions = excluded
            .iter()
            .filter_map(|p| normalize_local_path(p))
            .collect::<Vec<_>>();
        let (rules, mut warnings) = self.discover_rules(approved);
        let mut candidates = Vec::new();
        let mut visited = HashSet::new();
        for rule in rules {
            let mut pending = vec![rule.path.clone()];
            while let Some(dir) = pending.pop() {
                cancel.check()?;
                if exclusions.iter().any(|p| is_within(&dir, p)) {
                    continue;
                }
                let traversal = (|| -> io::Result<()> {
                    let lease = AncestorLease::open(self.api.as_ref(), &dir)?;
                    let h = self.api.open_directory(&dir)?;
                    let o = self.api.inspect_handle(&h, &dir)?;
                    validate_directory(self.api.as_ref(), &dir, &h, &o)?;
                    for child in fs::read_dir(&dir)? {
                        if cancel.is_cancelled() {
                            break;
                        }
                        let child = child?;
                        let Some(path) = normalize_local_path(&child.path().to_string_lossy())
                        else {
                            continue;
                        };
                        if !is_within(&path, &rule.path)
                            || exclusions.iter().any(|p| is_within(&path, p))
                        {
                            continue;
                        }
                        match self.api.inspect(&path) {
                            Ok(o) => {
                                if o.attributes & REPARSE != 0 || is_cloud(o.attributes) {
                                    continue;
                                }
                                if o.attributes & DIRECTORY != 0 {
                                    pending.push(path);
                                    continue;
                                }
                                if unsafe_file(&o)
                                    || !old_enough(&rule, &o, now)
                                    || !extension_allowed(&rule, &path)
                                    || !visited.insert(path.to_lowercase())
                                {
                                    continue;
                                }
                                candidates.push(CleanupCandidate {
                                    id: Uuid::new_v4(),
                                    rule: rule.clone(),
                                    file: o,
                                    reason_key: match rule.rule_id.as_str() {
                                        "UserTemp" => "Cleanup.OldTemp",
                                        "CrashDumps" => "Cleanup.OldCrashDump",
                                        _ => "Cleanup.BrowserCache",
                                    }
                                    .into(),
                                });
                            }
                            Err(_) => warnings.push(warning(
                                &rule.rule_id,
                                Some(path),
                                "Cleanup.FileUnavailable",
                            )),
                        }
                    }
                    lease.verify()
                })();
                if traversal.is_err() {
                    warnings.push(warning(&rule.rule_id, Some(dir), "Cleanup.RootUnavailable"));
                }
            }
        }
        cancel.check()?;
        candidates.sort_by_key(|c| c.file.path.to_lowercase());
        let plan = CleanupPlan {
            id: Uuid::new_v4(),
            created_utc: now,
            candidates,
            warnings,
            selected_ids: HashSet::new(),
        };
        let mut a = self.authority.lock().unwrap();
        if a.cleanup_generation == generation {
            a.cleanup = Some(CleanupIssued {
                plan: plan.clone(),
                approved: approved.to_vec(),
            });
        }
        Ok(plan)
    }
    pub fn execute_cleanup(
        &self,
        plan_id: Uuid,
        selected_ids: &HashSet<Uuid>,
        confirmed: bool,
        cancel: &Cancellation,
    ) -> anyhow::Result<CleanupReport> {
        if !confirmed {
            anyhow::bail!("Cleanup.ConfirmationRequired");
        }
        let issued = {
            let mut a = self.authority.lock().unwrap();
            let Some(p) = a.cleanup.as_ref().filter(|p| p.plan.id == plan_id) else {
                anyhow::bail!("Cleanup.PlanUnavailable");
            };
            if selected_ids
                .iter()
                .any(|id| !p.plan.candidates.iter().any(|c| c.id == *id))
            {
                anyhow::bail!("Cleanup.InvalidSelection");
            }
            a.cleanup.take().unwrap()
        };
        let _lock = self.execution.lock().unwrap();
        let mut items = Vec::new();
        let rules = if cancel.is_cancelled() || selected_ids.is_empty() {
            Vec::new()
        } else {
            self.discover_rules(&issued.approved).0
        };
        let mut before = HashMap::new();
        for candidate in issued
            .plan
            .candidates
            .iter()
            .filter(|c| selected_ids.contains(&c.id))
        {
            let result = if cancel.is_cancelled() {
                Err(Fault::policy("Cleanup.Cancelled"))
            } else if !rules.iter().any(|r| r == &candidate.rule) {
                Err(Fault::policy("Cleanup.RuleUnavailable"))
            } else {
                before
                    .entry(candidate.file.path[..3].to_owned())
                    .or_insert_with(|| available_bytes(&candidate.file.path[..3]));
                self.delete_candidate(candidate, cancel)
            };
            items.push(item(
                candidate.id,
                &candidate.file,
                &candidate.rule.rule_id,
                &candidate.rule.path,
                result,
            ));
        }
        let mut measured = !before.is_empty();
        let mut delta = 0i64;
        for (volume, previous) in before {
            match (previous, available_bytes(&volume)) {
                (Some(b), Some(a)) => {
                    if let Some(d) = a.checked_sub(b).and_then(|d| delta.checked_add(d)) {
                        delta = d
                    } else {
                        measured = false
                    }
                }
                _ => measured = false,
            }
        }
        Ok(CleanupReport {
            plan_id,
            items,
            was_cancelled: cancel.is_cancelled(),
            free_space_delta_bytes: if measured { delta } else { 0 },
            free_space_delta_available: measured,
        })
    }
    fn delete_candidate(&self, c: &CleanupCandidate, cancel: &Cancellation) -> Result<(), Fault> {
        let path = &c.file.path;
        if canonical(path).is_none()
            || !is_within(path, &c.rule.path)
            || equals_path(path, &c.rule.path)
            || !self.environment.is_local_path(path)
            || unsafe_file(&c.file)
            || !extension_allowed(&c.rule, path)
        {
            return Err(Fault::policy("Cleanup.UnsafePath"));
        }
        let lease = AncestorLease::open(self.api.as_ref(), path).map_err(Fault::io)?;
        if cancel.is_cancelled() {
            return Err(Fault::policy("Cleanup.Cancelled"));
        }
        let target = self.api.open_target(path).map_err(Fault::io)?;
        let initial = self.api.inspect_handle(&target, path).map_err(Fault::io)?;
        self.validate_candidate(c, &initial, &target, lease.volume())?;
        lease.verify().map_err(Fault::io)?;
        if !self.environment.is_local_path(path) {
            return Err(Fault::policy("Cleanup.NonLocalVolume"));
        }
        if c.rule
            .owner_process_name
            .as_ref()
            .is_some_and(|o| self.environment.check_owner_process(o) != OwnerProcessState::Closed)
        {
            return Err(Fault::policy("Cleanup.OwnerUnavailable"));
        }
        let latest = self.api.inspect_handle(&target, path).map_err(Fault::io)?;
        self.validate_candidate(c, &latest, &target, lease.volume())?;
        if c.rule
            .owner_process_name
            .as_ref()
            .is_some_and(|o| self.environment.check_owner_process(o) != OwnerProcessState::Closed)
        {
            return Err(Fault::policy("Cleanup.OwnerUnavailable"));
        }
        if cancel.is_cancelled() {
            return Err(Fault::policy("Cleanup.Cancelled"));
        }
        self.api.mark_for_deletion(&target).map_err(Fault::io)
    }
    fn validate_candidate(
        &self,
        c: &CleanupCandidate,
        o: &FileObservation,
        h: &NativeHandle,
        volume: u64,
    ) -> Result<(), Fault> {
        if unsafe_file(o) {
            return Err(Fault::policy("Cleanup.UnsafeFile"));
        }
        if !same(&c.file, o, true) {
            return Err(Fault::changed("Cleanup.FileChanged"));
        }
        if canonical(&self.api.final_path(h).map_err(Fault::io)?)
            .is_none_or(|p| !equals_path(&p, &c.file.path))
            || o.identity.as_ref().is_none_or(|id| id.volume != volume)
        {
            return Err(Fault::policy("Cleanup.UnsafePath"));
        }
        if !old_enough(&c.rule, o, Utc::now()) {
            return Err(Fault::policy("Cleanup.TooRecent"));
        }
        Ok(())
    }
    fn allowed_manual(&self, path: &str) -> bool {
        let k = &self.known;
        if !k.user_library_roots_verified
            || canonical(path).is_none()
            || path.len() == 3
            || !self.environment.is_local_path(path)
        {
            return false;
        }
        for blocked in [
            &k.windows,
            &k.program_files,
            &k.program_files_x86,
            &k.program_data,
            &self.application_directory,
            &self.data_directory,
        ] {
            let Some(boundary) = canonical(blocked) else {
                return false;
            };
            if is_within(path, &boundary) || is_within(&boundary, path) {
                return false;
            }
        }
        let Some(appdata) = parent(&k.local_app_data) else {
            return false;
        };
        let roaming = format!("{}\\AppData\\Roaming", k.user_profile);
        for broad in [&k.user_profile, &k.local_app_data, &appdata, &roaming]
            .into_iter()
            .chain(k.user_library_roots.iter())
        {
            let Some(boundary) = canonical(broad) else {
                return false;
            };
            if is_within(&boundary, path) {
                return false;
            }
        }
        true
    }
    pub fn preview_manual(
        &self,
        selected: &[String],
        cancel: &Cancellation,
    ) -> anyhow::Result<ManualDeletePlan> {
        let generation = {
            let mut a = self.authority.lock().unwrap();
            a.manual = None;
            a.manual_generation = a.manual_generation.wrapping_add(1);
            a.manual_generation
        };
        cancel.check()?;
        let mut warnings = Vec::new();
        let mut roots: Vec<String> = Vec::new();
        for selected in selected {
            cancel.check()?;
            match canonical(selected).filter(|p| self.allowed_manual(p)) {
                Some(p) => {
                    if !roots.iter().any(|r| equals_path(r, &p)) {
                        roots.push(p);
                    }
                }
                None => warnings.push(ManualDeleteWarning {
                    path: Some(selected.clone()),
                    reason_key: "Manual.ProtectedPath".into(),
                }),
            }
        }
        let all = roots.clone();
        roots.retain(|p| {
            !all.iter()
                .any(|other| !equals_path(other, p) && is_within(p, other))
        });
        roots.sort_by_key(|p| p.to_lowercase());
        let mut entries = Vec::new();
        let mut ancestors = HashMap::new();
        for root in &roots {
            match self.inventory(root, &mut entries, &mut ancestors, cancel) {
                Ok(()) => {}
                Err(e) => {
                    if cancel.is_cancelled() {
                        cancel.check()?;
                    }
                    warnings.push(ManualDeleteWarning {
                        path: Some(e.path.unwrap_or_else(|| root.clone())),
                        reason_key: e.reason.into(),
                    });
                }
            }
        }
        cancel.check()?;
        let plan = ManualDeletePlan {
            id: Uuid::new_v4(),
            created_utc: Utc::now(),
            roots,
            entries,
            warnings,
        };
        let mut a = self.authority.lock().unwrap();
        if plan.can_execute() && a.manual_generation == generation {
            a.manual = Some(ManualIssued {
                plan: plan.clone(),
                ancestors,
            });
        }
        Ok(plan)
    }
    fn inventory(
        &self,
        root: &str,
        entries: &mut Vec<ManualDeleteEntry>,
        ancestors: &mut HashMap<String, FileIdentity>,
        cancel: &Cancellation,
    ) -> Result<(), Fault> {
        let mut pending = vec![root.to_owned()];
        while let Some(path) = pending.pop() {
            let result = (|| -> Result<(), Fault> {
                if cancel.is_cancelled() {
                    return Err(Fault::policy("Manual.Cancelled"));
                }
                if !self.allowed_manual(&path) {
                    return Err(Fault::changed("Manual.ProtectedPath"));
                }
                let lease =
                    AncestorLease::open(self.api.as_ref(), &path).map_err(Fault::manual_io)?;
                merge_ancestors(ancestors, lease.identities())?;
                let handle = self.api.open_metadata(&path).map_err(Fault::manual_io)?;
                let o = self
                    .api
                    .inspect_handle(&handle, &path)
                    .map_err(Fault::manual_io)?;
                self.ensure_manual_safe(&path, &handle, &o)?;
                if o.attributes & DIRECTORY != 0 {
                    let dir = self.api.open_directory(&path).map_err(Fault::manual_io)?;
                    let pinned = self
                        .api
                        .inspect_handle(&dir, &path)
                        .map_err(Fault::manual_io)?;
                    self.ensure_manual_safe(&path, &dir, &pinned)?;
                    if pinned.identity != o.identity {
                        return Err(Fault::changed("Manual.Changed"));
                    }
                    merge_ancestors(
                        ancestors,
                        [(path.as_str(), o.identity.as_ref().unwrap())].into_iter(),
                    )?;
                    for child in fs::read_dir(&path).map_err(Fault::manual_io)? {
                        let child = child.map_err(Fault::manual_io)?;
                        pending.push(child.path().to_string_lossy().into_owned());
                    }
                }
                entries.push(ManualDeleteEntry {
                    id: Uuid::new_v4(),
                    root_path: root.into(),
                    file: o,
                });
                lease.verify().map_err(Fault::manual_io)?;
                Ok(())
            })();
            if let Err(mut e) = result {
                e.path = Some(path);
                return Err(e);
            }
        }
        Ok(())
    }
    fn ensure_manual_safe(
        &self,
        path: &str,
        h: &NativeHandle,
        o: &FileObservation,
    ) -> Result<(), Fault> {
        if o.identity.is_none()
            || o.logical_bytes < 0
            || o.attributes & (REPARSE | READONLY | SYSTEM) != 0
            || is_cloud(o.attributes)
            || canonical(&self.api.final_path(h).map_err(Fault::manual_io)?)
                .is_none_or(|p| !equals_path(path, &p))
        {
            return Err(Fault::changed("Manual.UnsafePath"));
        }
        Ok(())
    }
    pub fn execute_manual(
        &self,
        plan_id: Uuid,
        confirmed: bool,
        cancel: &Cancellation,
    ) -> anyhow::Result<CleanupReport> {
        if !confirmed {
            anyhow::bail!("Manual.ConfirmationRequired");
        }
        let issued = {
            let mut a = self.authority.lock().unwrap();
            if a.manual.as_ref().is_none_or(|p| p.plan.id != plan_id) {
                anyhow::bail!("Manual.PlanUnavailable");
            }
            a.manual.take().unwrap()
        };
        let _lock = self.execution.lock().unwrap();
        let mut items = Vec::new();
        for root in &issued.plan.roots {
            let mut entries = issued
                .plan
                .entries
                .iter()
                .filter(|e| equals_path(&e.root_path, root))
                .collect::<Vec<_>>();
            if let Err(e) = self.revalidate_inventory(root, &entries, &issued.ancestors, cancel) {
                for entry in entries {
                    items.push(item(
                        entry.id,
                        &entry.file,
                        "ManualSelection",
                        &entry.root_path,
                        Err(e.clone()),
                    ));
                }
                continue;
            }
            entries.sort_by(|a, b| {
                a.is_directory()
                    .cmp(&b.is_directory())
                    .then_with(|| b.file.path.len().cmp(&a.file.path.len()))
            });
            for entry in entries {
                let result = if cancel.is_cancelled() {
                    Err(Fault::policy("Manual.Cancelled"))
                } else {
                    self.delete_manual(entry, &issued.ancestors, cancel)
                };
                items.push(item(
                    entry.id,
                    &entry.file,
                    "ManualSelection",
                    &entry.root_path,
                    result,
                ));
            }
        }
        Ok(CleanupReport {
            plan_id,
            items,
            was_cancelled: cancel.is_cancelled(),
            free_space_delta_bytes: 0,
            free_space_delta_available: false,
        })
    }
    fn revalidate_inventory(
        &self,
        root: &str,
        entries: &[&ManualDeleteEntry],
        expected: &HashMap<String, FileIdentity>,
        cancel: &Cancellation,
    ) -> Result<(), Fault> {
        if cancel.is_cancelled() {
            return Err(Fault::policy("Manual.Cancelled"));
        }
        let lease = AncestorLease::open(self.api.as_ref(), root).map_err(Fault::manual_io)?;
        verify_expected(&lease, expected)?;
        let mut fresh = Vec::new();
        let mut ancestors = HashMap::new();
        self.inventory(root, &mut fresh, &mut ancestors, cancel)?;
        if fresh.len() != entries.len() {
            return Err(Fault::changed("Manual.Changed"));
        }
        let reviewed = entries
            .iter()
            .map(|e| (e.file.path.to_lowercase(), &e.file))
            .collect::<HashMap<_, _>>();
        for e in fresh {
            if reviewed
                .get(&e.file.path.to_lowercase())
                .is_none_or(|o| !same(o, &e.file, true))
            {
                return Err(Fault::changed("Manual.Changed"));
            }
        }
        for (path, id) in ancestors {
            if expected.get(&path) != Some(&id) {
                return Err(Fault::changed("Manual.Changed"));
            }
        }
        lease.verify().map_err(Fault::manual_io)
    }
    fn delete_manual(
        &self,
        e: &ManualDeleteEntry,
        expected: &HashMap<String, FileIdentity>,
        cancel: &Cancellation,
    ) -> Result<(), Fault> {
        let path = &e.file.path;
        if !self.allowed_manual(path) {
            return Err(Fault::changed("Manual.ProtectedPath"));
        }
        let lease = AncestorLease::open(self.api.as_ref(), path).map_err(Fault::manual_io)?;
        verify_expected(&lease, expected)?;
        if cancel.is_cancelled() {
            return Err(Fault::policy("Manual.Cancelled"));
        }
        let target = self.api.open_target(path).map_err(Fault::manual_io)?;
        let o = self
            .api
            .inspect_handle(&target, path)
            .map_err(Fault::manual_io)?;
        self.ensure_manual_safe(path, &target, &o)?;
        if !same(&e.file, &o, !e.is_directory())
            || o.identity
                .as_ref()
                .is_none_or(|id| id.volume != lease.volume())
        {
            return Err(Fault::changed("Manual.Changed"));
        }
        if e.is_directory()
            && fs::read_dir(path)
                .map_err(Fault::manual_io)?
                .next()
                .transpose()
                .map_err(Fault::manual_io)?
                .is_some()
        {
            return Err(Fault::changed("Manual.NotEmpty"));
        }
        lease.verify().map_err(Fault::manual_io)?;
        let latest = self
            .api
            .inspect_handle(&target, path)
            .map_err(Fault::manual_io)?;
        self.ensure_manual_safe(path, &target, &latest)?;
        if !same(&e.file, &latest, !e.is_directory()) {
            return Err(Fault::changed("Manual.Changed"));
        }
        if cancel.is_cancelled() {
            return Err(Fault::policy("Manual.Cancelled"));
        }
        // This kernel operation rejects nonempty directories, including a late unreviewed child.
        self.api
            .mark_for_deletion(&target)
            .map_err(Fault::manual_io)
    }
}

fn same(a: &FileObservation, b: &FileObservation, data: bool) -> bool {
    a.identity.is_some()
        && a.identity == b.identity
        && (a.attributes & DIRECTORY) == (b.attributes & DIRECTORY)
        && (!data
            || (a.logical_bytes == b.logical_bytes
                && a.modified_utc == b.modified_utc
                && a.link_count == b.link_count))
}
fn unsafe_file(o: &FileObservation) -> bool {
    o.identity.is_none()
        || o.logical_bytes < 0
        || o.attributes & (DIRECTORY | REPARSE | READONLY | SYSTEM) != 0
        || is_cloud(o.attributes)
}
fn old_enough(rule: &RuleRoot, o: &FileObservation, now: DateTime<Utc>) -> bool {
    rule.minimum_age
        .is_none_or(|age| o.modified_utc < now - Duration::seconds(age))
}
fn extension_allowed(rule: &RuleRoot, path: &str) -> bool {
    rule.rule_id != "CrashDumps" || path.to_ascii_lowercase().ends_with(".dmp")
}
fn warning(id: &str, path: Option<String>, reason: &str) -> CleanupWarning {
    CleanupWarning {
        rule_id: id.into(),
        path,
        reason_key: reason.into(),
    }
}
fn merge_ancestors<'a>(
    expected: &mut HashMap<String, FileIdentity>,
    observed: impl Iterator<Item = (&'a str, &'a FileIdentity)>,
) -> Result<(), Fault> {
    for (p, id) in observed {
        let key = p.to_lowercase();
        if expected.get(&key).is_some_and(|previous| previous != id) {
            return Err(Fault::changed("Manual.Changed"));
        }
        expected.insert(key, id.clone());
    }
    Ok(())
}
fn verify_expected(
    lease: &AncestorLease<'_>,
    expected: &HashMap<String, FileIdentity>,
) -> Result<(), Fault> {
    for (p, id) in lease.identities() {
        if expected.get(&p.to_lowercase()) != Some(id) {
            return Err(Fault::changed("Manual.Changed"));
        }
    }
    lease.verify().map_err(Fault::manual_io)
}
#[derive(Clone)]
struct Fault {
    outcome: CleanupOutcome,
    reason: &'static str,
    path: Option<String>,
}
impl Fault {
    fn changed(reason: &'static str) -> Self {
        Self {
            outcome: CleanupOutcome::SkippedChanged,
            reason,
            path: None,
        }
    }
    fn policy(reason: &'static str) -> Self {
        Self {
            outcome: CleanupOutcome::SkippedPolicy,
            reason,
            path: None,
        }
    }
    fn io(e: io::Error) -> Self {
        Self::from_io(e, false)
    }
    fn manual_io(e: io::Error) -> Self {
        Self::from_io(e, true)
    }
    fn from_io(e: io::Error, manual: bool) -> Self {
        Self {
            outcome: match e.raw_os_error() {
                Some(2 | 3) => CleanupOutcome::Missing,
                Some(5 | 32 | 33) => CleanupOutcome::SkippedBusy,
                _ => {
                    if e.kind() == io::ErrorKind::Other || e.kind() == io::ErrorKind::InvalidInput {
                        CleanupOutcome::SkippedChanged
                    } else {
                        CleanupOutcome::Failed
                    }
                }
            },
            reason: if manual {
                "Manual.Unavailable"
            } else {
                "Cleanup.Unavailable"
            },
            path: None,
        }
    }
}
fn item(
    id: Uuid,
    file: &FileObservation,
    rule_id: &str,
    root: &str,
    result: Result<(), Fault>,
) -> CleanupItemResult {
    let (outcome, reason_key) = match result {
        Ok(()) => (CleanupOutcome::Deleted, None),
        Err(e) => (e.outcome, Some(e.reason.into())),
    };
    CleanupItemResult {
        candidate_id: id,
        outcome,
        reason_key,
        audit: Some(CleanupAudit {
            path: file.path.clone(),
            rule_id: rule_id.into(),
            rule_root_path: root.into(),
            reviewed_modified_utc: file.modified_utc,
            logical_bytes: if file.attributes & DIRECTORY != 0 {
                0
            } else {
                file.logical_bytes
            },
            allocated_bytes: if file.attributes & DIRECTORY != 0 {
                None
            } else {
                file.allocated_bytes
            },
            link_count: file.link_count,
            identity: file.identity.clone(),
        }),
    }
}
fn available_bytes(volume: &str) -> Option<i64> {
    let p: Vec<u16> = volume.encode_utf16().chain(Some(0)).collect();
    let mut available = 0u64;
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            p.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then(|| i64::try_from(available).ok()).flatten()
}
