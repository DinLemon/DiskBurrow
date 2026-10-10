//! Boundary tests for the Windows runtime port, using only owned temp fixtures.
use super::*;
use std::path::Path;

fn idle(runtime: &mut Runtime) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while runtime.view.busy {
        runtime.poll();
        assert!(
            Instant::now() < deadline,
            "Worker timed out: {}",
            runtime.view.status
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        runtime.active.is_none(),
        "Idle runtime retained an active worker"
    );
}

fn local(path: &Path) -> String {
    normalize_local_path(path.to_str().expect("Owned fixture path is Unicode"))
        .expect("Owned fixture uses an ordinary absolute local drive path")
}

fn scanned(data: PathBuf, root: &Path) -> Runtime {
    let mut runtime = Runtime::new(data).unwrap();
    idle(&mut runtime);
    runtime.command(Command::Setting(Setting::Language, "en".into()));
    runtime.command(Command::SetRoot(local(root)));
    runtime.command(Command::Scan(false));
    idle(&mut runtime);
    assert!(runtime.view.error.is_none(), "{:?}", runtime.view.error);
    assert!(runtime.snapshot.as_ref().unwrap().traversal_completed);
    assert_eq!(runtime.live.as_ref().unwrap().root, local(root));
    runtime
}

fn index_of(runtime: &Runtime, path: &Path) -> usize {
    let expected = local(path);
    let live = runtime.live.as_ref().unwrap();
    (0..live.entries.len())
        .find(|&i| equals_path(&live.path(i), &expected))
        .expect("Fixture path is present in the live scan")
}

#[test]
fn port_parent_absorbs_children_and_covered_toggle_preserves_review() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    let parent = root.join("Родитель");
    fs::create_dir_all(&parent).unwrap();
    let child = parent.join("child.bin");
    fs::write(&child, b"owned fixture").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::Mark(local(&child)));
    runtime.command(Command::Mark(local(&parent)));
    assert_eq!(runtime.marks.len(), 1, "parent must absorb its child");
    assert_eq!(runtime.view.map_marked[0].path, local(&parent));
    runtime.command(Command::PreviewManual);
    idle(&mut runtime);
    let id = runtime.view.review.as_ref().unwrap().id.clone();
    runtime.command(Command::Mark(local(&child)));
    assert_eq!(runtime.marks.len(), 1);
    assert_eq!(runtime.view.review.as_ref().unwrap().id, id);
    assert!(runtime.covered(&local(&child)));
    assert!(child.exists());
}

fn forecast_idle(runtime: &mut Runtime) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        runtime.poll();
        if runtime.forecast_job.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "Forecast worker timed out");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn panel_forecast_uses_physical_metadata_without_creating_review_authority() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("known.bin");
    fs::write(&file, vec![7; 8192]).unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::Mark(local(&file)));
    forecast_idle(&mut runtime);
    let observation = runtime
        .forecast_result
        .as_ref()
        .expect("Owned native metadata can be observed");
    assert_eq!(observation.projection.reclaimable_files, 1);
    assert!(observation.projection.known_reclaim_bytes > 0);
    assert!(!observation.partial);
    assert!(
        runtime
            .view
            .map_forecast
            .contains("Free after successful deletion")
    );
    assert_eq!(runtime.view.map_marked.len(), 1);
    assert!(runtime.view.review.is_none());
    assert!(runtime.manual_plan.is_none());
    runtime.command(Command::ConfirmManual);
    assert!(file.exists(), "Forecast must never authorize a deletion");
}

#[test]
fn panel_changed_root_discards_pending_and_finished_forecasts() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    let other = fixture.path().join("other");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&other).unwrap();
    let file = root.join("known.bin");
    fs::write(&file, b"data").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::Mark(local(&file)));
    runtime.poll();
    runtime.command(Command::SetRoot(local(&other)));
    forecast_idle(&mut runtime);
    assert!(runtime.forecast_result.is_none());
    assert!(runtime.view.map_marked.is_empty());
    assert!(
        !runtime
            .view
            .map_forecast
            .contains("Free after successful deletion")
    );
    assert!(file.exists());
}

#[test]
fn panel_explicit_launch_waits_for_work_then_scans_with_session_overrides() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("папка с пробелами");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("data.bin"), b"owned").unwrap();
    let data = fixture.path().join("data");
    let mut runtime = Runtime::new(data.clone()).unwrap();
    assert!(
        runtime.view.busy,
        "Initial history read occupies coordinator"
    );
    runtime
        .apply_launch(crate::cli::LaunchOverrides {
            root: Some(local(&root)),
            show_hidden: Some(false),
            depth: Some(6),
            metric: Some(1),
            ..Default::default()
        })
        .unwrap();
    assert!(runtime.pending_launch.is_some());
    assert!(runtime.take_map_open());
    idle(&mut runtime);
    assert_eq!(runtime.live.as_ref().unwrap().root, local(&root));
    assert!(!runtime.view.settings.show_hidden);
    assert_eq!(runtime.view.settings.map_depth, 6);
    assert_eq!(runtime.view.map_metric, 2);
    assert!(runtime.pending_launch.is_none());
    let saved = SettingsStore::new(data).load();
    assert_eq!(saved.map_depth, AppSettings::default().map_depth);
}

#[test]
fn panel_confirmed_deletion_waits_for_observer_leases_and_queued_launch() {
    use diskburrow_windows::{NativeFileApi as _, WindowsNativeFileApi};
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    let parent = root.join("selected");
    let next = fixture.path().join("next");
    fs::create_dir_all(&parent).unwrap();
    fs::create_dir_all(&next).unwrap();
    let file = parent.join("child.bin");
    fs::write(&file, b"owned child").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::Mark(local(&parent)));
    runtime.command(Command::PreviewManual);
    idle(&mut runtime);
    forecast_idle(&mut runtime);
    let generation = runtime.forecast_generation;
    let scan_id = runtime.scan_id().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let path = local(&parent);
    let tx = runtime.tx.clone();
    let join = thread::spawn(move || {
        let lease = WindowsNativeFileApi.open_directory(&path).unwrap();
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(3));
        drop(lease);
        let _ = tx.send(Event::ForecastFinished {
            generation,
            scan_id,
            observation: None,
        });
    });
    ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    runtime.forecast_job = Some(ForecastJob {
        generation,
        scan_id,
        cancel: Cancellation::default(),
        join,
    });
    runtime.command(Command::ConfirmManual);
    runtime
        .queue_launch(crate::cli::LaunchOverrides {
            root: Some(local(&next)),
            ..Default::default()
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < deadline {
        runtime.poll();
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        file.exists(),
        "Deletion must wait until canceled observer releases directory leases"
    );
    assert!(
        runtime.last_report.is_none(),
        "No attempt may finish before the observation barrier"
    );
    assert_eq!(
        runtime.view.root,
        local(&root),
        "Queued launch must wait for deletion"
    );
    release_tx.send(()).unwrap();
    idle(&mut runtime);
    assert!(
        !parent.exists(),
        "Both owned child and directory should be deleted after release"
    );
    assert!(
        runtime
            .last_report
            .as_ref()
            .unwrap()
            .items
            .iter()
            .all(|r| r.outcome == CleanupOutcome::Deleted)
    );
    assert_eq!(runtime.view.root, local(&next));
    assert!(runtime.pending_launch.is_none());
}

#[test]
fn port_selected_txt_is_outermost_utf8_and_preserves_an_existing_destination() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    let directory = root.join("owned-directory");
    fs::create_dir_all(&directory).unwrap();
    let child = directory.join("child.txt");
    let sibling = root.join("данные.txt");
    fs::write(&child, b"child data must survive export").unwrap();
    fs::write(&sibling, b"sibling data must survive export").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    for path in [&directory, &child, &sibling] {
        runtime.command(Command::Mark(local(path)));
    }
    let live = runtime.live.as_ref().unwrap().clone();
    let snapshot = runtime.snapshot.as_ref().unwrap().clone();
    let marks = runtime.marks.clone();
    let export = runtime.selected_export().unwrap();
    assert_eq!(export.count, 2);
    assert!(export.prompt.contains("separate confirmation"));
    assert!(!export.list.lines().any(|line| line == local(&child)));
    let existing = fixture.path().join("existing-list.txt");
    let sentinel = b"previous user document\0is preserved";
    fs::write(&existing, sentinel).unwrap();
    runtime.command(Command::ExportSelection(local(&existing)));
    idle(&mut runtime);
    assert!(runtime.view.error.is_some());
    assert_eq!(fs::read(&existing).unwrap(), sentinel);
    let destination = fixture.path().join("selected.txt");
    runtime.command(Command::ExportSelection(local(&destination)));
    idle(&mut runtime);
    assert!(runtime.view.error.is_none(), "{:?}", runtime.view.error);
    assert_eq!(fs::read_to_string(&destination).unwrap(), export.list);
    let lines = export.list.lines().collect::<Vec<_>>();
    assert!(lines.contains(&local(&directory).as_str()));
    assert!(lines.contains(&local(&sibling).as_str()));
    assert_eq!(lines.len(), 2);
    assert_eq!(fs::read(&child).unwrap(), b"child data must survive export");
    assert_eq!(
        fs::read(&sibling).unwrap(),
        b"sibling data must survive export"
    );
    assert!(Arc::ptr_eq(runtime.live.as_ref().unwrap(), &live));
    assert!(Arc::ptr_eq(runtime.snapshot.as_ref().unwrap(), &snapshot));
    assert_eq!(runtime.marks, marks);
    assert!(runtime.manual_plan.is_none());
    assert!(runtime.last_report.is_none());
}

#[test]
fn port_empty_selection_and_relative_destination_do_not_create_output() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    fs::create_dir(&root).unwrap();
    let file = root.join("keep.txt");
    fs::write(&file, b"keep").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    let destination = fixture.path().join("empty.txt");
    runtime.command(Command::ExportSelection(local(&destination)));
    assert!(!runtime.view.busy);
    assert!(!destination.exists());
    runtime.command(Command::Mark(local(&file)));
    runtime.command(Command::ExportSelection(
        "relative-selected-list.txt".into(),
    ));
    assert!(!runtime.view.busy);
    assert!(runtime.view.error.is_some());
    assert_eq!(fs::read(&file).unwrap(), b"keep");
}

#[test]
fn port_widen_reuses_ordinary_small_subtree_and_focuses_its_previous_root() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let parent = fixture.path().join("tree");
    let root = parent.join("scan");
    fs::create_dir_all(&root).unwrap();
    let kept = root.join("kept.txt");
    fs::write(&kept, b"kept").unwrap();
    fs::write(parent.join("outside-old-root.txt"), b"new sibling").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::Mark(local(&kept)));
    let marks = runtime.marks.clone();
    let old_snapshot = runtime.snapshot.as_ref().unwrap().clone();
    let cache = runtime
        .scan_cache
        .as_ref()
        .expect("Small ordinary scan is cached");
    assert_eq!(cache.root, local(&root));
    assert_eq!(cache.entries, runtime.live.as_ref().unwrap().entries.len());
    assert!(cache.known_for(&local(&parent)).is_some());
    runtime.command(Command::MapWiden(local(&parent)));
    assert!(runtime.view.busy);
    assert_eq!(
        runtime.view.root,
        local(&root),
        "Pending widen changed completed root"
    );
    idle(&mut runtime);
    assert!(runtime.view.error.is_none(), "{:?}", runtime.view.error);
    assert_eq!(runtime.view.root, local(&parent));
    assert_eq!(runtime.snapshot.as_ref().unwrap().root, local(&parent));
    assert_ne!(runtime.snapshot.as_ref().unwrap().id, old_snapshot.id);
    let previous_root = index_of(&runtime, &root);
    assert_eq!(runtime.nav.current, previous_root);
    assert_eq!(runtime.nav.focused, Some(previous_root));
    assert_eq!(runtime.view.map_path, local(&root));
    assert_eq!(runtime.view.focused_path, local(&root));
    assert_eq!(runtime.live.as_ref().unwrap().entries[0].files, 2);
    assert_eq!(runtime.view.scan_reuse_notice, text("en", "Map.Cached"));
    assert_eq!(runtime.scan_cache.as_ref().unwrap().root, local(&parent));
    assert_eq!(runtime.marks, marks);
    assert_eq!(fs::read(&kept).unwrap(), b"kept");
}

#[test]
fn port_cancelled_widen_keeps_completed_snapshot_navigation_cache_and_marks() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let parent = fixture.path().join("tree");
    let root = parent.join("scan");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("kept.txt");
    fs::write(&file, b"unchanged").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::Mark(local(&file)));
    runtime.command(Command::MapFocus(index_of(&runtime, &file)));
    let snapshot = runtime.snapshot.as_ref().unwrap().clone();
    let live = runtime.live.as_ref().unwrap().clone();
    let marks = runtime.marks.clone();
    let focused = runtime.nav.focused;
    let map_path = runtime.view.map_path.clone();
    runtime.command(Command::MapWiden(local(&parent)));
    assert!(runtime.view.busy);
    // No poll intervenes: even an already completed worker must fail the
    // cancellation/generation publication gate after this explicit cancellation.
    runtime.command(Command::Cancel);
    idle(&mut runtime);
    assert_eq!(runtime.view.root, local(&root));
    assert!(Arc::ptr_eq(runtime.snapshot.as_ref().unwrap(), &snapshot));
    assert!(Arc::ptr_eq(runtime.live.as_ref().unwrap(), &live));
    assert_eq!(runtime.nav.focused, focused);
    assert_eq!(runtime.view.map_path, map_path);
    assert_eq!(runtime.scan_cache.as_ref().unwrap().root, local(&root));
    assert_eq!(runtime.marks, marks);
    assert_eq!(runtime.view.status, text("en", "Status.Cancelled"));
    assert_eq!(fs::read(&file).unwrap(), b"unchanged");
}

#[test]
fn port_failed_widen_keeps_completed_view_when_owned_ancestor_has_disappeared() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let parent = fixture.path().join("tree");
    let root = parent.join("scan");
    fs::create_dir_all(&root).unwrap();
    let file = root.join("kept.txt");
    fs::write(&file, b"owned file survives failed widening").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::Mark(local(&file)));
    let snapshot = runtime.snapshot.as_ref().unwrap().clone();
    let live = runtime.live.as_ref().unwrap().clone();
    let map_path = runtime.view.map_path.clone();
    let rows = runtime
        .view
        .map_objects
        .iter()
        .map(|row| row.path.clone())
        .collect::<Vec<_>>();
    let marks = runtime.marks.clone();
    let moved = fixture.path().join("moved-owned-tree");
    assert!(parent.starts_with(fixture.path()) && moved.starts_with(fixture.path()));
    fs::rename(&parent, &moved).unwrap();
    runtime.command(Command::MapWiden(local(&parent)));
    assert!(
        runtime.view.busy,
        "The strict ancestor request was not admitted"
    );
    idle(&mut runtime);
    assert!(
        runtime.view.error.is_some(),
        "Missing ancestor scan was accepted"
    );
    assert_eq!(runtime.view.root, local(&root));
    assert!(Arc::ptr_eq(runtime.snapshot.as_ref().unwrap(), &snapshot));
    assert!(Arc::ptr_eq(runtime.live.as_ref().unwrap(), &live));
    assert_eq!(runtime.view.map_path, map_path);
    assert_eq!(
        runtime
            .view
            .map_objects
            .iter()
            .map(|row| row.path.clone())
            .collect::<Vec<_>>(),
        rows
    );
    assert_eq!(runtime.marks, marks);
    assert_eq!(runtime.scan_cache.as_ref().unwrap().root, local(&root));
    assert_eq!(
        fs::read(moved.join("scan/kept.txt")).unwrap(),
        b"owned file survives failed widening"
    );
}

#[test]
fn port_hidden_projection_excludes_descendants_from_lists_search_and_tiles_without_losing_marks() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    let hidden = root.join(".hidden");
    fs::create_dir_all(&hidden).unwrap();
    let secret = hidden.join("needle-secret.txt");
    let visible = root.join("visible.txt");
    fs::write(&secret, b"hidden retained content").unwrap();
    fs::write(&visible, b"visible retained content").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::MapMetric(1));
    runtime.command(Command::Setting(Setting::MapDepth, "6".into()));
    runtime.command(Command::Mark(local(&secret)));
    let live = runtime.live.as_ref().unwrap().clone();
    let snapshot = runtime.snapshot.as_ref().unwrap().clone();
    let hidden_index = index_of(&runtime, &hidden);
    let secret_index = index_of(&runtime, &secret);
    assert!(
        runtime
            .map_tiles(1200., 900.)
            .iter()
            .any(|tile| tile.index == hidden_index)
    );
    assert!(
        runtime
            .view
            .files
            .iter()
            .any(|row| equals_path(&row.path, &local(&secret)))
    );
    runtime.command(Command::Setting(Setting::ShowHidden, "false".into()));
    assert!(
        !runtime
            .view
            .map_objects
            .iter()
            .any(|row| is_within(&row.path, &local(&hidden)))
    );
    assert!(
        !runtime
            .view
            .folders
            .iter()
            .any(|row| is_within(&row.path, &local(&hidden)))
    );
    assert!(
        !runtime
            .view
            .files
            .iter()
            .any(|row| is_within(&row.path, &local(&hidden)))
    );
    assert!(
        !runtime
            .map_tiles(1200., 900.)
            .iter()
            .any(|tile| tile.index == hidden_index || tile.index == secret_index)
    );
    assert!(
        runtime
            .view
            .files
            .iter()
            .any(|row| equals_path(&row.path, &local(&visible)))
    );
    runtime.command(Command::MapSearch("needle-secret".into()));
    runtime.command(Command::MapGlobal(true));
    runtime.command(Command::MapIsolate(true));
    assert_eq!(runtime.view.map_matches, 0);
    assert!(runtime.view.map_objects.is_empty());
    assert!(runtime.map_tiles(1200., 900.).is_empty());
    assert!(runtime.marks.contains(&local(&secret).to_lowercase()));
    assert_eq!(runtime.selected_export().unwrap().count, 1);
    assert!(Arc::ptr_eq(runtime.live.as_ref().unwrap(), &live));
    assert!(Arc::ptr_eq(runtime.snapshot.as_ref().unwrap(), &snapshot));
    assert!(
        runtime
            .view
            .map_visible_summary
            .contains(&text("en", "Map.Visible"))
    );
    assert!(
        runtime
            .view
            .map_visible_summary
            .contains(&text("en", "Map.FullScan"))
    );
    runtime.command(Command::Setting(Setting::ShowHidden, "true".into()));
    assert_eq!(runtime.view.map_matches, 1);
    assert_eq!(runtime.view.map_objects[0].path, local(&secret));
    assert!(
        runtime
            .map_tiles(1200., 900.)
            .iter()
            .any(|tile| tile.index == secret_index)
    );
    assert_eq!(fs::read(&secret).unwrap(), b"hidden retained content");
}

#[test]
fn port_hiding_current_subtree_returns_to_visible_ancestor_and_blocks_hidden_navigation() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    let visible = root.join("visible");
    let hidden = root.join(".hidden");
    let hidden_child = hidden.join("child");
    fs::create_dir_all(&visible).unwrap();
    fs::create_dir_all(&hidden_child).unwrap();
    let secret = hidden_child.join("secret.txt");
    fs::write(&secret, b"hidden focus remains data").unwrap();
    fs::write(visible.join("safe.txt"), b"visible").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    let visible_index = index_of(&runtime, &visible);
    let hidden_index = index_of(&runtime, &hidden_child);
    let secret_index = index_of(&runtime, &secret);
    runtime.command(Command::MapNavigate(visible_index));
    runtime.command(Command::MapNavigate(hidden_index));
    runtime.command(Command::MapFocus(secret_index));
    runtime.command(Command::Mark(local(&secret)));
    assert_eq!(runtime.view.map_path, local(&hidden_child));
    assert_eq!(runtime.view.focused_path, local(&secret));
    runtime.command(Command::Setting(Setting::ShowHidden, "false".into()));
    assert_eq!(
        runtime.nav.current, 0,
        "Hidden current directory was not lifted to its visible ancestor"
    );
    assert_eq!(runtime.view.map_path, local(&root));
    assert!(!is_within(&runtime.view.focused_path, &local(&hidden)));
    assert!(
        runtime
            .nav
            .focused
            .is_none_or(|i| runtime.live.as_ref().unwrap().visible(i, false))
    );
    runtime.command(Command::MapNavigate(hidden_index));
    assert_eq!(
        runtime.nav.current, 0,
        "A stale hidden tile navigated into the excluded subtree"
    );
    runtime.command(Command::MapFocus(secret_index));
    assert!(!is_within(&runtime.view.focused_path, &local(&hidden)));
    assert!(
        runtime
            .nav
            .focused
            .is_none_or(|i| runtime.live.as_ref().unwrap().visible(i, false))
    );
    runtime.command(Command::MapBack);
    assert!(
        runtime
            .live
            .as_ref()
            .unwrap()
            .visible(runtime.nav.current, false)
    );
    assert!(!is_within(&runtime.view.map_path, &local(&hidden)));
    runtime.command(Command::MapForward);
    assert!(
        runtime
            .live
            .as_ref()
            .unwrap()
            .visible(runtime.nav.current, false)
    );
    assert!(!is_within(&runtime.view.map_path, &local(&hidden)));
    assert!(!is_within(&runtime.view.focused_path, &local(&hidden)));
    assert!(runtime.marks.contains(&local(&secret).to_lowercase()));
    assert_eq!(fs::read(&secret).unwrap(), b"hidden focus remains data");
}

#[test]
fn port_map_depth_changes_cached_layout_without_rescanning_or_replacing_snapshot() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let root = fixture.path().join("scan");
    let mut deepest = root.clone();
    for number in 1..=6 {
        deepest = deepest.join(format!("level{number}"));
    }
    fs::create_dir_all(&deepest).unwrap();
    fs::write(deepest.join("keep.txt"), b"positive logical size").unwrap();
    let mut runtime = scanned(fixture.path().join("data"), &root);
    runtime.command(Command::MapMetric(1));
    let snapshot = runtime.snapshot.as_ref().unwrap().clone();
    let live = runtime.live.as_ref().unwrap().clone();
    let target = index_of(&runtime, &deepest);
    runtime.command(Command::Setting(Setting::MapDepth, "1".into()));
    let shallow = runtime.map_tiles(1200., 900.);
    assert_eq!(shallow.len(), 1);
    assert!(shallow.iter().all(|tile| tile.depth == 0));
    assert!(!shallow.iter().any(|tile| tile.index == target));
    runtime.command(Command::Setting(Setting::MapDepth, "6".into()));
    let deep = runtime.map_tiles(1200., 900.);
    assert!(
        deep.iter()
            .any(|tile| tile.index == target && tile.depth == 5)
    );
    assert!(deep.iter().all(|tile| tile.depth < 6));
    runtime.command(Command::Setting(Setting::MapDepth, "1".into()));
    assert_eq!(runtime.map_tiles(1200., 900.).len(), 1);
    assert!(Arc::ptr_eq(runtime.live.as_ref().unwrap(), &live));
    assert!(Arc::ptr_eq(runtime.snapshot.as_ref().unwrap(), &snapshot));
    assert!(!runtime.view.busy);
}

#[test]
fn port_display_settings_persist_and_invalid_edits_cannot_replace_the_saved_values() {
    let fixture = tempfile::Builder::new()
        .prefix("diskburrow-port-")
        .tempdir()
        .unwrap();
    let data = fixture.path().join("data");
    let mut runtime = Runtime::new(data.clone()).unwrap();
    idle(&mut runtime);
    for (setting, value) in [
        (Setting::Language, "en"),
        (Setting::Theme, "system"),
        (Setting::UiScale, "125"),
        (Setting::MapDepth, "6"),
        (Setting::ShowHidden, "false"),
        (Setting::SidebarWidth, "420"),
    ] {
        runtime.command(Command::Setting(setting, value.into()));
        assert!(runtime.view.error.is_none(), "{:?}", runtime.view.error);
    }
    let expected = runtime.view.settings.clone();
    runtime.command(Command::SaveSettings);
    idle(&mut runtime);
    assert!(runtime.view.error.is_none(), "{:?}", runtime.view.error);
    assert_eq!(SettingsStore::new(data.clone()).load(), expected);
    drop(runtime);
    let mut runtime = Runtime::new(data.clone()).unwrap();
    idle(&mut runtime);
    assert_eq!(runtime.view.settings, expected);
    for (setting, value) in [
        (Setting::Theme, "auto"),
        (Setting::UiScale, "80"),
        (Setting::MapDepth, "0"),
        (Setting::ShowHidden, "sometimes"),
        (Setting::SidebarWidth, "421"),
    ] {
        runtime.command(Command::Setting(setting, value.into()));
        assert!(runtime.view.error.is_some());
        assert_eq!(runtime.view.settings, expected);
        runtime.command(Command::SaveSettings);
        assert!(!runtime.view.busy, "Invalid display edits launched a save");
        assert_eq!(SettingsStore::new(data.clone()).load(), expected);
    }
}
