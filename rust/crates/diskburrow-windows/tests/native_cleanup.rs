#![cfg(windows)]
use diskburrow_services::CleanupOutcome;
use diskburrow_windows::*;
use std::os::windows::fs::OpenOptionsExt;
use std::{collections::HashSet, fs, sync::Arc};

// Every destructive test owns a fresh child directory. The caller redirects TEMP
// to its build scratch drive; no existing user TEMP/AppData file is a target.
struct Fixture {
    tree: tempfile::TempDir,
    known: KnownDirectories,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::temp_dir().join("diskburrow-native-fixtures");
        fs::create_dir_all(&base).unwrap();
        let tree = tempfile::Builder::new()
            .prefix("taskowned-cleanup-")
            .tempdir_in(base)
            .unwrap();
        fs::write(
            tree.path().join(".diskburrow-taskowned"),
            "generated native fixture",
        )
        .unwrap();
        let at = |p: &str| tree.path().join(p).to_string_lossy().into_owned();
        let known = KnownDirectories {
            user_profile: at("profile"),
            local_app_data: at(r"profile\AppData\Local"),
            windows: at("Windows"),
            program_files: at("Program Files"),
            program_files_x86: at("Program Files x86"),
            program_data: at("ProgramData"),
            temp_hint: at(r"profile\AppData\Local\Temp"),
            user_library_roots: vec![at(r"profile\Downloads")],
            user_library_roots_verified: true,
        };
        for p in [
            &known.user_profile,
            &known.local_app_data,
            &known.windows,
            &known.program_files,
            &known.program_files_x86,
            &known.program_data,
            &known.temp_hint,
        ] {
            fs::create_dir_all(p).unwrap();
        }
        Self { tree, known }
    }
    fn path(&self, p: &str) -> String {
        self.tree.path().join(p).to_string_lossy().into_owned()
    }
    fn file(&self, p: &str) -> String {
        let p = self.path(p);
        fs::create_dir_all(std::path::Path::new(&p).parent().unwrap()).unwrap();
        fs::write(&p, b"seventeen bytes!!").unwrap();
        p
    }
    fn service(&self) -> CleanupService {
        CleanupService::new(
            Arc::new(FixedRuleEnvironment {
                known: self.known.clone(),
                owner_state: OwnerProcessState::Closed,
            }),
            self.path("application"),
            self.path("data"),
        )
    }
}

type Hook = Box<dyn Fn(&str) + Send + Sync>;
#[derive(Default)]
struct Hooks {
    before_target: Option<Hook>,
    before_directory: Option<Hook>,
    after_inspect: Option<Hook>,
    before_delete: Option<Hook>,
    cloud: Option<String>,
    denied: Option<String>,
}
impl NativeFileApi for Hooks {
    fn open_directory(&self, p: &str) -> std::io::Result<NativeHandle> {
        if let Some(h) = &self.before_directory {
            h(p);
        }
        WindowsNativeFileApi.open_directory(p)
    }
    fn open_target(&self, p: &str) -> std::io::Result<NativeHandle> {
        if let Some(h) = &self.before_target {
            h(p);
        }
        WindowsNativeFileApi.open_target(p)
    }
    fn open_metadata(&self, p: &str) -> std::io::Result<NativeHandle> {
        if self.denied.as_deref().is_some_and(|d| equals_path(d, p)) {
            return Err(std::io::Error::from_raw_os_error(5));
        }
        WindowsNativeFileApi.open_metadata(p)
    }
    fn inspect_handle(&self, h: &NativeHandle, p: &str) -> std::io::Result<NativeFileObservation> {
        let mut o = WindowsNativeFileApi.inspect_handle(h, p)?;
        if self.cloud.as_deref().is_some_and(|c| equals_path(c, p)) {
            o.attributes |= 0x1000;
        }
        if let Some(h) = &self.after_inspect {
            h(p);
        }
        Ok(o)
    }
    fn mark_for_deletion(&self, h: &NativeHandle) -> std::io::Result<()> {
        let path = self.final_path(h)?;
        if let Some(h) = &self.before_delete {
            h(&path);
        }
        WindowsNativeFileApi.mark_for_deletion(h)
    }
}
impl Fixture {
    fn hooked(&self, h: Hooks) -> CleanupService {
        CleanupService::with_native_api(
            Arc::new(FixedRuleEnvironment {
                known: self.known.clone(),
                owner_state: OwnerProcessState::Closed,
            }),
            self.path("application"),
            self.path("data"),
            Arc::new(h),
        )
    }
}

#[test]
fn taskowned_busy_changed_and_identity_replaced_files_are_preserved() {
    let f = Fixture::new();
    let changed = f.file("changed");
    let busy = f.file("busy");
    let replaced = f.file("replaced");
    let svc = f.service();
    let cancel = Cancellation::default();
    let plan = svc
        .preview_manual(&[changed.clone(), busy.clone(), replaced.clone()], &cancel)
        .unwrap();
    let modified = plan
        .entries
        .iter()
        .find(|e| e.file.path == replaced)
        .unwrap()
        .file
        .modified_utc;
    fs::write(&changed, b"changed").unwrap();
    fs::rename(&replaced, format!("{replaced}.old")).unwrap();
    fs::write(&replaced, b"seventeen bytes!!").unwrap();
    fs::File::options()
        .write(true)
        .open(&replaced)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::from(modified)))
        .unwrap();
    let _busy = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&busy)
        .unwrap();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert_eq!(
        2,
        report
            .items
            .iter()
            .filter(|i| matches!(i.outcome, CleanupOutcome::SkippedChanged))
            .count()
    );
    assert_eq!(
        1,
        report
            .items
            .iter()
            .filter(|i| matches!(i.outcome, CleanupOutcome::SkippedBusy))
            .count()
    );
    for p in [changed, busy, replaced] {
        assert!(std::path::Path::new(&p).exists());
    }
}

#[test]
fn taskowned_manual_plan_is_service_owned_even_if_review_dto_is_mutated() {
    let f = Fixture::new();
    let file = f.file("selected");
    let neighbour = f.file("neighbour");
    let svc = f.service();
    let cancel = Cancellation::default();
    let mut plan = svc
        .preview_manual(std::slice::from_ref(&file), &cancel)
        .unwrap();
    plan.entries[0].file.path = neighbour.clone();
    plan.roots.clear();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert_eq!(report.items[0].audit.as_ref().unwrap().path, file);
    assert!(std::path::Path::new(&neighbour).exists());
}

#[test]
fn taskowned_hardlink_neighbour_survives_and_link_count_change_is_detected() {
    let f = Fixture::new();
    let selected = f.file("selected");
    let neighbour = f.path("alias");
    fs::hard_link(&selected, &neighbour).unwrap();
    let svc = f.service();
    let cancel = Cancellation::default();
    let plan = svc
        .preview_manual(std::slice::from_ref(&selected), &cancel)
        .unwrap();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(matches!(report.items[0].outcome, CleanupOutcome::Deleted));
    assert_eq!(2, report.items[0].audit.as_ref().unwrap().link_count);
    assert!(std::path::Path::new(&neighbour).exists());
    let selected = f.file("newselected");
    let plan = svc
        .preview_manual(std::slice::from_ref(&selected), &cancel)
        .unwrap();
    fs::hard_link(&selected, f.path("newalias")).unwrap();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(matches!(
        report.items[0].outcome,
        CleanupOutcome::SkippedChanged
    ));
    assert!(std::path::Path::new(&selected).exists());
}

#[test]
fn taskowned_invalid_boundaries_and_inaccessible_inventory_disable_entire_plan() {
    let f = Fixture::new();
    let file = f.file(r"selected\nested\file");
    let svc = f.hooked(Hooks {
        denied: Some(f.path(r"selected\nested")),
        ..Hooks::default()
    });
    let plan = svc
        .preview_manual(&[f.path("selected")], &Cancellation::default())
        .unwrap();
    assert!(!plan.can_execute());
    assert_eq!(plan.warnings[0].path, Some(f.path(r"selected\nested")));
    let mut known = f.known.clone();
    known.program_files = String::new();
    let invalid = CleanupService::new(
        Arc::new(FixedRuleEnvironment {
            known,
            owner_state: OwnerProcessState::Closed,
        }),
        f.path("application"),
        f.path("data"),
    );
    assert!(
        !invalid
            .preview_manual(&[file], &Cancellation::default())
            .unwrap()
            .can_execute()
    );
}

#[test]
fn taskowned_cloud_directory_never_enumerates_its_children() {
    let f = Fixture::new();
    let root = f.path("selected");
    let child = f.file(r"selected\child");
    let visits = Arc::new(std::sync::Mutex::new(Vec::new()));
    let copy = visits.clone();
    let svc = f.hooked(Hooks {
        cloud: Some(root.clone()),
        after_inspect: Some(Box::new(move |p| copy.lock().unwrap().push(p.to_owned()))),
        ..Hooks::default()
    });
    let plan = svc
        .preview_manual(&[root], &Cancellation::default())
        .unwrap();
    assert!(!plan.can_execute());
    assert!(!visits.lock().unwrap().contains(&child));
    assert!(std::path::Path::new(&child).exists());
}

#[test]
fn taskowned_late_child_and_partial_cancellation_survive() {
    let f = Fixture::new();
    let selected = f.file(r"selected\long-name");
    let remaining = f.file(r"selected\file");
    let arrived = f.path(r"selected\new");
    let stopped = Cancellation::default();
    let copy = stopped.clone();
    let selected_copy = selected.clone();
    let arrived_copy = arrived.clone();
    let svc = f.hooked(Hooks {
        before_delete: Some(Box::new(move |p| {
            if equals_path(p, &selected_copy) {
                fs::write(&arrived_copy, b"late arrival").unwrap();
                copy.cancel();
            }
        })),
        ..Hooks::default()
    });
    let plan = svc
        .preview_manual(&[f.path("selected")], &Cancellation::default())
        .unwrap();
    let report = svc.execute_manual(plan.id, true, &stopped).unwrap();
    assert!(report.was_cancelled);
    assert!(report.free_space_delta_available);
    assert_eq!(plan.entries.len(), report.items.len());
    assert!(
        report
            .items
            .iter()
            .any(|i| matches!(i.outcome, CleanupOutcome::Deleted))
    );
    assert!(std::path::Path::new(&remaining).exists());
    assert!(std::path::Path::new(&arrived).exists());
}

#[test]
fn taskowned_kernel_rejects_child_arriving_after_last_empty_check() {
    let f = Fixture::new();
    let root = f.path("selected");
    fs::create_dir(&root).unwrap();
    let arrived = f.path(r"selected\new");
    let root_copy = root.clone();
    let copy = arrived.clone();
    let svc = f.hooked(Hooks {
        before_delete: Some(Box::new(move |p| {
            if equals_path(p, &root_copy) {
                fs::write(&copy, b"late").unwrap();
            }
        })),
        ..Hooks::default()
    });
    let cancel = Cancellation::default();
    let plan = svc
        .preview_manual(std::slice::from_ref(&root), &cancel)
        .unwrap();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(!matches!(report.items[0].outcome, CleanupOutcome::Deleted));
    assert!(std::path::Path::new(&arrived).exists());
    assert!(std::path::Path::new(&root).exists());
}

#[test]
fn taskowned_final_target_and_parent_leases_deny_actual_rename_and_write() {
    let f = Fixture::new();
    let selected = f.file(r"selected\file");
    let root = f.path("selected");
    let selected_copy = selected.clone();
    let root_copy = root.clone();
    let checked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let copy = checked.clone();
    let svc = f.hooked(Hooks {
        before_delete: Some(Box::new(move |p| {
            if equals_path(p, &selected_copy) {
                assert!(fs::rename(&root_copy, format!("{root_copy}.moved")).is_err());
                assert!(fs::rename(&selected_copy, format!("{selected_copy}.moved")).is_err());
                assert!(fs::write(&selected_copy, b"changed").is_err());
                copy.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        })),
        ..Hooks::default()
    });
    let cancel = Cancellation::default();
    let plan = svc.preview_manual(&[root], &cancel).unwrap();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(checked.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        report
            .items
            .iter()
            .all(|i| matches!(i.outcome, CleanupOutcome::Deleted))
    );
}

#[test]
fn taskowned_cancel_after_lease_does_not_delete_target() {
    let f = Fixture::new();
    let selected = f.file("selected");
    let cancellation = Cancellation::default();
    let copy = cancellation.clone();
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = armed.clone();
    let svc = f.hooked(Hooks {
        before_target: Some(Box::new(move |_| {
            if signal.load(std::sync::atomic::Ordering::SeqCst) {
                copy.cancel();
            }
        })),
        ..Hooks::default()
    });
    let plan = svc
        .preview_manual(std::slice::from_ref(&selected), &Cancellation::default())
        .unwrap();
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let report = svc.execute_manual(plan.id, true, &cancellation).unwrap();
    assert!(report.was_cancelled);
    assert!(std::path::Path::new(&selected).exists());
    assert!(!matches!(report.items[0].outcome, CleanupOutcome::Deleted));
}

#[test]
fn taskowned_redirected_temp_requires_validated_narrow_optin() {
    let f = Fixture::new();
    let redirected = f.file(r"scratch\Temp\old");
    let mut known = f.known.clone();
    known.temp_hint = f.path(r"scratch\Temp");
    let svc = CleanupService::new(
        Arc::new(FixedRuleEnvironment {
            known: known.clone(),
            owner_state: OwnerProcessState::Closed,
        }),
        f.path("application"),
        f.path("data"),
    );
    let (rules, warnings) = svc.discover_rules(&[]);
    assert!(!rules.iter().any(|r| is_within(&redirected, &r.path)));
    assert!(
        warnings
            .iter()
            .any(|w| w.reason_key == "Cleanup.TempApprovalRequired")
    );
    let approved = svc.approve_custom_temp_root(&known.temp_hint).unwrap();
    let (rules, _) = svc.discover_rules(&[approved.path().into()]);
    assert!(rules.iter().any(|r| is_within(&redirected, &r.path)));
    for broad in [
        f.known.user_profile.clone(),
        f.known.local_app_data.clone(),
        f.known.windows.clone(),
        "E:\\".into(),
    ] {
        let mut k = known.clone();
        k.temp_hint = broad.clone();
        let svc = CleanupService::new(
            Arc::new(FixedRuleEnvironment {
                known: k,
                owner_state: OwnerProcessState::Closed,
            }),
            f.path("application"),
            f.path("data"),
        );
        assert!(svc.approve_custom_temp_root(&broad).is_none());
    }
}

struct ProcessEnvironment {
    known: KnownDirectories,
    state: std::sync::atomic::AtomicUsize,
}
impl RuleEnvironment for ProcessEnvironment {
    fn known_directories(&self) -> KnownDirectories {
        self.known.clone()
    }
    fn check_owner_process(&self, _: &str) -> OwnerProcessState {
        match self.state.load(std::sync::atomic::Ordering::SeqCst) {
            0 => OwnerProcessState::Closed,
            1 => OwnerProcessState::Running,
            _ => OwnerProcessState::Unavailable,
        }
    }
}
#[test]
fn taskowned_browser_running_or_unknown_skips_only_http_cache_and_rechecks_execution() {
    let f = Fixture::new();
    let chrome =
        f.file(r"profile\AppData\Local\Google\Chrome\User Data\Default\Cache\Cache_Data\data");
    let edge =
        f.file(r"profile\AppData\Local\Microsoft\Edge\User Data\Profile 1\Cache\Cache_Data\data");
    let cookies = f.file(r"profile\AppData\Local\Google\Chrome\User Data\Default\Network\Cookies");
    let env = Arc::new(ProcessEnvironment {
        known: f.known.clone(),
        state: std::sync::atomic::AtomicUsize::new(0),
    });
    let svc = CleanupService::new(env.clone(), f.path("application"), f.path("data"));
    let cancel = Cancellation::default();
    let plan = svc.preview_cleanup(&[], &[], &cancel).unwrap();
    assert_eq!(2, plan.candidates.len());
    let selected = plan.candidates.iter().map(|c| c.id).collect();
    env.state.store(1, std::sync::atomic::Ordering::SeqCst);
    let report = svc
        .execute_cleanup(plan.id, &selected, true, &cancel)
        .unwrap();
    assert!(
        report
            .items
            .iter()
            .all(|i| matches!(i.outcome, CleanupOutcome::SkippedPolicy))
    );
    for path in [&chrome, &edge, &cookies] {
        assert!(std::path::Path::new(path).exists());
    }
    for state in [1, 2] {
        env.state.store(state, std::sync::atomic::Ordering::SeqCst);
        let plan = svc.preview_cleanup(&[], &[], &cancel).unwrap();
        assert!(plan.candidates.is_empty());
        assert!(plan.warnings.iter().any(|w| w.reason_key
            == if state == 1 {
                "Cleanup.OwnerRunning"
            } else {
                "Cleanup.OwnerUnavailable"
            }));
    }
}

#[test]
fn taskowned_browser_owner_state_rechecked_after_initial_metadata() {
    let f = Fixture::new();
    let cache =
        f.file(r"profile\AppData\Local\Google\Chrome\User Data\Default\Cache\Cache_Data\data");
    let env = Arc::new(ProcessEnvironment {
        known: f.known.clone(),
        state: std::sync::atomic::AtomicUsize::new(0),
    });
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let copy = armed.clone();
    let owner = env.clone();
    let cache_copy = cache.clone();
    let api = Hooks {
        after_inspect: Some(Box::new(move |p| {
            if copy.load(std::sync::atomic::Ordering::SeqCst) && equals_path(p, &cache_copy) {
                owner.state.store(1, std::sync::atomic::Ordering::SeqCst);
            }
        })),
        ..Hooks::default()
    };
    let svc =
        CleanupService::with_native_api(env, f.path("application"), f.path("data"), Arc::new(api));
    let cancel = Cancellation::default();
    let plan = svc.preview_cleanup(&[], &[], &cancel).unwrap();
    let selected = plan.candidates.iter().map(|c| c.id).collect();
    armed.store(true, std::sync::atomic::Ordering::SeqCst);
    let report = svc
        .execute_cleanup(plan.id, &selected, true, &cancel)
        .unwrap();
    assert!(matches!(
        report.items[0].outcome,
        CleanupOutcome::SkippedPolicy
    ));
    assert!(std::path::Path::new(&cache).exists());
}

#[test]
fn taskowned_cleanup_exclusions_and_unknown_selection_never_authorize_other_names() {
    let f = Fixture::new();
    let keep = f.file(r"profile\AppData\Local\Temp\keep\file");
    let keeper = f.file(r"profile\AppData\Local\Temp\keeper\file");
    for path in [&keep, &keeper] {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                std::time::SystemTime::now() - std::time::Duration::from_secs(9 * 86400),
            ))
            .unwrap();
    }
    let svc = f.service();
    let cancel = Cancellation::default();
    let excluded = f.path(r"profile\AppData\Local\Temp\KEEP\..\KEEP\");
    let plan = svc.preview_cleanup(&[excluded], &[], &cancel).unwrap();
    assert_eq!(1, plan.candidates.len());
    assert_eq!(keeper, plan.candidates[0].file.path);
    assert!(
        svc.execute_cleanup(
            plan.id,
            &HashSet::from([uuid::Uuid::new_v4()]),
            true,
            &cancel
        )
        .is_err()
    );
    assert!(
        svc.execute_cleanup(
            plan.id,
            &HashSet::from([plan.candidates[0].id]),
            false,
            &cancel
        )
        .is_err()
    );
    let report = svc
        .execute_cleanup(
            plan.id,
            &HashSet::from([plan.candidates[0].id]),
            true,
            &cancel,
        )
        .unwrap();
    assert!(matches!(report.items[0].outcome, CleanupOutcome::Deleted));
    assert!(std::path::Path::new(&keep).exists());
}

fn set_attributes(path: &str, attributes: u32) {
    let p: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    assert_ne!(0, unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(p.as_ptr(), attributes)
    });
}
#[test]
fn taskowned_readonly_system_and_offline_entries_block_manual_inventory() {
    for attr in [1, 4, 0x1000] {
        let f = Fixture::new();
        let file = f.file(r"selected\file");
        set_attributes(&file, attr);
        let svc = f.service();
        let plan = svc
            .preview_manual(&[f.path("selected")], &Cancellation::default())
            .unwrap();
        set_attributes(&file, 0x80);
        assert!(!plan.can_execute());
        assert_eq!(plan.warnings[0].path, Some(file));
    }
}

#[test]
fn taskowned_changed_metadata_between_handle_reads_is_skipped() {
    for mode in [0, 1, 2] {
        let f = Fixture::new();
        let file = f.file("selected");
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let copy = armed.clone();
        let fired = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let once = fired.clone();
        let file_copy = file.clone();
        let svc = f.hooked(Hooks {
            after_inspect: Some(Box::new(move |p| {
                if equals_path(p, &file_copy)
                    && copy.load(std::sync::atomic::Ordering::SeqCst)
                    && !once.swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    match mode {
                        0 => {
                            // Attribute-only writers are not excluded by data sharing flags.
                            use std::os::windows::io::AsRawHandle;
                            let h = fs::OpenOptions::new().access_mode(0x100).share_mode(7).open(p).unwrap();
                            let info = windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO {
                                CreationTime: 0, LastAccessTime: 0, LastWriteTime: 1, ChangeTime: 0, FileAttributes: 0,
                            };
                            assert_ne!(0, unsafe {
                                windows_sys::Win32::Storage::FileSystem::SetFileInformationByHandle(
                                    h.as_raw_handle(), windows_sys::Win32::Storage::FileSystem::FileBasicInfo,
                                    (&info as *const windows_sys::Win32::Storage::FileSystem::FILE_BASIC_INFO).cast(),
                                    std::mem::size_of_val(&info) as u32,
                                )
                            });
                        }
                        1 => set_attributes(p, 1),
                        _ => set_attributes(p, 0x1000),
                    }
                }
            })),
            ..Hooks::default()
        });
        let cancel = Cancellation::default();
        let plan = svc
            .preview_manual(std::slice::from_ref(&file), &cancel)
            .unwrap();
        armed.store(true, std::sync::atomic::Ordering::SeqCst);
        let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
        set_attributes(&file, 0x80);
        assert!(!matches!(report.items[0].outcome, CleanupOutcome::Deleted));
        assert!(std::path::Path::new(&file).exists());
    }
}

/// Real mount-point reparse mutation, confined to generated task-owned fixtures.
fn junction(f: &Fixture, path: &str, target: &str) -> u32 {
    assert!(is_within(path, &f.tree.path().to_string_lossy()));
    assert!(is_within(target, &f.tree.path().to_string_lossy()));
    assert!(f.tree.path().join(".diskburrow-taskowned").is_file());
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, INVALID_HANDLE_VALUE},
        Storage::FileSystem::*,
        System::IO::DeviceIoControl,
    };
    let substitute = format!(r"\??\{}", target)
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let print = target
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let mut data = vec![0u8; 16 + substitute.len() + 2 + print.len() + 2];
    data[..4].copy_from_slice(&0xA0000003u32.to_le_bytes());
    let size = (data.len() - 8) as u16;
    data[4..6].copy_from_slice(&size.to_le_bytes());
    data[10..12].copy_from_slice(&(substitute.len() as u16).to_le_bytes());
    data[12..14].copy_from_slice(&((substitute.len() + 2) as u16).to_le_bytes());
    data[14..16].copy_from_slice(&(print.len() as u16).to_le_bytes());
    data[16..16 + substitute.len()].copy_from_slice(&substitute);
    let offset = 16 + substitute.len() + 2;
    data[offset..offset + print.len()].copy_from_slice(&print);
    let p: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    let h = unsafe {
        CreateFileW(
            p.as_ptr(),
            0x40000000,
            7,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return unsafe { GetLastError() };
    }
    let mut returned = 0;
    let ok = unsafe {
        DeviceIoControl(
            h,
            0x900A4,
            data.as_ptr().cast(),
            data.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    let error = if ok != 0 {
        0
    } else {
        unsafe { GetLastError() }
    };
    unsafe {
        CloseHandle(h);
    }
    error
}

#[test]
fn taskowned_actual_junction_descendant_blocks_plan_and_external_sentinel_survives() {
    let f = Fixture::new();
    let sentinel = f.file(r"outside\sentinel");
    let link = f.path(r"selected\link");
    fs::create_dir_all(&link).unwrap();
    assert_eq!(0, junction(&f, &link, &f.path("outside")));
    let plan = f
        .service()
        .preview_manual(&[f.path("selected")], &Cancellation::default())
        .unwrap();
    assert!(!plan.can_execute());
    assert_eq!(plan.warnings[0].path, Some(link));
    assert!(std::path::Path::new(&sentinel).exists());
}

#[test]
fn taskowned_native_directory_leases_deny_reparse_mutation() {
    let f = Arc::new(Fixture::new());
    let root = f.path("selected");
    fs::create_dir(&root).unwrap();
    let sentinel = f.file(r"outside\sentinel");
    let control = f.path("control");
    fs::create_dir(&control).unwrap();
    assert_eq!(0, junction(&f, &control, &f.path("outside")));
    fs::remove_dir(&control).unwrap();
    let root_copy = root.clone();
    let tree = f.clone();
    let svc = f.hooked(Hooks {
        before_delete: Some(Box::new(move |p| {
            if equals_path(p, &root_copy) {
                assert_eq!(32, junction(&tree, p, &tree.path("outside")));
            }
        })),
        ..Hooks::default()
    });
    let cancel = Cancellation::default();
    let plan = svc.preview_manual(&[root], &cancel).unwrap();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(matches!(report.items[0].outcome, CleanupOutcome::Deleted));
    assert!(std::path::Path::new(&sentinel).exists());
}

#[test]
fn taskowned_native_parent_replacement_before_execution_never_deletes_outside() {
    let f = Fixture::new();
    let selected = f.file(r"selected\file");
    let sentinel = f.file(r"outside\file");
    let root = f.path("selected");
    let svc = f.service();
    let cancel = Cancellation::default();
    let plan = svc
        .preview_manual(std::slice::from_ref(&root), &cancel)
        .unwrap();
    fs::rename(&root, format!("{root}.saved")).unwrap();
    fs::create_dir(&root).unwrap();
    assert_eq!(0, junction(&f, &root, &f.path("outside")));
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(
        report
            .items
            .iter()
            .all(|i| !matches!(i.outcome, CleanupOutcome::Deleted))
    );
    assert!(std::path::Path::new(&sentinel).exists());
    assert!(std::path::Path::new(&format!("{root}.saved\\file")).exists());
    assert!(std::path::Path::new(&selected).exists());
}

#[test]
fn taskowned_manual_review_is_readonly_deduplicated_singleuse_and_neighbours_survive() {
    let f = Fixture::new();
    let file = f.file(r"selected\sub\file");
    let neighbour = f.file(r"unselected\file");
    let svc = f.service();
    let cancel = Cancellation::default();
    let root = f.path("selected");
    let plan = svc
        .preview_manual(&[root.clone(), file.clone(), root.to_uppercase()], &cancel)
        .unwrap();
    assert!(plan.can_execute());
    assert_eq!(1, plan.roots.len());
    assert_eq!(3, plan.entries.len());
    assert!(std::path::Path::new(&file).exists());
    assert!(svc.execute_manual(plan.id, false, &cancel).is_err());
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert_eq!(3, report.items.len());
    assert!(report.free_space_delta_available);
    assert!(
        report
            .items
            .iter()
            .all(|x| matches!(x.outcome, CleanupOutcome::Deleted) && x.audit.is_some())
    );
    assert!(!std::path::Path::new(&root).exists());
    assert!(std::path::Path::new(&neighbour).exists());
    assert!(svc.execute_manual(plan.id, true, &cancel).is_err());
}

#[test]
fn taskowned_new_inventory_and_cancelled_preview_invalidate_review() {
    let f = Fixture::new();
    let file = f.file(r"selected\file");
    let svc = f.service();
    let cancel = Cancellation::default();
    let plan = svc.preview_manual(&[f.path("selected")], &cancel).unwrap();
    let arrived = f.file(r"selected\new");
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(
        report
            .items
            .iter()
            .all(|x| matches!(x.outcome, CleanupOutcome::SkippedChanged))
    );
    assert!(std::path::Path::new(&arrived).exists());
    let plan = svc
        .preview_manual(std::slice::from_ref(&file), &cancel)
        .unwrap();
    let stopped = Cancellation::default();
    stopped.cancel();
    assert!(
        svc.preview_manual(std::slice::from_ref(&file), &stopped)
            .is_err()
    );
    assert!(svc.execute_manual(plan.id, true, &cancel).is_err());
    assert!(std::path::Path::new(&file).exists());
}

#[test]
fn path_boundaries_and_protected_containers_fail_closed() {
    assert_eq!(
        normalize_local_path(r"E:\work\keep\..\KEEP\").unwrap(),
        r"E:\work\KEEP"
    );
    assert!(!is_within(r"E:\keeper", r"E:\keep"));
    for p in [
        r"\\server\share\x",
        r"\\?\E:\x",
        r"E:\x:stream",
        r"E:\x.",
        r"E:\NUL",
        r"E:\x ",
    ] {
        assert!(normalize_local_path(p).is_none(), "{p}");
    }
    let f = Fixture::new();
    let svc = f.service();
    let cancel = Cancellation::default();
    for path in [
        "E:\\".to_owned(),
        f.known.user_profile.clone(),
        f.known.local_app_data.clone(),
        f.known.windows.clone(),
        f.path("application"),
        f.path("data"),
        f.path(r"selected\..\other"),
    ] {
        assert!(!svc.preview_manual(&[path], &cancel).unwrap().can_execute());
    }
}

#[test]
fn taskowned_precancelled_execution_audits_every_item() {
    let f = Fixture::new();
    let file = f.file(r"selected\file");
    let svc = f.service();
    let plan = svc
        .preview_manual(&[f.path("selected")], &Cancellation::default())
        .unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    let report = svc.execute_manual(plan.id, true, &cancel).unwrap();
    assert!(report.was_cancelled);
    assert_eq!(plan.entries.len(), report.items.len());
    assert!(report.free_space_delta_available);
    assert!(report.items.iter().all(|x| x.audit.is_some()));
    assert!(std::path::Path::new(&file).exists());
}

#[test]
fn taskowned_metadata_has_stable_identity_and_hardlink_allocation() {
    let f = Fixture::new();
    let file = f.file("file");
    let link = f.path("link");
    fs::hard_link(&file, &link).unwrap();
    let a = inspect(&file).unwrap();
    let b = inspect(&link).unwrap();
    assert_eq!(a.identity, b.identity);
    assert_eq!(2, a.link_count);
    assert_eq!(17, a.logical_bytes);
    assert!(a.allocated_bytes.is_some());
}

#[test]
fn taskowned_reclaim_excludes_complete_and_external_native_hardlinks_without_overlap() {
    let f = Fixture::new();
    let selected = f.file(r"selected\file");
    fs::write(&selected, vec![5u8; 65_536]).unwrap();
    let internal = f.path(r"selected\alias");
    fs::hard_link(&selected, &internal).unwrap();
    let svc = f.service();
    let cancel = Cancellation::default();
    let plan = svc
        .preview_manual(&[f.path("selected"), internal], &cancel)
        .unwrap();
    let observation = inspect(&selected).unwrap();
    assert!(observation.allocated_bytes.unwrap() > 0);
    let volume = observation.identity.unwrap().volume;
    let estimate = project_manual_reclaim(&plan, volume);
    assert_eq!(estimate.known_reclaim_bytes, 0);
    assert_eq!(estimate.reclaimable_files, 0);
    assert_eq!(estimate.excluded_hardlink_files, 1);
    let outside = f.path("outside-alias");
    fs::hard_link(&selected, &outside).unwrap();
    let plan = svc.preview_manual(&[f.path("selected")], &cancel).unwrap();
    let estimate = project_manual_reclaim(&plan, volume);
    assert_eq!(estimate.known_reclaim_bytes, 0);
    assert_eq!(estimate.excluded_hardlink_files, 1);
    assert!(std::path::Path::new(&outside).exists());
}

#[test]
fn taskowned_cleanup_age_browser_scope_and_selected_ids() {
    let f = Fixture::new();
    let old = f.file(r"profile\AppData\Local\Temp\old");
    let exact = f.file(r"profile\AppData\Local\Temp\exact");
    let dump = f.file(r"profile\AppData\Local\CrashDumps\app.DMP");
    let txt = f.file(r"profile\AppData\Local\CrashDumps\notes.txt");
    let cache =
        f.file(r"profile\AppData\Local\Google\Chrome\User Data\Default\Cache\Cache_Data\data");
    let cookies = f.file(r"profile\AppData\Local\Google\Chrome\User Data\Default\Network\Cookies");
    let now = chrono::Utc::now();
    let now =
        chrono::DateTime::from_timestamp(now.timestamp(), now.timestamp_subsec_nanos() / 100 * 100)
            .unwrap();
    for path in [&old, &dump, &txt] {
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(std::time::SystemTime::from(now - chrono::Duration::days(8))),
            )
            .unwrap();
    }
    fs::File::options()
        .write(true)
        .open(&exact)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(std::time::SystemTime::from(now - chrono::Duration::days(7))),
        )
        .unwrap();
    let svc = f.service();
    let cancel = Cancellation::default();
    let plan = svc.preview_cleanup_at(&[], &[], now, &cancel).unwrap();
    let names: HashSet<_> = plan
        .candidates
        .iter()
        .map(|c| c.file.path.clone())
        .collect();
    assert_eq!(names, HashSet::from([old.clone(), dump, cache]));
    assert!(plan.selected_ids.is_empty());
    let selected = plan
        .candidates
        .iter()
        .filter(|c| c.file.path == old)
        .map(|c| c.id)
        .collect();
    let report = svc
        .execute_cleanup(plan.id, &selected, true, &cancel)
        .unwrap();
    assert_eq!(1, report.items.len());
    assert!(matches!(report.items[0].outcome, CleanupOutcome::Deleted));
    assert!(std::path::Path::new(&cookies).exists());
    assert!(std::path::Path::new(&txt).exists());
}

#[test]
fn taskowned_manual_observer_preserves_issued_preview_and_cannot_authorize_deletion() {
    let f = Fixture::new();
    let reviewed = f.file("reviewed");
    let observed = f.file("observed");
    let svc = f.service();
    let cancel = Cancellation::default();
    let issued = svc
        .preview_manual(std::slice::from_ref(&reviewed), &cancel)
        .unwrap();
    let observation = svc
        .observe_manual(std::slice::from_ref(&observed), &cancel)
        .unwrap();
    assert_eq!(observation.file_count(), 1);
    assert_ne!(observation.id, issued.id);
    let error = svc
        .execute_manual(observation.id, true, &cancel)
        .unwrap_err();
    assert!(error.to_string().contains("Manual.PlanUnavailable"));
    assert!(std::path::Path::new(&observed).exists());
    let report = svc.execute_manual(issued.id, true, &cancel).unwrap();
    assert!(
        report
            .items
            .iter()
            .all(|item| item.outcome == CleanupOutcome::Deleted)
    );
    assert!(!std::path::Path::new(&reviewed).exists());
    assert!(std::path::Path::new(&observed).exists());
}

#[test]
fn taskowned_cancelled_manual_observer_preserves_issued_preview() {
    let f = Fixture::new();
    let file = f.file("reviewed");
    let svc = f.service();
    let cancel = Cancellation::default();
    let issued = svc
        .preview_manual(std::slice::from_ref(&file), &cancel)
        .unwrap();
    let stopped = Cancellation::default();
    stopped.cancel();
    assert!(
        svc.observe_manual(std::slice::from_ref(&file), &stopped)
            .is_err()
    );
    let report = svc.execute_manual(issued.id, true, &cancel).unwrap();
    assert!(
        report
            .items
            .iter()
            .all(|item| item.outcome == CleanupOutcome::Deleted)
    );
    assert!(!std::path::Path::new(&file).exists());
}

#[test]
fn taskowned_manual_observer_caps_input_roots_without_opening_metadata() {
    let f = Fixture::new();
    let file = f.file("observed");
    let opened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = opened.clone();
    let svc = f.hooked(Hooks {
        after_inspect: Some(Box::new(move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })),
        ..Hooks::default()
    });
    let plan = svc
        .observe_manual(&vec![file.clone(); 2001], &Cancellation::default())
        .unwrap();
    assert!(plan.roots.is_empty());
    assert!(plan.entries.is_empty());
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.reason_key == "Manual.ObservationLimit")
    );
    assert_eq!(opened.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert!(std::path::Path::new(&file).exists());
}

#[test]
fn taskowned_manual_observer_cancels_during_directory_enumeration() {
    let f = Fixture::new();
    for n in 0..12 {
        f.file(&format!("observed\\file-{n}"));
    }
    let root = f.path("observed");
    let cancel = Cancellation::default();
    let signal = cancel.clone();
    let watched = root.clone();
    let svc = f.hooked(Hooks {
        after_inspect: Some(Box::new(move |path| {
            if equals_path(path, &watched) {
                signal.cancel();
            }
        })),
        ..Hooks::default()
    });
    assert!(svc.observe_manual(&[root], &cancel).is_err());
    assert!(std::path::Path::new(&f.path("observed\\file-11")).exists());
}

#[test]
fn taskowned_manual_observer_rejects_long_or_deep_roots_before_native_metadata() {
    let f = Fixture::new();
    let opened = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = opened.clone();
    let svc = f.hooked(Hooks {
        after_inspect: Some(Box::new(move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })),
        ..Hooks::default()
    });
    for root in [
        f.path(&"a".repeat(16_385)),
        f.path(&vec!["a"; 65].join("\\")),
    ] {
        let plan = svc
            .observe_manual(&[root], &Cancellation::default())
            .unwrap();
        assert!(plan.roots.is_empty());
        assert!(plan.entries.is_empty());
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.reason_key == "Manual.ObservationLimit")
        );
    }
    assert_eq!(opened.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[derive(Clone, Copy, Debug)]
enum ObservationCheckpointTrigger {
    AncestorOpen,
    DeepAncestorOpen,
    AncestorInspect,
    AncestorFinalPath,
    TargetMetadata,
    TargetInspect,
    TargetDirectoryOpen,
    TargetFinalPath,
    VerifyInspect,
}
struct ObservationCheckpointApi {
    watched: String,
    trigger: ObservationCheckpointTrigger,
    cancel: Cancellation,
    delay: std::time::Duration,
    ancestor_open_count: std::sync::atomic::AtomicUsize,
    fired: std::sync::atomic::AtomicBool,
    target_seen: std::sync::atomic::AtomicBool,
    calls_after_trigger: std::sync::atomic::AtomicUsize,
}
impl ObservationCheckpointApi {
    fn before_call(&self) {
        if self.fired.load(std::sync::atomic::Ordering::Acquire) {
            self.calls_after_trigger
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    fn after_call(&self, event: &str, target: bool) {
        use ObservationCheckpointTrigger as Trigger;
        if event == "directory" && !target {
            self.ancestor_open_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let fire = match self.trigger {
            Trigger::AncestorOpen => event == "directory" && !target,
            Trigger::DeepAncestorOpen => {
                event == "directory"
                    && !target
                    && self
                        .ancestor_open_count
                        .load(std::sync::atomic::Ordering::Relaxed)
                        == 3
            }
            Trigger::AncestorInspect => event == "inspect" && !target,
            Trigger::AncestorFinalPath => event == "final" && !target,
            Trigger::TargetMetadata => event == "metadata" && target,
            Trigger::TargetInspect => event == "inspect" && target,
            Trigger::TargetDirectoryOpen => event == "directory" && target,
            Trigger::TargetFinalPath => event == "final" && target,
            Trigger::VerifyInspect => {
                event == "inspect"
                    && !target
                    && self.target_seen.load(std::sync::atomic::Ordering::Acquire)
            }
        };
        if event == "inspect" && target {
            self.target_seen
                .store(true, std::sync::atomic::Ordering::Release);
        }
        if fire && !self.fired.swap(true, std::sync::atomic::Ordering::AcqRel) {
            if !self.delay.is_zero() {
                std::thread::sleep(self.delay);
            } else {
                self.cancel.cancel();
            }
        }
    }
}
impl NativeFileApi for ObservationCheckpointApi {
    fn open_directory(&self, path: &str) -> std::io::Result<NativeHandle> {
        self.before_call();
        let handle = WindowsNativeFileApi.open_directory(path)?;
        self.after_call("directory", equals_path(path, &self.watched));
        Ok(handle)
    }
    fn open_metadata(&self, path: &str) -> std::io::Result<NativeHandle> {
        self.before_call();
        let handle = WindowsNativeFileApi.open_metadata(path)?;
        self.after_call("metadata", equals_path(path, &self.watched));
        Ok(handle)
    }
    fn inspect_handle(
        &self,
        handle: &NativeHandle,
        path: &str,
    ) -> std::io::Result<NativeFileObservation> {
        self.before_call();
        let observation = WindowsNativeFileApi.inspect_handle(handle, path)?;
        self.after_call("inspect", equals_path(path, &self.watched));
        Ok(observation)
    }
    fn final_path(&self, handle: &NativeHandle) -> std::io::Result<String> {
        self.before_call();
        let path = WindowsNativeFileApi.final_path(handle)?;
        self.after_call("final", equals_path(&path, &self.watched));
        Ok(path)
    }
}
fn checkpoint_observer(fixture: &Fixture, api: Arc<ObservationCheckpointApi>) -> CleanupService {
    CleanupService::with_native_api(
        Arc::new(FixedRuleEnvironment {
            known: fixture.known.clone(),
            owner_state: OwnerProcessState::Closed,
        }),
        fixture.path("application"),
        fixture.path("data"),
        api,
    )
}
fn checkpoint_api(
    path: String,
    trigger: ObservationCheckpointTrigger,
    delay: bool,
) -> Arc<ObservationCheckpointApi> {
    Arc::new(ObservationCheckpointApi {
        watched: path,
        trigger,
        delay: if delay {
            std::time::Duration::from_millis(3100)
        } else {
            std::time::Duration::ZERO
        },
        ancestor_open_count: std::sync::atomic::AtomicUsize::new(0),
        cancel: Cancellation::default(),
        fired: std::sync::atomic::AtomicBool::new(false),
        target_seen: std::sync::atomic::AtomicBool::new(false),
        calls_after_trigger: std::sync::atomic::AtomicUsize::new(0),
    })
}
#[test]
fn taskowned_observer_checks_cancellation_before_every_next_native_call() {
    use ObservationCheckpointTrigger as Trigger;
    let fixture = Fixture::new();
    fixture.file("observed\\child");
    let root = fixture.path("observed");
    for trigger in [
        Trigger::AncestorOpen,
        Trigger::AncestorInspect,
        Trigger::AncestorFinalPath,
        Trigger::TargetMetadata,
        Trigger::TargetInspect,
        Trigger::TargetDirectoryOpen,
        Trigger::TargetFinalPath,
        Trigger::VerifyInspect,
    ] {
        let api = checkpoint_api(root.clone(), trigger, false);
        let observer = checkpoint_observer(&fixture, api.clone());
        let error = observer
            .observe_manual(std::slice::from_ref(&root), &api.cancel)
            .unwrap_err();
        assert!(
            error.to_string().contains("Cleanup.Cancelled"),
            "{trigger:?}: {error}"
        );
        assert!(
            api.fired.load(std::sync::atomic::Ordering::Acquire),
            "{trigger:?}"
        );
        assert_eq!(
            api.calls_after_trigger
                .load(std::sync::atomic::Ordering::Relaxed),
            0,
            "{trigger:?}: cancellation must stop before the next native call"
        );
    }
    assert!(std::path::Path::new(&fixture.path("observed\\child")).exists());
}
#[test]
fn taskowned_observer_checks_deadline_after_native_open_before_next_inspect() {
    let fixture = Fixture::new();
    let file = fixture.file("observed");
    for trigger in [
        ObservationCheckpointTrigger::AncestorOpen,
        ObservationCheckpointTrigger::TargetMetadata,
    ] {
        let api = checkpoint_api(file.clone(), trigger, true);
        let observer = checkpoint_observer(&fixture, api.clone());
        let plan = observer
            .observe_manual(std::slice::from_ref(&file), &api.cancel)
            .unwrap();
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.reason_key == "Manual.ObservationLimit"),
            "{trigger:?}"
        );
        assert!(plan.entries.is_empty());
        assert!(api.fired.load(std::sync::atomic::Ordering::Acquire));
        assert_eq!(
            api.calls_after_trigger
                .load(std::sync::atomic::Ordering::Relaxed),
            0,
            "{trigger:?}: expired deadline must stop before the next native call"
        );
    }
    assert!(std::path::Path::new(&file).exists());
}

#[test]
fn taskowned_observer_shared_deadline_expired_before_metadata_starts() {
    let fixture = Fixture::new();
    let file = fixture.file("observed");
    let api = checkpoint_api(
        file.clone(),
        ObservationCheckpointTrigger::AncestorOpen,
        false,
    );
    // Count all calls without injecting cancellation: an expired caller deadline
    // must reject the input before the first native operation starts.
    api.fired.store(true, std::sync::atomic::Ordering::Release);
    let observer = checkpoint_observer(&fixture, api.clone());
    let plan = observer
        .observe_manual_since(
            std::slice::from_ref(&file),
            &api.cancel,
            std::time::Instant::now() - std::time::Duration::from_secs(4),
        )
        .unwrap();
    assert!(plan.entries.is_empty());
    assert!(plan.roots.is_empty());
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.reason_key == "Manual.ObservationLimit")
    );
    assert_eq!(
        api.calls_after_trigger
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert!(std::path::Path::new(&file).exists());
}

#[test]
fn taskowned_observer_shared_deadline_is_checked_inside_deep_ancestor_chain() {
    let fixture = Fixture::new();
    let file = fixture.file("one\\two\\three\\four\\five\\six\\seven\\eight\\observed");
    let mut api = checkpoint_api(
        file.clone(),
        ObservationCheckpointTrigger::DeepAncestorOpen,
        true,
    );
    Arc::get_mut(&mut api).unwrap().delay = std::time::Duration::from_millis(150);
    let observer = checkpoint_observer(&fixture, api.clone());
    let plan = observer
        .observe_manual_since(
            std::slice::from_ref(&file),
            &api.cancel,
            std::time::Instant::now() - std::time::Duration::from_millis(2900),
        )
        .unwrap();
    assert!(api.fired.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        api.ancestor_open_count
            .load(std::sync::atomic::Ordering::Relaxed),
        3
    );
    assert_eq!(
        api.calls_after_trigger
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert!(plan.entries.is_empty());
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.reason_key == "Manual.ObservationLimit")
    );
    assert!(std::path::Path::new(&file).exists());
}
