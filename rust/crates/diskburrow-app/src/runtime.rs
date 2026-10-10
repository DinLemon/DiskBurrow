//! UI-owned state. Workers publish through bounded operation identities; no worker touches GPUI.
use crate::git_inspection::{self, GitInspection};
use crate::{
    contract::*,
    locale::{gb, parse_threshold, text},
    operation::{Admission, Coordinator, Purpose},
    platform::{self, PlatformBridge, PlatformEvent},
};
use anyhow::{Result, bail, ensure};
use chrono::{DateTime, Local, Utc};
use diskburrow_engine::{
    AgeBand, EntryClassification, LiveIndex, MapMetric, MapNavigation, MapProjection, OTHERS_INDEX,
    Recommendation,
};
use diskburrow_services::*;
use diskburrow_windows::{
    Cancellation, CleanupPlan, CleanupService, ManualDeletePlan, ManualReclaimProjection,
    RuleEnvironment, WindowsRuleEnvironment, equals_path, is_within, normalize_local_path,
    project_manual_reclaim,
};
use disktree_core::scan::{ScanHandle, ScanOptions, ScanProgress as CoreProgress};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use uuid::Uuid;
const ROW_LIMIT: usize = 2000;
struct Active {
    token: u64,
    cancel: Cancellation,
    progress: Option<Arc<CoreProgress>>,
    join: JoinHandle<()>,
    automatic_request: Option<(u64, String)>,
    scan_root: Option<String>,
    deletion_root: Option<String>,
}
struct GitJob {
    generation: u64,
    cancel: Cancellation,
    join: JoinHandle<()>,
}
struct ForecastJob {
    generation: u64,
    scan_id: Uuid,
    cancel: Cancellation,
    join: JoinHandle<()>,
}
/// Keep canceled metadata leases owned until they are released, including spawn failure.
struct ObservationBarrier(Vec<JoinHandle<()>>);
impl ObservationBarrier {
    fn wait(mut self) {
        for join in self.0.drain(..) {
            let _ = join.join();
        }
    }
}
impl Drop for ObservationBarrier {
    fn drop(&mut self) {
        for join in self.0.drain(..) {
            let _ = join.join();
        }
    }
}
#[derive(Clone)]
struct ForecastObservation {
    space: VolumeSpace,
    observed_utc: DateTime<Utc>,
    projection: ManualReclaimProjection,
    partial: bool,
}
#[derive(Default)]
struct Analysis {
    classifications: Vec<EntryClassification>,
    recommendations: Vec<Recommendation>,
}
enum Work {
    Scan {
        index: Arc<LiveIndex>,
        snapshot: Arc<ScanSnapshot>,
        history: Vec<ScanSnapshot>,
        history_error: Option<String>,
        cache: Option<crate::scan_cache::ScanCache>,
        reused: bool,
        focus_path: Option<String>,
        analysis: Analysis,
    },
    CleanupPreview(CleanupPlan),
    ManualPreview {
        plan: ManualDeletePlan,
        projection: Option<ManualReclaimProjection>,
        git: Vec<(String, GitInspection)>,
        git_limited: bool,
    },
    Deleted(CleanupReport),
    SettingsSaved(AppSettings),
    Exported,
    History {
        root: String,
        history: Vec<ScanSnapshot>,
    },
    Journal(Option<String>),
}
enum Event {
    ForecastFinished {
        generation: u64,
        scan_id: Uuid,
        observation: Option<ForecastObservation>,
    },
    Finished(u64, Result<Work>),
    Progress(u64, Arc<CoreProgress>),
    GitFinished {
        generation: u64,
        path: String,
        inspection: GitInspection,
    },
}
struct MapCache {
    key: (u64, u32, u32),
    tiles: Vec<MapTile>,
}
pub struct Runtime {
    view: UiView,
    data_dir: PathBuf,
    service: Arc<CleanupService>,
    coordinator: Coordinator,
    active: Option<Active>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    live: Option<Arc<LiveIndex>>,
    scan_cache: Option<crate::scan_cache::ScanCache>,
    analysis: Analysis,
    git_generation: u64,
    git_job: Option<GitJob>,
    git_desired: Option<(u64, String)>,
    git_result: Option<(String, GitInspection)>,
    forecast_generation: u64,
    forecast_job: Option<ForecastJob>,
    forecast_result: Option<ForecastObservation>,
    forecast_attempted: bool,
    pending_launch: Option<crate::cli::LaunchOverrides>,
    scan_threads: Option<disktree_core::scan_threads::ScanThreads>,
    map_open_requested: bool,
    refreshing_deletion: bool,
    snapshot: Option<Arc<ScanSnapshot>>,
    known_cache_roots: (String, String),
    volume_observation: Option<VolumeObservation>,
    history: Vec<ScanSnapshot>,
    history_selection: Option<Uuid>,
    marks: HashSet<String>,
    cleanup_plan: Option<CleanupPlan>,
    manual_plan: Option<ManualDeletePlan>,
    cleanup_selection: HashSet<Uuid>,
    reviewed_cleanup: HashSet<Uuid>,
    cleanup_filter: String,
    cleanup_category: String,
    last_report: Option<CleanupReport>,
    nav: MapNavigation,
    map_revision: u64,
    map_cache: RefCell<Option<MapCache>>,
    settings_errors: HashMap<&'static str, String>,
    status_key: String,
    started: Instant,
    next_monitor: Instant,
    scheduler: MonitoringScheduler,
    alerts: AlertPolicy,
    monitoring: bool,
    bridge: Option<PlatformBridge>,
    platform_rx: Option<Receiver<PlatformEvent>>,
    show_requested: bool,
    exiting: bool,
    journal_workers: Vec<JoinHandle<()>>,
}
impl Runtime {
    pub fn new(data_dir: PathBuf) -> Result<Self> {
        let mut store = SettingsStore::new(data_dir.clone());
        let settings = store.load();
        let recovered = store.last_user_message.clone();
        let alert_state = store.load_alert_state();
        let recovered = match (recovered, store.last_user_message) {
            (Some(a), Some(b)) => Some(format!("{a}; {b}")),
            (a, b) => a.or(b),
        };
        let root = platform::system_root();
        ensure!(!root.is_empty(), "System volume could not be resolved");
        let service = Arc::new(CleanupService::new(
            Arc::new(WindowsRuleEnvironment),
            std::env::current_exe()?
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Application directory missing"))?
                .to_string_lossy()
                .into_owned(),
            data_dir.to_string_lossy().into_owned(),
        ));
        let scheduler =
            MonitoringScheduler::new(settings.clone(), Utc::now(), Duration::ZERO, root.clone())?;
        let alerts = AlertPolicy::new(settings.clone(), root.clone(), alert_state)?;
        let (tx, rx) = mpsc::channel();
        let view = UiView {
            settings: settings.clone(),
            root: root.clone(),
            busy: false,
            paused: settings.paused,
            status: text(&settings.language, "Status.Ready"),
            error: recovered,
            overview: vec![],
            volume_stamp: String::new(),
            issue_rows: vec![],
            folders: vec![],
            files: vec![],
            history: vec![],
            changes: vec![],
            history_heading: String::new(),
            cleanup: vec![],
            cleanup_warnings: vec![],
            cleanup_results: vec![],
            observed_caches: vec![],
            recommendations: vec![],
            cleanup_summary: String::new(),
            cleanup_categories: vec![],
            cleanup_category: "All".into(),
            review: None,
            selected_count: 0,
            map_marked: vec![],
            map_marked_summary: String::new(),
            map_forecast: String::new(),
            map_covering_parent: String::new(),
            map_path: String::new(),
            map_breadcrumbs: vec![],
            map_parents: vec![],
            map_visible_summary: String::new(),
            scan_reuse_notice: String::new(),
            map_objects: vec![],
            map_matches: 0,
            map_query: String::new(),
            map_global: false,
            map_isolate: false,
            map_metric: 0,
            map_size_metric: 0,
            map_color: 0,
            map_zoom: 1.0,
            map_has_data: false,
            focused_path: String::new(),
            focused_summary: String::new(),
            git_summary: String::new(),
            drives: platform::list_drives(),
            can_cleanup: false,
            can_manual: false,
        };
        let mut result = Self {
            view,
            data_dir,
            service,
            coordinator: Coordinator::default(),
            active: None,
            tx,
            rx,
            live: None,
            scan_cache: None,
            analysis: Analysis::default(),
            git_generation: 0,
            git_job: None,
            git_desired: None,
            git_result: None,
            forecast_generation: 0,
            forecast_job: None,
            forecast_result: None,
            forecast_attempted: false,
            pending_launch: None,
            scan_threads: None,
            map_open_requested: false,
            refreshing_deletion: false,
            snapshot: None,
            volume_observation: None,
            known_cache_roots: {
                let known = WindowsRuleEnvironment.known_directories();
                (known.user_profile, known.local_app_data)
            },
            history: vec![],
            history_selection: None,
            marks: HashSet::new(),
            cleanup_plan: None,
            manual_plan: None,
            cleanup_selection: HashSet::new(),
            reviewed_cleanup: HashSet::new(),
            cleanup_filter: String::new(),
            cleanup_category: "All".into(),
            last_report: None,
            nav: MapNavigation::new(),
            map_revision: 0,
            map_cache: RefCell::new(None),
            settings_errors: HashMap::new(),
            status_key: "Status.Ready".into(),
            started: Instant::now(),
            next_monitor: Instant::now(),
            scheduler,
            alerts,
            monitoring: false,
            bridge: None,
            platform_rx: None,
            show_requested: false,
            exiting: false,
            journal_workers: vec![],
        };
        result.refresh_history();
        result.rebuild();
        Ok(result)
    }
    pub fn attach_bridge(&mut self, bridge: PlatformBridge, receiver: Receiver<PlatformEvent>) {
        if self.view.settings.autostart
            && let Ok(exe) = std::env::current_exe()
            && platform::AutostartRegistration::new(&exe).is_ok_and(|r| {
                r.is_path_changed(&platform::WindowsRunRegistry)
                    .unwrap_or(true)
            })
        {
            self.view.error = Some(self.label("Settings.AutostartMoved"));
        }
        bridge.update(&self.view.settings.language, self.view.paused);
        self.bridge = Some(bridge);
        self.platform_rx = Some(receiver);
        self.monitoring = true;
    }
    pub fn view(&self) -> &UiView {
        &self.view
    }
    /// Identity of the completed scan whose numeric map indices are currently live.
    pub fn scan_id(&self) -> Option<Uuid> {
        self.snapshot.as_ref().map(|snapshot| snapshot.id)
    }
    pub fn take_show(&mut self) -> bool {
        std::mem::take(&mut self.show_requested)
    }
    pub fn take_map_open(&mut self) -> bool {
        std::mem::take(&mut self.map_open_requested)
    }
    pub fn forecast_pending(&self) -> bool {
        self.forecast_job.is_some()
    }
    pub fn queue_launch(&mut self, launch: crate::cli::LaunchOverrides) -> Result<()> {
        launch.validate()?;
        ensure!(!self.exiting, "Application is shutting down");
        self.show_requested = true;
        self.map_open_requested = true;
        self.pending_launch = Some(launch);
        Ok(())
    }
    pub fn apply_launch(&mut self, launch: crate::cli::LaunchOverrides) -> Result<()> {
        launch.validate()?;
        if self.view.busy {
            return self.queue_launch(launch);
        }
        ensure!(!self.view.busy && !self.exiting, "Application is busy");
        let root = launch
            .root
            .clone()
            .unwrap_or_else(|| self.view.root.clone());
        ensure!(
            platform::is_local_path(&root),
            "Only a confirmed local directory can be scanned"
        );
        let threads = launch
            .threads
            .map(|policy| {
                policy.to_scan_threads(thread::available_parallelism().map_or(1, |n| n.get()))
            })
            .transpose()?;
        self.command(Command::SetRoot(root));
        if let Some(hidden) = launch.show_hidden {
            self.view.settings.show_hidden = hidden;
        }
        if let Some(depth) = launch.depth {
            self.view.settings.map_depth = depth;
        }
        if let Some(apparent) = launch.apparent_size {
            self.view.map_size_metric = u8::from(apparent);
        }
        if launch.metric == Some(1) {
            self.view.map_metric = 2;
        } else if launch.metric == Some(0) || launch.apparent_size.is_some() {
            self.view.map_metric = self.view.map_size_metric;
        }
        if threads.is_some() {
            self.scan_threads = threads;
        }
        self.map_open_requested = true;
        self.command(Command::Scan(false));
        Ok(())
    }
    pub fn should_exit(&self) -> bool {
        self.exiting
            && !self.coordinator.is_busy()
            && self.git_job.is_none()
            && self.forecast_job.is_none()
            && self.journal_workers.iter().all(JoinHandle::is_finished)
    }
    fn label(&self, key: &str) -> String {
        text(&self.view.settings.language, key)
    }
    fn status(&mut self, key: &str) {
        self.status_key = key.into();
        self.view.status = self.label(key);
    }
    fn error(&mut self, error: impl std::fmt::Display) {
        let error = error.to_string();
        self.view.error = Some(self.label(&error));
        self.status("Status.Error");
    }
    fn invalidate_review(&mut self) {
        self.view.review = None;
        self.manual_plan = None;
        self.reviewed_cleanup.clear();
        if self.status_key == "Status.Analyzing" {
            self.cancel();
        }
    }
    fn map_changed(&mut self) {
        self.map_revision = self.map_revision.wrapping_add(1);
        self.map_cache.borrow_mut().take();
    }
    fn cancel(&mut self) {
        self.invalidate_forecast();
        self.forecast_attempted = true;
        self.coordinator.cancel();
        if let Some(active) = &self.active {
            active.cancel.cancel();
            if let Some(progress) = &active.progress {
                progress.cancel();
            }
        }
        if let Some(job) = &self.git_job {
            job.cancel.cancel();
        }
        self.git_desired = None;
    }
    fn invalidate_forecast(&mut self) {
        self.forecast_generation = self.forecast_generation.wrapping_add(1);
        self.forecast_result = None;
        self.forecast_attempted = false;
        if let Some(job) = &self.forecast_job {
            job.cancel.cancel();
        }
    }
    fn start_forecast(&mut self) {
        if self.forecast_job.is_some()
            || self.forecast_attempted
            || self.view.busy
            || self.exiting
            || self.marks.is_empty()
        {
            return;
        }
        let Some(snapshot) = &self.snapshot else {
            return;
        };
        if !equals_path(&snapshot.root, &self.view.root) {
            return;
        }
        self.forecast_attempted = true;
        let scan_id = snapshot.id;
        let root = snapshot.root.clone();
        let generation = self.forecast_generation;
        let selected = self
            .marks
            .iter()
            .take(ROW_LIMIT + 1)
            .cloned()
            .collect::<Vec<_>>();
        let service = self.service.clone();
        let cancel = Cancellation::default();
        let child = cancel.clone();
        let tx = self.tx.clone();
        match thread::Builder::new()
            .name("diskburrow-forecast".into())
            .spawn(move || {
                let observation = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<ForecastObservation> {
                        use diskburrow_windows::{NativeFileApi as _, WindowsNativeFileApi};
                        let api = WindowsNativeFileApi;
                        let before = api.inspect(&root)?;
                        ensure!(
                            before.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) == 0,
                            "Unsafe root"
                        );
                        let identity = before
                            .identity
                            .ok_or_else(|| anyhow::anyhow!("Unknown volume identity"))?;
                        let plan = service.observe_manual(&selected, &child)?;
                        ensure!(!child.is_cancelled(), "Cancelled");
                        let space = platform::volume_space(&root)?;
                        let after = api.inspect(&root)?;
                        ensure!(
                            after.identity.as_ref() == Some(&identity)
                                && after.attributes == before.attributes,
                            "Root changed"
                        );
                        ensure!(!child.is_cancelled(), "Cancelled");
                        Ok(ForecastObservation {
                            space,
                            observed_utc: Utc::now(),
                            projection: project_manual_reclaim(&plan, identity.volume),
                            partial: !plan.warnings.is_empty(),
                        })
                    },
                ))
                .ok()
                .and_then(Result::ok);
                let _ = tx.send(Event::ForecastFinished {
                    generation,
                    scan_id,
                    observation,
                });
            }) {
            Ok(join) => {
                self.forecast_job = Some(ForecastJob {
                    generation,
                    scan_id,
                    cancel,
                    join,
                })
            }
            Err(_) => self.forecast_result = None,
        }
    }
    fn clear_git(&mut self) {
        self.git_generation = self.git_generation.wrapping_add(1);
        self.git_desired = None;
        self.git_result = None;
        if let Some(job) = &self.git_job {
            job.cancel.cancel();
        }
    }
    fn select_git(&mut self) {
        self.clear_git();
        if self.exiting {
            return;
        }
        if let Some(index) = &self.live
            && let Some(focused) = self.nav.focused
            && let Some(entry) = index.entries.get(focused)
            && entry.directory
            && entry.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) == 0
        {
            self.git_desired = Some((self.git_generation, index.path(focused)));
        }
        self.start_git_job();
    }
    fn start_git_job(&mut self) {
        if self.git_job.is_some() || self.exiting {
            return;
        }
        let Some((generation, path)) = self.git_desired.take() else {
            return;
        };
        let cancel = Cancellation::default();
        let child = cancel.clone();
        let tx = self.tx.clone();
        match thread::Builder::new()
            .name("diskburrow-git".into())
            .spawn(move || {
                let inspection = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    git_inspection::inspect(&path, &child)
                }))
                .unwrap_or(GitInspection {
                    repository: git_inspection::RepositoryState::Unknown,
                    changed: None,
                    untracked: None,
                    stash: None,
                    ahead: None,
                    uncertainty: Some(git_inspection::GitUncertainty::CommandFailed),
                });
                let _ = tx.send(Event::GitFinished {
                    generation,
                    path,
                    inspection,
                });
            }) {
            Ok(join) => {
                self.git_job = Some(GitJob {
                    generation,
                    cancel,
                    join,
                })
            }
            Err(_) => self.view.git_summary = self.label("Git.Unknown"),
        }
    }
    fn spawn(
        &mut self,
        purpose: Purpose,
        status: &str,
        work: impl FnOnce(Cancellation, Sender<Event>, u64) -> Result<Work> + Send + 'static,
    ) {
        if self.exiting {
            return;
        }
        let scan_root = match &purpose {
            Purpose::Scan { root, .. } => Some(root.clone()),
            _ => None,
        };
        let deletion_root = if purpose == Purpose::Cleanup {
            self.snapshot.as_ref().map(|s| s.root.clone())
        } else {
            None
        };
        let Admission::Started(token) = self.coordinator.begin(purpose) else {
            return;
        };
        if deletion_root.is_some() {
            self.invalidate_forecast();
        }
        let cancel = Cancellation::default();
        let child = cancel.clone();
        let tx = self.tx.clone();
        let failure_tx = tx.clone();
        match thread::Builder::new()
            .name("diskburrow-worker".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    work(child, tx, token)
                }))
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "The background worker stopped unexpectedly"
                    ))
                });
                let _ = failure_tx.send(Event::Finished(token, result));
            }) {
            Ok(join) => {
                self.active = Some(Active {
                    token,
                    cancel,
                    progress: None,
                    join,
                    automatic_request: None,
                    scan_root,
                    deletion_root,
                });
                self.view.busy = true;
                self.view.error = None;
                self.status(status);
            }
            Err(error) => {
                self.coordinator.complete(token, true);
                if let Some(root) = scan_root {
                    self.scheduler.record_manual_outcome(
                        &root,
                        AutomaticScanOutcome::Failed,
                        self.started.elapsed(),
                    );
                }
                self.error(error);
            }
        }
    }
    pub fn command(&mut self, command: Command) {
        let focus_only = matches!(command, Command::MapFocus(_) | Command::MapZoom(_));
        match command {
            Command::SetRoot(root) => {
                if root != self.view.root {
                    self.invalidate_forecast();
                    self.invalidate_review();
                    self.clear_git();
                    self.marks.clear();
                    self.view.root = root;
                    self.history.clear();
                    self.history_selection = None;
                    self.map_changed();
                }
            }
            Command::Scan(fast) => self.start_scan(fast),
            Command::Cancel => self.cancel(),
            Command::Pause => {
                self.view.settings.paused = !self.view.settings.paused;
                self.view.paused = self.view.settings.paused;
                let _ = self.scheduler.apply_settings(self.view.settings.clone());
                if let Some(bridge) = &self.bridge {
                    bridge.update(&self.view.settings.language, self.view.paused);
                }
                self.status(if self.view.paused {
                    "Status.Paused"
                } else {
                    "Status.Ready"
                });
            }
            Command::Setting(setting, value) => self.setting(setting, value),
            Command::SaveSettings => self.save_settings(),
            Command::Mark(path) => {
                let Some(path) = normalize_local_path(&path) else {
                    return;
                };
                if !self.view.busy
                    && let Some(live) = &self.live
                    && let Some(index) = Self::find_map_path(live, &path)
                    && live.entries[index].attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) == 0
                {
                    match crate::mark_selection::toggle(&mut self.marks, &path) {
                        crate::mark_selection::MarkChange::Added { .. }
                        | crate::mark_selection::MarkChange::Removed => {
                            self.invalidate_review();
                            self.invalidate_forecast();
                            self.map_changed();
                        }
                        crate::mark_selection::MarkChange::Covered { .. }
                        | crate::mark_selection::MarkChange::Invalid => {}
                    }
                }
            }
            Command::PreviewManual => self.preview_manual(),
            Command::ConfirmManual => self.execute(false),
            Command::DismissReview => self.invalidate_review(),
            Command::AnalyzeCleanup => self.analyze_cleanup(),
            Command::ToggleCandidate(id) => {
                if let Ok(id) = Uuid::parse_str(&id)
                    && self
                        .cleanup_plan
                        .as_ref()
                        .is_some_and(|p| p.candidates.iter().any(|c| c.id == id))
                {
                    self.invalidate_review();
                    if !self.cleanup_selection.remove(&id) {
                        self.cleanup_selection.insert(id);
                    }
                }
            }
            Command::CleanupFilter(filter) => self.cleanup_filter = filter,
            Command::CleanupCategory(category) => self.cleanup_category = category,
            Command::ExcludeCandidate(id) => {
                if let Some(candidate) = Uuid::parse_str(&id)
                    .ok()
                    .and_then(|id| {
                        self.cleanup_plan
                            .as_ref()?
                            .candidates
                            .iter()
                            .find(|c| c.id == id)
                    })
                    .cloned()
                {
                    self.invalidate_review();
                    self.view.settings.excluded_paths.push(candidate.file.path);
                    self.cleanup_plan = None;
                    self.cleanup_selection.clear();
                }
            }
            Command::ExcludeCategory => {
                if let Some(plan) = &self.cleanup_plan {
                    let roots: Vec<_> = plan
                        .candidates
                        .iter()
                        .filter(|c| c.rule.rule_id == self.cleanup_category)
                        .map(|c| c.rule.path.clone())
                        .collect();
                    self.view.settings.excluded_paths.extend(roots);
                }
                self.invalidate_review();
                self.view.settings.excluded_paths.sort();
                self.view.settings.excluded_paths.dedup();
                self.cleanup_plan = None;
                self.cleanup_selection.clear();
            }
            Command::PreviewCleanup => self.preview_cleanup(),
            Command::ConfirmCleanup => self.execute(true),
            Command::SelectHistory(id) => self.history_selection = Uuid::parse_str(&id).ok(),
            Command::Export(destination) => self.export(destination),
            Command::ExportSelection(destination) => self.export_selection(destination),
            Command::ClearMarks => {
                if !self.marks.is_empty() {
                    self.marks.clear();
                    self.invalidate_review();
                    self.invalidate_forecast();
                    self.map_changed();
                }
            }
            Command::MapColor(color) => {
                if color <= 1 {
                    self.view.map_color = color;
                    self.map_changed();
                }
            }
            Command::MapWiden(root) => {
                if let Some(live) = &self.live
                    && let Some(root) = normalize_local_path(&root)
                    && !equals_path(&root, &live.root)
                    && is_within(&live.root, &root)
                    && !self.view.busy
                {
                    let focus = live.root.clone();
                    self.start_scan_root(false, root, Some(focus));
                }
            }
            Command::MapNavigate(index) => {
                if let Some(live) = &self.live
                    && live.visible(index, self.view.settings.show_hidden)
                {
                    self.nav.navigate(live, index);
                }
                self.select_git();
                self.map_changed();
            }
            Command::MapBack => {
                self.nav.back();
                self.ensure_visible_navigation();
                self.select_git();
                self.map_changed();
            }
            Command::MapForward => {
                self.nav.forward();
                self.ensure_visible_navigation();
                self.select_git();
                self.map_changed();
            }
            Command::MapUp => {
                if let Some(live) = &self.live {
                    self.nav.up(live);
                }
                self.select_git();
                self.map_changed();
            }
            Command::MapRoot => {
                if let Some(live) = &self.live {
                    self.nav.root(live);
                }
                self.select_git();
                self.map_changed();
            }
            Command::MapFocus(index) => {
                if let Some(live) = &self.live
                    && live.visible(index, self.view.settings.show_hidden)
                {
                    self.nav.focus(live, index);
                }
                self.select_git();
            }
            Command::MapDismissFocus => {
                self.nav.focused = None;
                self.clear_git();
            }
            Command::MapMark(index) => {
                if let Some(live) = &self.live
                    && index < live.entries.len()
                {
                    let path = live.path(index);
                    self.command(Command::Mark(path));
                }
            }
            Command::MapSearch(query) => {
                self.view.map_query = query;
                self.map_changed();
            }
            Command::MapIsolate(isolate) => {
                self.view.map_isolate = isolate;
                self.map_changed();
            }
            Command::MapGlobal(global) => {
                self.view.map_global = global;
                self.map_changed();
            }
            Command::MapMetric(metric) => {
                if metric <= 2 {
                    self.view.map_metric = metric;
                    if metric <= 1 {
                        self.view.map_size_metric = metric;
                    }
                    self.map_changed();
                }
            }
            Command::MapSizeMetric(metric) => {
                if metric <= 1 {
                    self.view.map_size_metric = metric;
                    if self.view.map_metric <= 1 {
                        self.view.map_metric = metric;
                    }
                    self.map_changed();
                }
            }
            Command::MapZoom(zoom) => {
                if zoom.is_finite() {
                    self.view.map_zoom = zoom.clamp(0.25, 16.0);
                }
            }
            Command::Open(path) => {
                if let Some(path) = normalize_local_path(&path) {
                    platform::open_path(&path);
                }
            }
            Command::Maintenance(index) => platform::open_maintenance(index),
            Command::Exit => {
                self.exiting = true;
                self.cancel();
            }
        }
        if focus_only {
            self.rebuild_focus();
        } else {
            self.rebuild();
        }
    }
    fn setting(&mut self, setting: Setting, value: String) {
        let key = match setting {
            Setting::Interval => "interval",
            Setting::LowSpaceGb => "low",
            Setting::GrowthGb => "growth",
            Setting::Language => "language",
            Setting::Theme => "theme",
            Setting::UiScale => "ui-scale",
            Setting::MapDepth => "map-depth",
            Setting::ShowHidden => "show-hidden",
            Setting::SidebarWidth => "sidebar-width",
            Setting::Battery => "battery",
            Setting::Autostart => "autostart",
            Setting::Exclusions => "exclusions",
            Setting::CustomTemp => "custom",
        };
        let result = (|| -> Result<()> {
            match setting {
                Setting::Interval => {
                    let n = value.parse::<i32>()?;
                    ensure!([1, 6, 12, 24].contains(&n), "Unsupported scan interval");
                    self.view.settings.interval_hours = n;
                }
                Setting::LowSpaceGb => {
                    self.view.settings.low_space_bytes =
                        parse_threshold(&value).map_err(anyhow::Error::msg)?
                }
                Setting::GrowthGb => {
                    let n = parse_threshold(&value).map_err(anyhow::Error::msg)?;
                    ensure!(n > 0, "Growth threshold must be positive");
                    self.view.settings.growth_bytes = n;
                }
                Setting::Language => {
                    ensure!(
                        ["ru", "en"].contains(&value.as_str()),
                        "Unsupported language"
                    );
                    self.view.settings.language = value;
                    if !self.view.scan_reuse_notice.is_empty() {
                        self.view.scan_reuse_notice = self.label("Map.Cached");
                    }
                    self.map_changed();
                    self.view.status = self.label(&self.status_key);
                }
                Setting::Theme => {
                    ensure!(
                        ["light", "dark", "system"].contains(&value.as_str()),
                        "Unsupported theme"
                    );
                    self.view.settings.theme = value;
                }
                Setting::UiScale => {
                    let n = value.parse::<u16>()?;
                    ensure!(
                        [75, 90, 100, 110, 125, 150].contains(&n),
                        "Unsupported UI scale"
                    );
                    self.view.settings.ui_scale_percent = n;
                }
                Setting::MapDepth => {
                    let n = value.parse::<u8>()?;
                    ensure!((1..=6).contains(&n), "Unsupported map depth");
                    self.view.settings.map_depth = n;
                    self.map_changed();
                }
                Setting::ShowHidden => {
                    self.view.settings.show_hidden = value.parse::<bool>()?;
                    self.ensure_visible_navigation();
                    self.select_git();
                    self.map_changed();
                }
                Setting::SidebarWidth => {
                    let n = value.parse::<u16>()?;
                    ensure!((180..=420).contains(&n), "Unsupported sidebar width");
                    self.view.settings.sidebar_width = n;
                }
                Setting::Battery => self.view.settings.allow_on_battery = value.parse::<bool>()?,
                Setting::Autostart => self.view.settings.autostart = value.parse::<bool>()?,
                Setting::Exclusions => {
                    let mut paths = Vec::new();
                    for line in value.lines().map(str::trim).filter(|p| !p.is_empty()) {
                        paths.push(normalize_local_path(line).ok_or_else(|| {
                            anyhow::anyhow!("Every exclusion must be an absolute local path")
                        })?);
                    }
                    paths.sort();
                    paths.dedup();
                    self.view.settings.excluded_paths = paths;
                    self.invalidate_review();
                    self.cleanup_plan = None;
                    self.cleanup_selection.clear();
                }
                Setting::CustomTemp => {
                    self.view.settings.approved_custom_temp_path = if value.trim().is_empty() {
                        None
                    } else {
                        Some(normalize_local_path(value.trim()).ok_or_else(|| {
                            anyhow::anyhow!("Custom TEMP must be an absolute local path")
                        })?)
                    };
                    self.invalidate_review();
                    self.cleanup_plan = None;
                    self.cleanup_selection.clear();
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.settings_errors.remove(key);
                if self.settings_errors.is_empty() {
                    self.view.error = None;
                }
            }
            Err(error) => {
                self.settings_errors.insert(key, error.to_string());
                self.view.error = Some(format!("{} {error}", self.label("Settings.Invalid")));
            }
        }
        if let Some(bridge) = &self.bridge {
            bridge.update(&self.view.settings.language, self.view.settings.paused);
        }
    }
    fn save_settings(&mut self) {
        if !self.settings_errors.is_empty() {
            self.error(self.label("Settings.Invalid"));
            return;
        }
        let settings = self.view.settings.clone();
        if let Err(error) = settings.validate() {
            self.error(error);
            return;
        }
        if settings
            .approved_custom_temp_path
            .as_ref()
            .is_some_and(|p| self.service.approve_custom_temp_root(p).is_none())
        {
            self.error(self.label("Settings.TempRejected"));
            return;
        }
        let data = self.data_dir.clone();
        let integrations = self.bridge.is_some();
        let executable = std::env::current_exe();
        self.spawn(Purpose::Save, "Status.Saving", move |cancel, _, _| {
            ensure!(!cancel.is_cancelled(), "Cancelled");
            if integrations {
                platform::AutostartRegistration::new(&executable?)?.commit_with(
                    &platform::WindowsRunRegistry,
                    settings.autostart,
                    || SettingsStore::new(data).save(&settings),
                )?;
            } else {
                SettingsStore::new(data).save(&settings)?;
            }
            Ok(Work::SettingsSaved(settings))
        });
    }
    fn start_scan(&mut self, fast: bool) {
        let Some(root) = normalize_local_path(&self.view.root) else {
            self.error("Select an absolute local directory");
            return;
        };
        if fast && root.len() != 3 {
            self.error(self.label("FastScan.Hint"));
            return;
        }
        self.start_scan_root(fast, root, None);
    }
    fn start_scan_root(&mut self, fast: bool, root: String, focus_path: Option<String>) {
        self.invalidate_forecast();
        let cache = if focus_path.is_some() {
            self.scan_cache.clone()
        } else {
            None
        };
        self.invalidate_review();
        self.clear_git();
        let started = Utc::now();
        let data = self.data_dir.clone();
        let scan_threads = self.scan_threads;
        self.spawn(
            Purpose::Scan {
                root: root.clone(),
                fast,
            },
            "Status.Scanning",
            move |cancel, tx, token| {
                ensure!(
                    platform::is_local_path(&root),
                    "Only a confirmed local directory can be scanned"
                );
                ensure!(!cancel.is_cancelled(), "Cancelled");
                let mut next_cache = None;
                let mut reused = false;
                let index = if fast {
                    let progress = Arc::new(CoreProgress::default());
                    let _ = tx.send(Event::Progress(token, progress.clone()));
                    crate::helper::scan(&root, cancel.clone(), progress)?
                } else {
                    let before = crate::scan_cache::observe(&root);
                    let known = cache.as_ref().and_then(|cache| cache.known_for(&root));
                    reused = known.is_some();
                    let mut options = ScanOptions::default();
                    if let Some(threads) = scan_threads {
                        options.threads = threads;
                    }
                    if reused && let Some(cache) = &cache {
                        options.maximum_entries =
                            options.maximum_entries.saturating_sub(cache.entries);
                        options.maximum_resident_bytes = options
                            .maximum_resident_bytes
                            .saturating_sub(cache.reserved_bytes * 2);
                    }
                    let handle = ScanHandle::spawn_with(PathBuf::from(&root), options, known);
                    let _ = tx.send(Event::Progress(token, handle.progress.clone()));
                    let tree = loop {
                        if cancel.is_cancelled() {
                            handle.cancel();
                        }
                        if let Some(result) = handle.poll() {
                            break result?;
                        }
                        thread::sleep(Duration::from_millis(60));
                    };
                    ensure!(!cancel.is_cancelled(), "Cancelled");
                    if reused {
                        ensure!(
                            cache.as_ref().is_some_and(|cache| cache.still_valid()),
                            "Cached directory changed during traversal; previous results retained"
                        );
                    }
                    next_cache = crate::scan_cache::ScanCache::capture(&root, &tree, before);
                    LiveIndex::from_tree(root, tree)?
                };
                ensure!(!cancel.is_cancelled(), "Cancelled");
                let mut snapshot = index.snapshot(started, Utc::now());
                enrich_snapshot(&mut snapshot, &cancel);
                let snapshot = Arc::new(snapshot);
                let mut store = SqliteHistoryStore::new(StorageBudget::new(data));
                let history = store.load_recent(&snapshot.root, 30);
                let history_error = store.last_user_message;
                let analysis = Analysis {
                    classifications: index.classifications(),
                    recommendations: index.recommendations(Utc::now().timestamp(), 100),
                };
                ensure!(!cancel.is_cancelled(), "Cancelled");
                Ok(Work::Scan {
                    index: Arc::new(index),
                    snapshot,
                    history,
                    history_error,
                    cache: next_cache,
                    reused,
                    focus_path,
                    analysis,
                })
            },
        );
    }
    fn start_automatic_scan(&mut self, root: String, request_id: u64) {
        // A scheduled reservation belongs only to the newly admitted operation.
        // Never attach it to an existing manual worker or a superseded token.
        let previous = self.active.as_ref().map(|a| a.token);
        self.view.root = root.clone();
        self.start_scan(false);
        if let Some(active) = &mut self.active
            && previous != Some(active.token)
        {
            active.automatic_request = Some((request_id, root));
        } else {
            self.scheduler.finish_automatic(
                request_id,
                &root,
                AutomaticScanOutcome::Failed,
                self.started.elapsed(),
            );
        }
    }
    fn preview_manual(&mut self) {
        if self.marks.is_empty() {
            return;
        }
        let mut paths: Vec<_> = self.marks.iter().cloned().collect();
        paths.sort();
        let service = self.service.clone();
        self.view.review = None;
        let root = self.snapshot.as_ref().map(|snapshot| snapshot.root.clone());
        self.spawn(Purpose::Review, "Status.Analyzing", move |cancel, _, _| {
            let plan = service.preview_manual(&paths, &cancel)?;
            let projection = root
                .as_deref()
                .and_then(|root| diskburrow_windows::inspect(root).ok())
                .and_then(|file| file.identity)
                .map(|identity| project_manual_reclaim(&plan, identity.volume));
            let mut git = Vec::new();
            for path in plan.roots.iter().take(8) {
                ensure!(!cancel.is_cancelled(), "Cancelled");
                git.push((path.clone(), git_inspection::inspect(path, &cancel)));
            }
            ensure!(!cancel.is_cancelled(), "Cancelled");
            let git_limited = plan.roots.len() > 8;
            Ok(Work::ManualPreview {
                plan,
                projection,
                git,
                git_limited,
            })
        });
    }
    fn analyze_cleanup(&mut self) {
        let service = self.service.clone();
        let exclusions = self.view.settings.excluded_paths.clone();
        let approved: Vec<_> = self
            .view
            .settings
            .approved_custom_temp_path
            .iter()
            .filter(|p| service.approve_custom_temp_root(p).is_some())
            .cloned()
            .collect();
        self.view.review = None;
        self.spawn(Purpose::Review, "Status.Analyzing", move |cancel, _, _| {
            Ok(Work::CleanupPreview(service.preview_cleanup(
                &exclusions,
                &approved,
                &cancel,
            )?))
        });
    }
    fn preview_cleanup(&mut self) {
        if self.view.busy {
            return;
        }
        let Some(plan) = &self.cleanup_plan else {
            return;
        };
        if self.cleanup_selection.is_empty() {
            return;
        }
        self.reviewed_cleanup = self.cleanup_selection.clone();
        let rows: Vec<_> = plan
            .candidates
            .iter()
            .filter(|c| self.reviewed_cleanup.contains(&c.id))
            .take(ROW_LIMIT)
            .map(|c| Row {
                key: c.id.to_string(),
                path: c.file.path.clone(),
                cells: vec![
                    gb(c.file.logical_bytes, &self.view.settings.language),
                    self.label(&c.rule.rule_id),
                    self.label(&c.reason_key),
                ],
                ..Default::default()
            })
            .collect();
        self.view.review = Some(Review {
            id: plan.id.to_string(),
            title: self.label("Nav.Cleanup"),
            summary: format!(
                "{}: {}",
                self.label("Cleanup.Selected"),
                self.reviewed_cleanup.len()
            ),
            warnings: vec![self.label("Manual.PermanentWarning")],
            rows,
            can_confirm: true,
            cleanup: true,
        });
    }
    fn execute(&mut self, cleanup: bool) {
        self.scan_cache = None;
        if self.view.busy {
            return;
        }
        let Some(review) = self.view.review.take() else {
            return;
        };
        if review.cleanup != cleanup || !review.can_confirm {
            self.view.review = Some(review);
            return;
        }
        let Ok(id) = Uuid::parse_str(&review.id) else {
            return;
        };
        let service = self.service.clone();
        let selected = self.reviewed_cleanup.clone();
        self.invalidate_forecast();
        self.clear_git();
        let mut observations = Vec::new();
        if let Some(job) = self.forecast_job.take() {
            job.cancel.cancel();
            observations.push(job.join);
        }
        if let Some(job) = self.git_job.take() {
            job.cancel.cancel();
            observations.push(job.join);
        }
        let barrier = ObservationBarrier(observations);
        self.manual_plan = None;
        self.reviewed_cleanup.clear();
        self.spawn(Purpose::Cleanup, "Status.Cleaning", move |cancel, _, _| {
            barrier.wait();
            ensure!(!cancel.is_cancelled(), "Cancelled");
            Ok(Work::Deleted(if cleanup {
                service.execute_cleanup(id, &selected, true, &cancel)?
            } else {
                service.execute_manual(id, true, &cancel)?
            }))
        });
    }
    fn export(&mut self, destination: String) {
        let Some(snapshot) = self.snapshot.clone() else {
            return;
        };
        if !PathBuf::from(&destination).is_absolute() {
            self.error("An absolute report destination is required");
            return;
        }
        self.spawn(Purpose::Export, "Status.Exporting", move |cancel, _, _| {
            let destination = PathBuf::from(destination);
            let directory = destination
                .parent()
                .ok_or_else(|| anyhow::anyhow!("Report directory is missing"))?;
            let temporary = directory.join(format!(".diskburrow-report-{}.tmp", Uuid::new_v4()));
            let result = (|| {
                ensure!(!cancel.is_cancelled(), "Cancelled");
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                serde_json::to_writer_pretty(&mut file, snapshot.as_ref())?;
                file.flush()?;
                file.sync_all()?;
                drop(file);
                ensure!(!cancel.is_cancelled(), "Cancelled");
                fs::hard_link(&temporary, &destination)?;
                fs::remove_file(&temporary)?;
                Ok(Work::Exported)
            })();
            if temporary.exists() {
                let _ = fs::remove_file(&temporary);
            }
            result
        });
    }
    pub fn selected_export(&self) -> Option<crate::selection_export::SelectedExport> {
        self.live.as_ref().map(|live| {
            crate::selection_export::render_selected(
                live,
                &self.marks,
                &self.view.settings.language,
            )
        })
    }
    fn export_selection(&mut self, destination: String) {
        let Some(export) = self.selected_export().filter(|export| export.count > 0) else {
            return;
        };
        if !std::path::Path::new(&destination).is_absolute() {
            self.error("An absolute list destination is required");
            return;
        }
        self.spawn(Purpose::Export, "Status.Exporting", move |cancel, _, _| {
            let destination = PathBuf::from(destination);
            let parent = destination
                .parent()
                .ok_or_else(|| anyhow::anyhow!("A list destination directory is required"))?;
            let temporary = parent.join(format!(".diskburrow-selection-{}.tmp", Uuid::new_v4()));
            let result = (|| {
                ensure!(!cancel.is_cancelled(), "Cancelled");
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(export.list.as_bytes())?;
                file.flush()?;
                file.sync_all()?;
                drop(file);
                ensure!(!cancel.is_cancelled(), "Cancelled");
                // Atomic non-overwriting publication; failure cleanup never removes the user destination.
                use std::os::windows::ffi::OsStrExt as _;
                let source = temporary
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                let target = destination
                    .as_os_str()
                    .encode_wide()
                    .chain(Some(0))
                    .collect::<Vec<_>>();
                ensure!(
                    !source[..source.len() - 1].contains(&0)
                        && !target[..target.len() - 1].contains(&0),
                    "Invalid list path"
                );
                // No REPLACE_EXISTING or COPY_ALLOWED: publish within the same directory,
                // including filesystems without hardlinks, and preserve any existing target.
                let moved = unsafe {
                    windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                        source.as_ptr(),
                        target.as_ptr(),
                        windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
                    )
                };
                if moved == 0 {
                    return Err(std::io::Error::last_os_error().into());
                }
                Ok(Work::Exported)
            })();
            if temporary.exists() {
                let _ = fs::remove_file(&temporary);
            }
            result
        });
    }
    fn projection(&self) -> MapProjection {
        MapProjection {
            depth: self.view.settings.map_depth as u32,
            show_hidden: self.view.settings.show_hidden,
        }
    }
    fn find_map_path(live: &LiveIndex, path: &str) -> Option<usize> {
        if equals_path(&live.root, path) {
            return Some(0);
        }
        if !is_within(path, &live.root) {
            return None;
        }
        let components = live.root.trim_end_matches('\\').split('\\').count();
        let mut current = 0;
        for name in path.trim_end_matches('\\').split('\\').skip(components) {
            current = *live.entries[current]
                .children
                .iter()
                .find(|i| equals_path(&live.entries[**i].name, name))?;
        }
        Some(current)
    }
    pub fn map_path(&self, index: usize) -> Option<String> {
        self.live
            .as_ref()
            .filter(|live| index < live.entries.len())
            .map(|live| live.path(index))
    }
    pub fn siblings(&self, target: usize) -> Vec<Row> {
        let Some(live) = &self.live else {
            return vec![];
        };
        let Some(parent) = live.entries.get(target).and_then(|entry| entry.parent) else {
            return vec![];
        };
        let metric = self.view.map_metric;
        let projected = (!self.view.settings.show_hidden).then(|| {
            live.projected_weights_with_projection(
                parent,
                match metric {
                    1 => MapMetric::Logical,
                    2 => MapMetric::Files,
                    _ => MapMetric::Allocated,
                },
                "",
                false,
                false,
                self.projection(),
            )
        });
        let weight = |i: usize| {
            projected.as_ref().map_or_else(
                || match metric {
                    1 => live.entries[i].logical,
                    2 => live.entries[i].files,
                    _ => live.entries[i].allocated.unwrap_or(0),
                },
                |weights| weights[i],
            )
        };
        let mut siblings = live.entries[parent]
            .children
            .iter()
            .copied()
            .filter(|i| live.visible(*i, self.view.settings.show_hidden))
            .collect::<Vec<_>>();
        siblings.sort_by_key(|i| (std::cmp::Reverse(weight(*i)), *i));
        let total = weight(parent).max(1) as f64;
        siblings
            .into_iter()
            .take(ROW_LIMIT)
            .map(|i| Row {
                key: i.to_string(),
                path: live.path(i),
                selected: false,
                selectable: false,
                cells: vec![format!(
                    "{:.1}% · {}",
                    100. * weight(i) as f64 / total,
                    if metric == 2 {
                        weight(i).to_string()
                    } else {
                        gb(weight(i), &self.view.settings.language)
                    }
                )],
            })
            .collect()
    }
    fn refresh_history(&mut self) {
        let data = self.data_dir.clone();
        let root = self.view.root.clone();
        self.spawn(Purpose::Review, "Status.Ready", move |_, _, _| {
            let mut store = SqliteHistoryStore::new(StorageBudget::new(data));
            let history = store.load_recent(&root, 30);
            if let Some(message) = store.last_user_message {
                bail!("{message}");
            }
            Ok(Work::History { root, history })
        });
    }
    pub fn poll(&mut self) -> bool {
        let old_status = self.view.status.clone();
        let old_error = self.view.error.clone();
        let mut dirty = false;
        let launches = self
            .bridge
            .as_ref()
            .map(|bridge| {
                std::iter::from_fn(|| bridge.try_recv_launch())
                    .take(16)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for payload in launches {
            match serde_json::from_str::<crate::cli::LaunchOverrides>(&payload)
                .map_err(anyhow::Error::from)
                .and_then(|launch| self.queue_launch(launch))
            {
                Ok(()) => dirty = true,
                Err(error) => {
                    self.error(error);
                    dirty = true;
                }
            }
        }
        let events: Vec<_> = self
            .platform_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                PlatformEvent::Show => self.show_requested = true,
                PlatformEvent::Scan => self.command(Command::Scan(false)),
                PlatformEvent::Pause => self.command(Command::Pause),
                PlatformEvent::Exit => self.command(Command::Exit),
                PlatformEvent::Open(path) => {
                    self.show_requested = true;
                    if normalize_local_path(&path).is_some() {
                        self.view.focused_path = path;
                    }
                }
            }
        }
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::ForecastFinished {
                    generation,
                    scan_id,
                    observation,
                } => {
                    if self
                        .forecast_job
                        .as_ref()
                        .is_some_and(|job| job.generation == generation && job.scan_id == scan_id)
                    {
                        let job = self.forecast_job.take().unwrap();
                        let publish = !job.cancel.is_cancelled()
                            && generation == self.forecast_generation
                            && self.scan_id() == Some(scan_id)
                            && !self.exiting;
                        let _ = job.join.join();
                        if publish {
                            self.forecast_result = observation;
                            dirty = true;
                        }
                    }
                }
                Event::GitFinished {
                    generation,
                    path,
                    inspection,
                } => {
                    let publish = if self
                        .git_job
                        .as_ref()
                        .is_some_and(|job| job.generation == generation)
                    {
                        let job = self.git_job.take().unwrap();
                        let publish = !job.cancel.is_cancelled();
                        let _ = job.join.join();
                        publish
                    } else {
                        false
                    };
                    if publish && generation == self.git_generation && !self.exiting {
                        self.git_result = Some((path, inspection));
                        dirty = true;
                    }
                    self.start_git_job();
                }
                Event::Progress(token, progress) => {
                    if let Some(active) = &mut self.active
                        && active.token == token
                    {
                        if active.cancel.is_cancelled() {
                            progress.cancel();
                        }
                        active.progress = Some(progress);
                    }
                }
                Event::Finished(token, result) => {
                    if token == 0 {
                        if let Ok(Work::Journal(Some(message))) = result {
                            self.view.error = Some(message);
                        }
                        continue;
                    }
                    if self.active.as_ref().is_none_or(|a| a.token != token) {
                        continue;
                    }
                    dirty = true;
                    let active = self.active.take().unwrap();
                    let cancelled = active.cancel.is_cancelled();
                    let _ = active.join.join();
                    let mut result = result;
                    if let Some(root) = &active.scan_root {
                        if matches!(&result, Ok(Work::Scan { snapshot, .. })
                            if path_key(&snapshot.root) != path_key(root))
                        {
                            result = Err(anyhow::anyhow!("Scan returned another root"));
                        }
                        let outcome = if cancelled {
                            AutomaticScanOutcome::Cancelled
                        } else if matches!(result, Ok(Work::Scan { .. })) {
                            AutomaticScanOutcome::Completed
                        } else {
                            AutomaticScanOutcome::Failed
                        };
                        if let Some((request_id, expected_root)) = &active.automatic_request {
                            self.scheduler.finish_automatic(
                                *request_id,
                                expected_root,
                                outcome,
                                self.started.elapsed(),
                            );
                        } else {
                            self.scheduler.record_manual_outcome(
                                root,
                                outcome,
                                self.started.elapsed(),
                            );
                        }
                    }
                    let publish = self.coordinator.complete(token, cancelled);
                    self.view.busy = false;
                    match result {
                        Ok(Work::Deleted(report)) => {
                            self.last_report = Some(report.clone());
                            self.cleanup_plan = None;
                            self.cleanup_selection.clear();
                            self.marks.clear();
                            self.map_changed();
                            self.journal_cleanup(report);
                            self.status("Manual.Stale");
                            if let Some(root) = active.deletion_root
                                && !self.exiting
                            {
                                self.view.root = root;
                                self.start_scan(false);
                                self.refreshing_deletion = self.view.busy;
                            }
                        }
                        Ok(work @ Work::SettingsSaved(_)) => self.accept(work),
                        Ok(work) if publish => self.accept(work),
                        Ok(_) => self.status("Status.Cancelled"),
                        Err(_) if cancelled => self.status("Status.Cancelled"),
                        Err(error) => {
                            if self.refreshing_deletion && active.scan_root.is_some() {
                                self.error(format!(
                                    "{}: {error}",
                                    self.label("Manual.RefreshFailed")
                                ));
                            } else {
                                self.error(error);
                            }
                        }
                    }
                    if active.scan_root.is_some() {
                        self.refreshing_deletion = false;
                    }
                }
            }
        }
        if let Some(active) = &self.active
            && let Some(progress) = &active.progress
        {
            let p = progress.snapshot();
            self.view.status = format!(
                "{} · {} {} · {} {}",
                self.label(&self.status_key),
                p.files,
                self.label("Manual.Files"),
                p.dirs,
                self.label("Manual.Folders")
            );
        }
        self.journal_workers.retain(|join| !join.is_finished());
        if !self.view.busy
            && !self.exiting
            && let Some(launch) = self.pending_launch.take()
        {
            if let Err(error) = self.apply_launch(launch) {
                self.error(error);
            }
            dirty = true;
        }
        let had_forecast = self.forecast_job.is_some();
        self.start_forecast();
        dirty |= had_forecast != self.forecast_job.is_some();
        if self.monitoring && !self.exiting && Instant::now() >= self.next_monitor {
            self.next_monitor = Instant::now() + Duration::from_secs(1);
            let root = platform::system_root();
            let actions = self.scheduler.tick(
                Utc::now(),
                self.started.elapsed(),
                platform::on_battery(),
                self.coordinator.is_busy(),
                platform::is_local_path(&root),
            );
            if actions.check_free_space {
                self.check_alerts(None, None);
                self.rebuild();
                dirty = true;
            }
            if let Some(root) = actions.request_full_scan
                && let Some(request_id) = actions.automatic_request_id
            {
                self.start_automatic_scan(root, request_id);
                dirty = true;
            }
        }
        if let Some(bridge) = &self.bridge {
            bridge.set_busy(self.view.busy);
        }
        if dirty {
            self.rebuild();
        }
        dirty
            || self.view.status != old_status
            || self.view.error != old_error
            || self.show_requested
            || self.should_exit()
    }
    fn accept(&mut self, work: Work) {
        match work {
            Work::Scan {
                index,
                snapshot,
                history,
                history_error,
                cache,
                reused,
                focus_path,
                analysis,
            } => {
                let previous = history.first().cloned();
                self.history = history;
                self.history_selection = None;
                self.view.root = snapshot.root.clone();
                self.live = Some(index);
                self.scan_cache = cache;
                self.view.scan_reuse_notice = if reused {
                    self.label("Map.Cached")
                } else {
                    String::new()
                };
                self.analysis = analysis;
                self.invalidate_forecast();
                self.clear_git();
                self.snapshot = Some(snapshot.clone());
                self.nav = MapNavigation::new();
                if let Some(path) = focus_path {
                    if let Some(live) = &self.live
                        && let Some(target) = Self::find_map_path(live, &path)
                    {
                        self.nav.navigate(live, target);
                    }
                } else {
                    self.marks.clear();
                }
                self.map_changed();
                self.scheduler.record_completed(&snapshot, Utc::now());
                self.status("Status.Ready");
                self.check_alerts(previous.as_ref(), Some(&snapshot));
                let data = self.data_dir.clone();
                let tx = self.tx.clone();
                let saved_snapshot = snapshot.clone();
                self.journal_workers.push(thread::spawn(move || {
                    let mut store = SqliteHistoryStore::new(StorageBudget::new(data));
                    let outcome = store.save(&saved_snapshot);
                    let _ = tx.send(Event::Finished(0, Ok(Work::Journal(outcome.user_message))));
                }));
                self.history.insert(0, (*snapshot).clone());
                self.history.truncate(30);
                if let Some(message) = history_error {
                    self.view.error = Some(message);
                }
            }
            Work::CleanupPreview(plan) => {
                self.cleanup_selection.clear();
                self.cleanup_plan = Some(plan);
                self.last_report = None;
                self.status("Status.Ready");
            }
            Work::ManualPreview {
                plan,
                projection,
                git,
                git_limited,
            } => {
                let mut summary = format!(
                    "{}: {} · {}: {} · {}",
                    self.label("Manual.Files"),
                    plan.file_count(),
                    self.label("Manual.Folders"),
                    plan.directory_count(),
                    gb(plan.estimated_data_bytes(), &self.view.settings.language)
                );
                summary.push_str(&format!(
                    " · {}: {}",
                    self.label("Reclaim.Known"),
                    projection.as_ref().map_or_else(
                        || self.label("Unknown"),
                        |projection| gb(
                            projection.known_reclaim_bytes,
                            &self.view.settings.language
                        )
                    )
                ));
                let mut warnings: Vec<String> = plan
                    .warnings
                    .iter()
                    .map(|w| {
                        format!(
                            "{} {}",
                            w.path.as_deref().unwrap_or(""),
                            self.label(&w.reason_key)
                        )
                    })
                    .collect();
                warnings.push(self.label("Reclaim.Note"));
                if let Some(projection) = projection {
                    warnings.push(format!(
                        "{}: {} / {} / {}",
                        self.label("Reclaim.Excluded"),
                        projection.excluded_hardlink_files,
                        projection.excluded_unknown_files,
                        projection.excluded_foreign_files
                    ));
                }
                if !git.is_empty() {
                    warnings.push(self.label("Git.ReviewNote"));
                }
                for (path, inspection) in git {
                    warnings.push(format!(
                        "{path}: {}",
                        crate::recommendations::git_summary(
                            &inspection,
                            &self.view.settings.language
                        )
                    ));
                }
                if git_limited {
                    warnings.push(self.label("Git.ReviewLimit"));
                }
                let rows = plan
                    .entries
                    .iter()
                    .take(ROW_LIMIT)
                    .map(|e| Row {
                        key: e.id.to_string(),
                        path: e.file.path.clone(),
                        cells: vec![gb(e.file.logical_bytes, &self.view.settings.language)],
                        ..Default::default()
                    })
                    .collect();
                self.view.review = Some(Review {
                    id: plan.id.to_string(),
                    title: self.label("Manual.Review"),
                    summary,
                    warnings,
                    rows,
                    can_confirm: plan.can_execute(),
                    cleanup: false,
                });
                self.manual_plan = Some(plan);
                self.status("Manual.Ready");
            }
            Work::SettingsSaved(settings) => {
                self.view.settings = settings.clone();
                self.view.paused = settings.paused;
                let _ = self.scheduler.apply_settings(settings.clone());
                let _ = self.alerts.apply_settings(settings);
                self.status("Status.Saved");
            }
            Work::Exported => self.status("Status.Exported"),
            Work::History { root, history } => {
                if path_key(&root) == path_key(&self.view.root) {
                    self.history = history;
                }
                self.status("Status.Ready");
            }
            Work::Journal(message) => {
                if let Some(message) = message {
                    self.view.error = Some(message);
                }
            }
            Work::Deleted(_) => {}
        }
    }
    fn journal_cleanup(&mut self, report: CleanupReport) {
        let data = self.data_dir.clone();
        let tx = self.tx.clone();
        self.journal_workers.push(thread::spawn(move || {
            let mut store = SqliteHistoryStore::new(StorageBudget::new(data));
            let result = store.append_cleanup(&report);
            let _ = tx.send(Event::Finished(0, Ok(Work::Journal(result.user_message))));
        }));
    }
    fn check_alerts(&mut self, previous: Option<&ScanSnapshot>, current: Option<&ScanSnapshot>) {
        let root = current.map_or_else(platform::system_root, |s| s.root.clone());
        let Ok(space) = platform::volume_space(&root) else {
            return;
        };
        self.volume_observation = Some(VolumeObservation {
            root: root.clone(),
            space: space.clone(),
            observed_utc: Utc::now(),
        });
        if let Ok(alerts) = self
            .alerts
            .evaluate(previous, current, space.free_bytes, Utc::now())
        {
            for alert in alerts {
                if let Some(bridge) = &self.bridge {
                    let key = match alert.kind {
                        AlertKind::LowSpace => "Alert.Low",
                        AlertKind::FolderGrowth => "Alert.Growth",
                    };
                    bridge.notify(
                        "DiskBurrow",
                        &format!(
                            "{}: {} · {}",
                            self.label(key),
                            gb(alert.bytes, &self.view.settings.language),
                            alert.destination_path
                        ),
                        Some(alert.destination_path),
                    );
                }
            }
            let state = self.alerts.state.clone();
            let data = self.data_dir.clone();
            self.journal_workers.push(thread::spawn(move || {
                let _ = SettingsStore::new(data).save_alert_state(&state);
            }));
        }
    }
    fn marked(&self, path: &str) -> bool {
        self.marks.contains(&path.to_lowercase())
    }
    fn covered(&self, path: &str) -> bool {
        crate::mark_selection::covered_ancestor(&self.marks, path).is_some()
    }
    fn rebuild(&mut self) {
        let language = self.view.settings.language.clone();
        self.view.selected_count = self.marks.len();
        self.view.map_marked = self
            .marks
            .iter()
            .take(ROW_LIMIT)
            .map(|key| {
                let path = self
                    .live
                    .as_ref()
                    .and_then(|live| Self::find_map_path(live, key).map(|i| live.path(i)))
                    .unwrap_or_else(|| key.clone());
                Row {
                    key: path.clone(),
                    path,
                    selected: true,
                    selectable: !self.view.busy,
                    cells: vec![],
                }
            })
            .collect();
        self.view.map_marked.sort_by(|a, b| a.path.cmp(&b.path));
        self.view.map_marked_summary = if self.marks.is_empty() {
            self.label("Map.NoMarks")
        } else {
            format!(
                "{}: {} · {}: {}",
                self.label("Map.Marked"),
                self.marks.len(),
                self.label("Map.Displayed"),
                self.view.map_marked.len()
            )
        };
        self.rebuild_forecast();
        self.view.can_manual = !self.view.busy && !self.marks.is_empty();
        self.view.can_cleanup = !self.view.busy && !self.cleanup_selection.is_empty();
        if let Some(snapshot) = &self.snapshot {
            self.view.observed_caches = crate::recommendations::observed(
                &snapshot.directories,
                &self.known_cache_roots.0,
                &self.known_cache_roots.1,
                &language,
            );
            let directory = snapshot.directories.first();
            self.view.overview = vec![
                (self.label("Root"), snapshot.root.clone()),
                (
                    self.label("Overview.Logical"),
                    gb(directory.map_or(0, |d| d.logical_bytes), &language),
                ),
                (
                    self.label("Overview.Allocated"),
                    directory
                        .and_then(|d| d.allocated_bytes)
                        .map_or_else(|| self.label("Unknown"), |n| gb(n, &language)),
                ),
                (
                    self.label("Manual.Files"),
                    self.live
                        .as_ref()
                        .map_or(0, |index| index.entries[0].files)
                        .to_string(),
                ),
                (
                    self.label("Manual.Folders"),
                    snapshot.directories.len().to_string(),
                ),
                (
                    self.label("Overview.Checked"),
                    time(snapshot.completed_utc, &language),
                ),
            ];
            let mut counts: HashMap<i32, usize> = HashMap::new();
            self.view.overview.push((
                self.label("Overview.Coverage"),
                self.label(if directory.is_some_and(|d| d.coverage_complete) {
                    "Yes"
                } else {
                    "No"
                }),
            ));
            if let Some(observation) = &self.volume_observation {
                self.view.overview.extend([
                    (
                        self.label("Overview.Total"),
                        gb(observation.space.total_bytes, &language),
                    ),
                    (
                        self.label("Overview.Free"),
                        gb(observation.space.free_bytes, &language),
                    ),
                    (
                        self.label("Overview.Used"),
                        gb(observation.space.used_bytes(), &language),
                    ),
                ]);
                self.view.volume_stamp = format!(
                    "{} {} · {}",
                    self.label("Overview.CountersObserved"),
                    observation.root,
                    time(observation.observed_utc, &language)
                );
            }
            for issue in &snapshot.issues {
                *counts.entry(issue.kind as i32).or_default() += 1;
            }
            self.view.issue_rows = counts
                .into_iter()
                .map(|(kind, count)| Row {
                    path: self.label(issue_key(kind)),
                    cells: vec![count.to_string()],
                    ..Default::default()
                })
                .collect();
            self.view.issue_rows.sort_by(|a, b| a.path.cmp(&b.path));
            let mut dirs: Vec<_> = snapshot
                .directories
                .iter()
                .filter(|d| self.visible_path(&d.path))
                .collect();
            dirs.sort_by_key(|d| std::cmp::Reverse(d.logical_bytes));
            self.view.folders = dirs
                .into_iter()
                .take(ROW_LIMIT)
                .map(|d| Row {
                    key: d.path.clone(),
                    path: d.path.clone(),
                    selected: self.marked(&d.path) || self.covered(&d.path),
                    selectable: d.path != snapshot.root && !self.covered(&d.path),
                    cells: vec![
                        gb(d.logical_bytes, &language),
                        d.allocated_bytes
                            .map_or_else(|| self.label("Unknown"), |n| gb(n, &language)),
                        self.label(if d.coverage_complete { "Yes" } else { "No" }),
                    ],
                })
                .collect();
            self.view.files = snapshot
                .largest_files
                .iter()
                .filter(|f| self.visible_path(&f.path))
                .map(|f| Row {
                    key: f.path.clone(),
                    path: f.path.clone(),
                    selected: self.marked(&f.path) || self.covered(&f.path),
                    selectable: !self.covered(&f.path)
                        && f.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) == 0,
                    cells: vec![
                        gb(f.logical_bytes, &language),
                        f.allocated_bytes
                            .map_or_else(|| self.label("Unknown"), |n| gb(n, &language)),
                        time(f.modified_utc, &language),
                    ],
                })
                .collect();
        }
        self.view.history = self
            .history
            .iter()
            .map(|s| Row {
                key: s.id.to_string(),
                path: s.root.clone(),
                cells: vec![
                    time(s.completed_utc, &language),
                    gb(
                        s.directories.first().map_or(0, |d| d.logical_bytes),
                        &language,
                    ),
                    self.label(
                        if s.directories.first().is_some_and(|d| d.coverage_complete) {
                            "Yes"
                        } else {
                            "No"
                        },
                    ),
                ],
                ..Default::default()
            })
            .collect();
        self.view.changes.clear();
        self.view.history_heading.clear();
        if let Some(selected) = self
            .history_selection
            .and_then(|id| self.history.iter().position(|s| s.id == id))
            && let Some(previous) = self.history.get(selected + 1)
        {
            let current = &self.history[selected];
            self.view.history_heading = format!(
                "{} → {}",
                time(previous.completed_utc, &language),
                time(current.completed_utc, &language)
            );
            self.view.changes = compare_snapshots(previous, current)
                .into_iter()
                .take(ROW_LIMIT)
                .map(|c| Row {
                    path: c.path,
                    cells: vec![
                        if c.comparable {
                            gb(c.logical_delta_bytes, &language)
                        } else {
                            self.label("Unknown")
                        },
                        self.label(match c.kind {
                            FolderChangeKind::New => "Change.New",
                            FolderChangeKind::Removed => "Change.Removed",
                            FolderChangeKind::Changed => "Change.Changed",
                            FolderChangeKind::Unavailable => "Change.Unavailable",
                        }),
                        self.label(if c.comparable { "Yes" } else { "No" }),
                    ],
                    ..Default::default()
                })
                .collect();
        }
        self.rebuild_cleanup();
        self.view.recommendations = self
            .live
            .as_ref()
            .map(|index| {
                crate::recommendations::insights(
                    index,
                    &self.analysis.classifications,
                    &self.analysis.recommendations,
                    &self.view.settings.language,
                )
            })
            .unwrap_or_default();
        self.rebuild_map();
    }
    fn rebuild_cleanup(&mut self) {
        self.view.cleanup.clear();
        self.view.cleanup_warnings.clear();
        self.view.cleanup_categories = vec!["All".into()];
        self.view.cleanup_category = self.cleanup_category.clone();
        if let Some(plan) = &self.cleanup_plan {
            let filter = self.cleanup_filter.to_lowercase();
            self.view.cleanup = plan
                .candidates
                .iter()
                .filter(|c| {
                    (self.cleanup_category == "All" || c.rule.rule_id == self.cleanup_category)
                        && (filter.is_empty() || c.file.path.to_lowercase().contains(&filter))
                })
                .take(ROW_LIMIT)
                .map(|c| Row {
                    key: c.id.to_string(),
                    path: c.file.path.clone(),
                    selected: self.cleanup_selection.contains(&c.id),
                    selectable: true,
                    cells: vec![
                        gb(c.file.logical_bytes, &self.view.settings.language),
                        self.label(&c.rule.rule_id),
                        self.label(&c.reason_key),
                    ],
                })
                .collect();
            let mut categories: Vec<_> = plan
                .candidates
                .iter()
                .map(|c| c.rule.rule_id.clone())
                .collect();
            categories.sort();
            categories.dedup();
            self.view.cleanup_categories.extend(categories);
            self.view.cleanup_warnings = plan
                .warnings
                .iter()
                .map(|w| Row {
                    path: w.path.clone().unwrap_or_default(),
                    cells: vec![self.label(&w.rule_id), self.label(&w.reason_key)],
                    ..Default::default()
                })
                .collect();
            self.view.cleanup_summary = format!(
                "{}: {} · {}: {} · {}",
                self.label("Cleanup.Count"),
                plan.candidates.len(),
                self.label("Cleanup.Selected"),
                self.cleanup_selection.len(),
                gb(
                    plan.candidates
                        .iter()
                        .filter(|c| self.cleanup_selection.contains(&c.id))
                        .fold(0i64, |n, c| n.saturating_add(c.file.logical_bytes)),
                    &self.view.settings.language
                )
            );
        } else {
            self.view.cleanup_summary = self.label("Cleanup.Eligible");
        }
        if let Some(report) = &self.last_report {
            self.view.cleanup_summary = format!(
                "{}: {} · {}",
                self.label("Reclaim.Actual"),
                if report.free_space_delta_available {
                    gb(report.free_space_delta_bytes, &self.view.settings.language)
                } else {
                    self.label("Unknown")
                },
                self.label("Reclaim.Note")
            );
        }
        self.view.cleanup_results = self
            .last_report
            .as_ref()
            .map(|r| {
                r.items
                    .iter()
                    .take(ROW_LIMIT)
                    .map(|item| Row {
                        key: item.candidate_id.to_string(),
                        path: item
                            .audit
                            .as_ref()
                            .map_or_else(|| item.candidate_id.to_string(), |a| a.path.clone()),
                        cells: vec![
                            self.label(outcome_key(item.outcome)),
                            item.reason_key
                                .as_ref()
                                .map_or_else(String::new, |key| self.label(key)),
                        ],
                        ..Default::default()
                    })
                    .collect()
            })
            .unwrap_or_default();
    }
    fn ensure_visible_navigation(&mut self) {
        let Some(live) = &self.live else {
            return;
        };
        if !live.visible(self.nav.current, self.view.settings.show_hidden) {
            let mut target = self.nav.current;
            while !live.visible(target, self.view.settings.show_hidden) {
                let Some(parent) = live.entries[target].parent else {
                    break;
                };
                target = parent;
            }
            self.nav = MapNavigation::new();
            self.nav.navigate(live, target);
        }
        if self
            .nav
            .focused
            .is_some_and(|i| !live.visible(i, self.view.settings.show_hidden))
        {
            self.nav.focused = live
                .visible(self.nav.current, self.view.settings.show_hidden)
                .then_some(self.nav.current);
        }
    }
    fn visible_path(&self, path: &str) -> bool {
        self.view.settings.show_hidden
            || self.live.as_ref().is_some_and(|live| {
                Self::find_map_path(live, path).is_some_and(|i| live.visible(i, false))
            })
    }
    fn rebuild_map(&mut self) {
        let Some(index) = &self.live else {
            return;
        };
        self.view.map_has_data = true;
        self.view.map_path = index.path(self.nav.current);
        self.view.map_breadcrumbs.clear();
        let mut at = Some(self.nav.current);
        while let Some(i) = at {
            self.view.map_breadcrumbs.push((
                i,
                if i == 0 {
                    index.root.clone()
                } else {
                    index.entries[i].name.to_string()
                },
            ));
            at = index.entries[i].parent;
        }
        self.view.map_breadcrumbs.reverse();
        self.view.map_parents = std::path::Path::new(&index.root)
            .ancestors()
            .skip(1)
            .take(64)
            .filter_map(|path| {
                normalize_local_path(&path.to_string_lossy()).map(|p| (p.clone(), p))
            })
            .collect();
        self.view.map_parents.reverse();
        let weights = index.projected_weights_with_projection(
            self.nav.current,
            match self.view.map_metric {
                1 => MapMetric::Logical,
                2 => MapMetric::Files,
                _ => MapMetric::Allocated,
            },
            &self.view.map_query,
            self.view.map_isolate,
            self.view.map_global,
            self.projection(),
        );
        let full = match self.view.map_metric {
            1 => index.entries[0].logical,
            2 => index.entries[0].files,
            _ => index.entries[0].allocated.unwrap_or(0),
        };
        let metric_key = match self.view.map_metric {
            1 => "Map.Metric.Logical",
            2 => "Map.Metric.Files",
            _ => "Map.Metric.Allocated",
        };
        let amount = |n: i64| {
            if self.view.map_metric == 2 {
                n.to_string()
            } else {
                gb(n, &self.view.settings.language)
            }
        };
        self.view.map_visible_summary = format!(
            "{} ({}): {} · {}: {}",
            self.label("Map.Visible"),
            self.label(metric_key),
            amount(weights[self.nav.current]),
            self.label("Map.FullScan"),
            if self.view.map_metric == 0 && index.entries[0].allocated.is_none() {
                self.label("Unknown")
            } else {
                amount(full)
            }
        );
        let logical_weights = (self.view.map_metric != 1).then(|| {
            index.projected_weights_with_projection(
                self.nav.current,
                MapMetric::Logical,
                &self.view.map_query,
                self.view.map_isolate,
                self.view.map_global,
                self.projection(),
            )
        });
        let matches = index.search_with_projection(
            self.nav.current,
            &self.view.map_query,
            self.view.map_global,
            1000,
            self.projection(),
        );
        self.view.map_matches = matches.total;
        self.view.map_objects = matches
            .indices
            .into_iter()
            .map(|i| {
                let e = &index.entries[i];
                let path = index.path(i);
                Row {
                    key: i.to_string(),
                    path: path.clone(),
                    selected: self.marked(&path),
                    selectable: i != 0 && e.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) == 0,
                    cells: vec![
                        gb(
                            logical_weights
                                .as_ref()
                                .map_or(weights[i], |logical| logical[i]),
                            &self.view.settings.language,
                        ),
                        e.allocated.map_or_else(
                            || self.label("Unknown"),
                            |n| gb(n, &self.view.settings.language),
                        ),
                        e.files.to_string(),
                    ],
                }
            })
            .collect();
        self.rebuild_focus();
    }
    fn rebuild_forecast(&mut self) {
        let language = &self.view.settings.language;
        self.view.map_forecast = if let Some(observation) = &self.forecast_result {
            let projection = &observation.projection;
            let excluded = projection.excluded_hardlink_files
                + projection.excluded_unknown_files
                + projection.excluded_foreign_files;
            let known = gb(projection.known_reclaim_bytes, language);
            let after = if observation.partial || excluded > 0 {
                self.label("Unknown")
            } else {
                gb(
                    observation
                        .space
                        .free_bytes
                        .saturating_add(projection.known_reclaim_bytes)
                        .min(observation.space.total_bytes),
                    language,
                )
            };
            format!(
                "{}: {}\n{}: {}\n{}: {}\n{}: {}\n{} · {}\n{}{}",
                self.label("Overview.Free"),
                gb(observation.space.free_bytes, language),
                self.label("Reclaim.Known"),
                known,
                self.label("Map.FreeAfter"),
                after,
                self.label("Reclaim.Excluded"),
                excluded,
                self.label("Overview.CountersObserved"),
                time(observation.observed_utc, language),
                self.label("Reclaim.Note"),
                if observation.partial {
                    format!("\n{}", self.label("Map.ForecastPartial"))
                } else {
                    String::new()
                }
            )
        } else {
            let free = self
                .volume_observation
                .as_ref()
                .filter(|o| equals_path(&o.root, &self.view.root))
                .map(|o| {
                    format!(
                        "{}: {}\n{} · {}\n",
                        self.label("Overview.Free"),
                        gb(o.space.free_bytes, language),
                        self.label("Overview.CountersObserved"),
                        time(o.observed_utc, language)
                    )
                })
                .unwrap_or_default();
            let key = if self.marks.is_empty() {
                "Map.NoMarks"
            } else if self
                .forecast_job
                .as_ref()
                .is_some_and(|job| job.generation == self.forecast_generation)
            {
                "Map.ForecastPending"
            } else {
                "Map.ForecastUnknown"
            };
            format!("{free}{}", self.label(key))
        };
    }
    fn rebuild_focus(&mut self) {
        self.view.map_covering_parent.clear();
        self.view.focused_path.clear();
        self.view.focused_summary.clear();
        self.view.git_summary.clear();
        let Some(index) = &self.live else {
            return;
        };
        if let Some(focused) = self.nav.focused
            && let Some(entry) = index.entries.get(focused)
        {
            self.view.focused_path = index.path(focused);
            if let Some(parent) =
                crate::mark_selection::covered_ancestor(&self.marks, &self.view.focused_path)
            {
                self.view.map_covering_parent = Self::find_map_path(index, &parent)
                    .map(|i| index.path(i))
                    .unwrap_or(parent);
            }
            self.view.focused_summary = format!(
                "{} · {} · {} {} · {}",
                gb(entry.logical, &self.view.settings.language),
                entry.allocated.map_or_else(
                    || self.label("Unknown"),
                    |n| gb(n, &self.view.settings.language)
                ),
                entry.files,
                self.label("Manual.Files"),
                self.label(if entry.coverage { "Yes" } else { "No" })
            );
            let whole = index.entries[0].logical.max(1) as f64;
            self.view.focused_summary.push_str(&format!(
                " · {:.1}% · {}",
                100. * entry.logical as f64 / whole,
                DateTime::from_timestamp(entry.modified, 0)
                    .filter(|_| entry.modified > 0)
                    .map_or_else(
                        || self.label("Unknown"),
                        |date| time(date, &self.view.settings.language)
                    )
            ));
            if let Some(class) = self.analysis.classifications.get(focused) {
                self.view.focused_summary.push_str(&format!(
                    " · {}",
                    self.label(crate::recommendations::category_key(class.category as u8))
                ));
                if let Some(reclaim) = class.reclaim {
                    self.view.focused_summary.push_str(&format!(
                        " · {}",
                        self.label(crate::recommendations::reclaim_key(reclaim))
                    ));
                }
            }
            if let Some((path, inspection)) = &self.git_result
                && path_key(path) == path_key(&self.view.focused_path)
            {
                self.view.git_summary =
                    crate::recommendations::git_summary(inspection, &self.view.settings.language);
            } else if self.git_desired.is_some()
                || self
                    .git_job
                    .as_ref()
                    .is_some_and(|job| job.generation == self.git_generation)
            {
                self.view.git_summary = self.label("Git.Checking");
            }
        }
    }
    pub fn map_tiles(&self, width: f32, height: f32) -> Vec<MapTile> {
        let key = (self.map_revision, width.to_bits(), height.to_bits());
        if let Some(cache) = self.map_cache.borrow().as_ref()
            && cache.key == key
        {
            return cache.tiles.clone();
        }
        let Some(index) = &self.live else {
            return vec![];
        };
        let metric = match self.view.map_metric {
            1 => MapMetric::Logical,
            2 => MapMetric::Files,
            _ => MapMetric::Allocated,
        };
        let query = self.view.map_query.to_lowercase();
        let tiles = index
            .layout_with_projection(
                self.nav.current,
                (width, height),
                metric,
                &query,
                self.view.map_isolate,
                self.view.map_global,
                self.projection(),
            )
            .into_iter()
            .map(|tile| {
                if tile.index == OTHERS_INDEX {
                    return MapTile {
                        index: tile.index,
                        name: self.label("Map.Others"),
                        x: tile.x,
                        y: tile.y,
                        width: tile.width,
                        height: tile.height,
                        directory: false,
                        marked: false,
                        covered: false,
                        matched: false,
                        depth: tile.depth,
                        category: 8,
                        reclaim: false,
                        age: 0,
                    };
                }
                let entry = &index.entries[tile.index];
                let path = index.path(tile.index);
                MapTile {
                    index: tile.index,
                    name: entry.name.to_string(),
                    directory: entry.directory,
                    marked: self.marked(&path),
                    covered: self.covered(&path),
                    matched: !query.is_empty()
                        && if query.contains('\\') {
                            path.to_lowercase().contains(&query)
                        } else {
                            entry.name.to_lowercase().contains(&query)
                        },
                    x: tile.x,
                    y: tile.y,
                    width: tile.width,
                    height: tile.height,
                    depth: tile.depth,
                    category: self
                        .analysis
                        .classifications
                        .get(tile.index)
                        .map_or(8, |c| c.category as u8),
                    reclaim: self
                        .analysis
                        .classifications
                        .get(tile.index)
                        .is_some_and(|c| c.reclaim.is_some()),
                    age: match AgeBand::from_modified(entry.modified, Utc::now().timestamp()) {
                        AgeBand::Unknown => 0,
                        AgeBand::Within7Days => 1,
                        AgeBand::Within30Days => 2,
                        AgeBand::Within180Days => 3,
                        AgeBand::Within365Days => 4,
                        AgeBand::Older => 5,
                    },
                }
            })
            .collect::<Vec<_>>();
        *self.map_cache.borrow_mut() = Some(MapCache {
            key,
            tiles: tiles.clone(),
        });
        tiles
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.cancel();
        if let Some(job) = self.forecast_job.take() {
            let _ = job.join.join();
        }
        if let Some(job) = self.git_job.take() {
            let _ = job.join.join();
        }
        if let Some(active) = self.active.take() {
            let _ = active.join.join();
        }
        for join in self.journal_workers.drain(..) {
            let _ = join.join();
        }
        self.bridge.take();
    }
}
fn time(value: DateTime<Utc>, lang: &str) -> String {
    value
        .with_timezone(&Local)
        .format(if lang == "ru" {
            "%d.%m.%Y %H:%M %:z"
        } else {
            "%m/%d/%Y %I:%M %p %:z"
        })
        .to_string()
}
fn issue_key(kind: i32) -> &'static str {
    match kind {
        0 => "Issue.AccessDenied",
        1 => "Issue.ReparseSkipped",
        2 => "Issue.CloudSkipped",
        3 => "Issue.MetadataUnavailable",
        4 => "Issue.ChangedDuringScan",
        _ => "Issue.IoFailure",
    }
}
fn outcome_key(kind: CleanupOutcome) -> &'static str {
    match kind {
        CleanupOutcome::Deleted => "Outcome.Deleted",
        CleanupOutcome::Missing => "Outcome.Missing",
        CleanupOutcome::SkippedChanged => "Outcome.SkippedChanged",
        CleanupOutcome::SkippedBusy => "Outcome.SkippedBusy",
        CleanupOutcome::SkippedPolicy => "Outcome.SkippedPolicy",
        CleanupOutcome::Failed => "Outcome.Failed",
    }
}

fn enrich_snapshot(snapshot: &mut ScanSnapshot, cancel: &Cancellation) {
    use diskburrow_windows::{NativeFileApi, WindowsNativeFileApi, equals_path};
    let api = WindowsNativeFileApi;
    for file in &mut snapshot.largest_files {
        if cancel.is_cancelled() {
            return;
        }
        if file.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0 {
            continue;
        }
        let observed = (|| {
            let handle = api.open_metadata(&file.path)?;
            if !equals_path(&api.final_path(&handle)?, &file.path) {
                return Err(std::io::Error::other("Changed ancestor"));
            }
            api.inspect_handle(&handle, &file.path)
        })();
        match observed {
            Ok(mut observed)
                if observed.logical_bytes == file.logical_bytes
                    && observed.modified_utc.timestamp() == file.modified_utc.timestamp()
                    && observed.attributes == file.attributes =>
            {
                observed.allocated_bytes = file.allocated_bytes;
                *file = observed;
            }
            Ok(_) => snapshot.issues.push(ScanIssue {
                path: file.path.clone(),
                kind: ScanIssueKind::ChangedDuringScan,
            }),
            Err(_) => snapshot.issues.push(ScanIssue {
                path: file.path.clone(),
                kind: ScanIssueKind::MetadataUnavailable,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn port_system_theme_is_accepted_by_runtime() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        runtime.command(Command::Setting(Setting::Theme, "system".into()));
        assert_eq!(runtime.view().settings.theme, "system");
        assert!(runtime.view().error.is_none());
    }
    #[test]
    fn cancelled_or_stale_git_result_cannot_publish_verified_counts() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().to_owned()).unwrap();
        idle(&mut runtime);
        let cancel = Cancellation::default();
        cancel.cancel();
        runtime.git_generation = 7;
        runtime.git_job = Some(GitJob {
            generation: 7,
            cancel,
            join: thread::spawn(|| {}),
        });
        let inspection = GitInspection {
            repository: crate::git_inspection::RepositoryState::Repository,
            changed: Some(0),
            untracked: Some(0),
            stash: Some(0),
            ahead: Some(0),
            uncertainty: None,
        };
        runtime
            .tx
            .send(Event::GitFinished {
                generation: 7,
                path: fixture.path().to_string_lossy().into_owned(),
                inspection: inspection.clone(),
            })
            .unwrap();
        runtime.poll();
        assert!(
            runtime.git_result.is_none(),
            "Cancelled work published verified Git counts"
        );
        runtime
            .tx
            .send(Event::GitFinished {
                generation: 6,
                path: fixture.path().to_string_lossy().into_owned(),
                inspection,
            })
            .unwrap();
        runtime.poll();
        assert!(runtime.git_result.is_none());
    }
    #[test]
    fn failed_post_delete_refresh_keeps_report_and_journal() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("scan");
        fs::create_dir(&root).unwrap();
        let file = root.join("owned-fixture.bin");
        fs::write(&file, b"owned fixture").unwrap();
        let data = fixture.path().join("data");
        let mut runtime = Runtime::new(data.clone()).unwrap();
        idle(&mut runtime);
        runtime.command(Command::Setting(Setting::Language, "en".into()));
        runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        runtime.command(Command::Mark(file.to_string_lossy().into_owned()));
        runtime.command(Command::PreviewManual);
        idle(&mut runtime);
        assert!(
            runtime
                .view
                .review
                .as_ref()
                .unwrap()
                .summary
                .contains("Known reclaim on current volume")
        );
        assert!(
            runtime
                .view
                .review
                .as_ref()
                .unwrap()
                .warnings
                .iter()
                .any(|text| text.contains("Git:"))
        );
        let moved = fixture.path().join("moved");
        fs::rename(&root, &moved).unwrap();
        runtime.command(Command::ConfirmManual);
        idle(&mut runtime);
        assert!(runtime.last_report.is_some());
        assert!(moved.join("owned-fixture.bin").exists());
        assert!(
            runtime
                .view
                .error
                .as_ref()
                .is_some_and(|message| message.contains("Deletion completed; refresh scan failed"))
        );
        assert!(!runtime.view.cleanup_results.is_empty());
        for join in runtime.journal_workers.drain(..) {
            join.join().unwrap();
        }
        let report_id = runtime.last_report.as_ref().unwrap().plan_id.to_string();
        let journal = fs::read(data.join("history.db")).unwrap();
        assert!(
            journal
                .windows(report_id.len())
                .any(|bytes| bytes == report_id.as_bytes())
        );
    }
    #[test]
    fn manual_review_repeats_git_after_selection_changes_on_disk() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("scan");
        let repo = root.join("checkout");
        fs::create_dir_all(&repo).unwrap();
        let init = std::process::Command::new("git")
            .args(["-c", "core.hooksPath=NUL", "init", "--quiet", "--template="])
            .current_dir(&repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "NUL")
            .output()
            .unwrap();
        assert!(init.status.success());
        fs::write(repo.join("first.txt"), b"owned fixture").unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        runtime.command(Command::Setting(Setting::Language, "en".into()));
        runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        let index = runtime
            .live
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .position(|entry| entry.name.as_ref() == "checkout")
            .unwrap();
        runtime.command(Command::MapFocus(index));
        let deadline = Instant::now() + Duration::from_secs(10);
        while runtime.git_job.is_some() || runtime.git_desired.is_some() {
            runtime.poll();
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(runtime.view.git_summary.contains("1 untracked"));
        runtime.command(Command::Setting(Setting::Language, "ru".into()));
        assert!(runtime.view.git_summary.contains("1 не отслеживается"));
        runtime.command(Command::Setting(Setting::Language, "en".into()));
        fs::write(repo.join("second.txt"), b"owned fixture").unwrap();
        runtime.command(Command::Mark(repo.to_string_lossy().into_owned()));
        runtime.command(Command::PreviewManual);
        idle(&mut runtime);
        assert!(runtime.view.review.as_ref().unwrap().can_confirm);
        assert!(
            runtime
                .view
                .review
                .as_ref()
                .unwrap()
                .warnings
                .iter()
                .any(|message| message.contains("2 untracked"))
        );
        runtime.command(Command::DismissReview);
        assert!(repo.join("second.txt").exists());
    }
    fn idle(runtime: &mut Runtime) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while runtime.view.busy {
            runtime.poll();
            assert!(Instant::now() < deadline, "Worker timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn failed_automatic_fixture_scan_returns_idle_instead_of_retrying_immediately() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        let missing = fixture
            .path()
            .join("missing")
            .to_string_lossy()
            .into_owned();
        runtime.scheduler = MonitoringScheduler::new(
            AppSettings::default(),
            Utc::now() - chrono::Duration::minutes(10),
            Duration::ZERO,
            missing,
        )
        .unwrap();
        runtime.monitoring = true;
        runtime.poll();
        let first = runtime.active.as_ref().unwrap().token;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            runtime.next_monitor = Instant::now();
            runtime.poll();
            if runtime.active.as_ref().is_none_or(|a| a.token != first) {
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !runtime.view.busy,
            "Failure immediately launched another automatic scan"
        );
        assert!(runtime.view.error.is_some());
        assert!(
            runtime
                .scheduler
                .tick(
                    Utc::now() + chrono::Duration::days(2),
                    runtime.started.elapsed() + Duration::from_secs(901),
                    false,
                    false,
                    true,
                )
                .request_full_scan
                .is_some(),
            "Failed automatic worker never released its scheduler slot"
        );
    }
    #[test]
    fn cancelled_automatic_fixture_scan_waits_six_hours_but_manual_other_root_is_immediate() {
        let fixture = tempfile::tempdir().unwrap();
        let scan_root = fixture.path().join("automatic");
        let manual_root = fixture.path().join("manual");
        fs::create_dir(&scan_root).unwrap();
        fs::create_dir(&manual_root).unwrap();
        fs::write(manual_root.join("one.bin"), b"fixture").unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        let root = scan_root.to_string_lossy().into_owned();
        runtime.scheduler = MonitoringScheduler::new(
            AppSettings::default(),
            Utc::now() - chrono::Duration::minutes(10),
            Duration::ZERO,
            root.clone(),
        )
        .unwrap();
        let request = runtime
            .scheduler
            .tick(Utc::now(), runtime.started.elapsed(), false, false, true)
            .automatic_request_id
            .unwrap();
        runtime.start_automatic_scan(root, request);
        runtime.command(Command::Cancel);
        idle(&mut runtime);
        assert!(runtime.snapshot.is_none(), "Cancelled result was published");
        assert!(
            runtime
                .scheduler
                .tick(
                    Utc::now() + chrono::Duration::days(2),
                    Duration::from_secs(901),
                    false,
                    false,
                    true
                )
                .request_full_scan
                .is_none()
        );
        runtime.command(Command::SetRoot(manual_root.to_string_lossy().into_owned()));
        runtime.command(Command::Scan(false));
        assert!(
            runtime.view.busy,
            "Automatic delay blocked explicit manual scan"
        );
        idle(&mut runtime);
        assert_eq!(
            runtime.snapshot.as_ref().unwrap().root,
            manual_root.to_string_lossy()
        );
        assert!(
            runtime
                .scheduler
                .tick(
                    Utc::now() + chrono::Duration::days(2),
                    Duration::from_secs(21_599),
                    false,
                    false,
                    true
                )
                .request_full_scan
                .is_none()
        );
        assert!(
            runtime
                .scheduler
                .tick(
                    Utc::now() + chrono::Duration::days(2),
                    Duration::from_secs(21_601),
                    false,
                    false,
                    true
                )
                .request_full_scan
                .is_some()
        );
    }
    #[test]
    fn superseded_finished_token_cannot_release_the_automatic_reservation() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        let root = fixture.path().to_string_lossy().into_owned();
        runtime.scheduler = MonitoringScheduler::new(
            AppSettings::default(),
            Utc::now() - chrono::Duration::minutes(10),
            Duration::ZERO,
            root.clone(),
        )
        .unwrap();
        let request = runtime
            .scheduler
            .tick(Utc::now(), Duration::ZERO, false, false, true)
            .automatic_request_id
            .unwrap();
        runtime.spawn(
            Purpose::Scan {
                root: root.clone(),
                fast: false,
            },
            "Status.Scanning",
            |cancel, _, _| {
                while !cancel.is_cancelled() {
                    thread::sleep(Duration::from_millis(5));
                }
                bail!("Cancelled")
            },
        );
        let token = runtime.active.as_ref().unwrap().token;
        runtime.active.as_mut().unwrap().automatic_request = Some((request, root));
        runtime
            .tx
            .send(Event::Finished(
                token - 1,
                Err(anyhow::anyhow!("Late old worker")),
            ))
            .unwrap();
        runtime.poll();
        assert_eq!(runtime.active.as_ref().unwrap().token, token);
        assert!(
            runtime
                .scheduler
                .tick(
                    Utc::now() + chrono::Duration::days(2),
                    Duration::from_secs(90_000),
                    false,
                    false,
                    true
                )
                .request_full_scan
                .is_none()
        );
        runtime.command(Command::Cancel);
        idle(&mut runtime);
    }
    #[test]
    fn manual_system_scan_cancellation_postpones_automatic_scan() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        let root = fixture.path().to_string_lossy().into_owned();
        runtime.scheduler = MonitoringScheduler::new(
            AppSettings::default(),
            Utc::now() - chrono::Duration::minutes(10),
            Duration::ZERO,
            root.clone(),
        )
        .unwrap();
        runtime.command(Command::SetRoot(root));
        runtime.command(Command::Scan(false));
        runtime.command(Command::Cancel);
        idle(&mut runtime);
        assert!(
            runtime
                .scheduler
                .tick(
                    Utc::now() + chrono::Duration::days(2),
                    Duration::from_secs(901),
                    false,
                    false,
                    true
                )
                .request_full_scan
                .is_none()
        );
    }
    #[test]
    fn manual_system_scan_failure_postpones_automatic_scan() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        let root = fixture
            .path()
            .join("missing")
            .to_string_lossy()
            .into_owned();
        runtime.scheduler = MonitoringScheduler::new(
            AppSettings::default(),
            Utc::now() - chrono::Duration::minutes(10),
            Duration::ZERO,
            root.clone(),
        )
        .unwrap();
        runtime.command(Command::SetRoot(root));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        assert!(
            runtime
                .scheduler
                .tick(
                    Utc::now() + chrono::Duration::days(2),
                    Duration::from_secs(899),
                    false,
                    false,
                    true
                )
                .request_full_scan
                .is_none()
        );
    }
    #[test]
    fn non_scan_failure_does_not_postpone_the_automatic_scan() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        runtime.scheduler = MonitoringScheduler::new(
            AppSettings::default(),
            Utc::now() - chrono::Duration::minutes(10),
            Duration::ZERO,
            fixture.path().to_string_lossy().into_owned(),
        )
        .unwrap();
        runtime.spawn(Purpose::Review, "Status.Analyzing", |_, _, _| {
            bail!("Fixture review failed")
        });
        idle(&mut runtime);
        assert!(
            runtime
                .scheduler
                .tick(Utc::now(), Duration::from_secs(1), false, false, true)
                .request_full_scan
                .is_some()
        );
    }
    #[test]
    fn idle_poll_does_not_rebuild_large_view_projections() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().to_owned()).unwrap();
        idle(&mut runtime);
        runtime.view.cleanup_summary = "projection sentinel".into();
        runtime.poll();
        assert_eq!(runtime.view.cleanup_summary, "projection sentinel");
    }
    #[test]
    fn completed_scan_shows_independent_volume_counters_and_coverage() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        runtime.command(Command::Setting(Setting::Language, "en".into()));
        runtime.command(Command::SetRoot(
            fixture.path().to_string_lossy().into_owned(),
        ));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        for key in [
            "Overview.Total",
            "Overview.Free",
            "Overview.Used",
            "Overview.Coverage",
        ] {
            assert!(
                runtime
                    .view
                    .overview
                    .iter()
                    .any(|(label, value)| label == &text("en", key) && !value.is_empty()),
                "Missing {key}"
            );
        }
    }
    #[test]
    fn ordinary_scan_export_and_retained_history_use_only_the_fixture() {
        let fixture = tempfile::tempdir().unwrap();
        let data = fixture.path().join("data");
        let root = fixture.path().join("scan");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("sample.txt"), b"fixture contents").unwrap();
        let root = root.to_string_lossy().into_owned();
        let mut runtime = Runtime::new(data.clone()).unwrap();
        idle(&mut runtime);
        runtime.command(Command::SetRoot(root.clone()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        assert!(runtime.view.error.is_none(), "{:?}", runtime.view.error);
        assert_eq!(runtime.snapshot.as_ref().unwrap().root, root);
        assert_eq!(runtime.live.as_ref().unwrap().entries[0].files, 1);
        let destination = fixture.path().join("report.json");
        runtime.command(Command::Export(destination.to_string_lossy().into_owned()));
        idle(&mut runtime);
        let exported: ScanSnapshot =
            serde_json::from_slice(&fs::read(&destination).unwrap()).unwrap();
        assert_eq!(exported.largest_files[0].logical_bytes, 16);
        let original = fs::read(&destination).unwrap();
        runtime.command(Command::Export(destination.to_string_lossy().into_owned()));
        idle(&mut runtime);
        assert!(runtime.view.error.is_some());
        assert_eq!(fs::read(&destination).unwrap(), original);
        drop(runtime);
        let mut runtime = Runtime::new(data).unwrap();
        idle(&mut runtime);
        runtime.command(Command::SetRoot(root.clone()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        assert_eq!(
            runtime.history.len(),
            2,
            "The first scan must survive application restart"
        );
        assert!(runtime.history.iter().all(|s| s.root == root));
    }
    #[test]
    fn invalid_settings_cannot_be_saved_and_language_updates_columns() {
        let fixture = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::new(fixture.path().to_owned()).unwrap();
        idle(&mut runtime);
        runtime.command(Command::Setting(
            Setting::LowSpaceGb,
            "15.0000000001".into(),
        ));
        runtime.command(Command::SaveSettings);
        assert!(!runtime.view.busy);
        assert!(runtime.view.error.is_some());
        runtime.command(Command::Setting(Setting::LowSpaceGb, "15.000000001".into()));
        runtime.command(Command::Setting(Setting::Language, "en".into()));
        runtime.command(Command::SaveSettings);
        idle(&mut runtime);
        let settings = SettingsStore::new(fixture.path().to_owned()).load();
        assert_eq!(settings.low_space_bytes, 15_000_000_001);
        assert_eq!(settings.language, "en");
    }
    #[test]
    fn manual_review_invalidation_preserves_file_and_fresh_confirmation_deletes_only_fixture() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("scan");
        fs::create_dir(&root).unwrap();
        let file = root.join("owned-test-file.bin");
        fs::write(&file, b"owned fixture").unwrap();
        let mut runtime = Runtime::new(fixture.path().join("data")).unwrap();
        idle(&mut runtime);
        runtime.command(Command::SetRoot(root.to_string_lossy().into_owned()));
        runtime.command(Command::Scan(false));
        idle(&mut runtime);
        assert!(
            runtime.snapshot.as_ref().unwrap().largest_files[0]
                .identity
                .is_some()
        );
        let path = file.to_string_lossy().into_owned();
        runtime.command(Command::Mark(path.clone()));
        runtime.command(Command::PreviewManual);
        idle(&mut runtime);
        assert!(runtime.view.review.as_ref().unwrap().can_confirm);
        runtime.command(Command::Mark(path.clone()));
        runtime.command(Command::ConfirmManual);
        assert!(file.exists());
        assert!(runtime.view.review.is_none());
        runtime.command(Command::Mark(path));
        runtime.command(Command::PreviewManual);
        idle(&mut runtime);
        runtime.command(Command::ConfirmManual);
        idle(&mut runtime);
        assert!(!file.exists());
        assert!(
            runtime
                .snapshot
                .as_ref()
                .unwrap()
                .largest_files
                .iter()
                .all(|entry| path_key(&entry.path) != path_key(&file.to_string_lossy()))
        );
        assert!(
            runtime
                .last_report
                .as_ref()
                .unwrap()
                .items
                .iter()
                .all(|i| i.outcome == CleanupOutcome::Deleted)
        );
    }
}

#[cfg(test)]
#[path = "runtime_port_tests.rs"]
mod port_tests;
