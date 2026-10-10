//! Opt-in headless verification using an explicit data directory and read-only scan root.
use crate::{
    contract::{Command, Setting},
    locale,
    runtime::Runtime,
};
use anyhow::{Result, ensure};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

fn await_idle(runtime: &mut Runtime) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(600);
    while runtime.view().busy {
        runtime.poll();
        if Instant::now() > deadline {
            runtime.command(Command::Cancel);
            anyhow::bail!("Verification worker timed out");
        }
        thread::sleep(Duration::from_millis(10));
    }
    ensure!(
        runtime.view().error.is_none(),
        "{}",
        runtime.view().error.as_deref().unwrap_or("")
    );
    Ok(())
}
pub fn run(data: PathBuf, root: String, destination: &Path) -> Result<()> {
    let started = Instant::now();
    let stage = |name: &str| {
        let _ = writeln!(
            std::io::stderr(),
            "Verification: {name} ({:.2}s)",
            started.elapsed().as_secs_f64()
        );
    };
    ensure!(
        destination.is_absolute() && !destination.exists(),
        "A new absolute report path is required"
    );
    let mut runtime = Runtime::new(data)?;
    await_idle(&mut runtime)?;
    stage("startup ready");
    runtime.apply_launch(crate::cli::LaunchOverrides {
        root: Some(root),
        ..Default::default()
    })?;
    let launch_opened_map = runtime.take_map_open();
    ensure!(launch_opened_map, "Explicit launch did not request the map");
    await_idle(&mut runtime)?;
    stage("scan completed");
    ensure!(
        runtime.view().map_has_data,
        "A completed live index is required"
    );
    let snapshot_path = destination.with_extension("snapshot.json");
    runtime.command(Command::Export(
        snapshot_path.to_string_lossy().into_owned(),
    ));
    await_idle(&mut runtime)?;
    stage("snapshot exported");
    let snapshot: diskburrow_services::ScanSnapshot =
        serde_json::from_slice(&fs::read(&snapshot_path)?)?;
    let mut appearances = vec![];
    for language in ["ru", "en"] {
        for theme in ["light", "dark"] {
            runtime.command(Command::Setting(Setting::Language, language.into()));
            runtime.command(Command::Setting(Setting::Theme, theme.into()));
            stage("appearance projected");
            let view = runtime.view();
            appearances.push(serde_json::json!({"language":language,"theme":theme,"path_header":locale::text(language,"Path"),"logical_header":locale::text(language,"Logical"),"overview":view.overview,"folders":view.folders.iter().map(|r|(&r.path,&r.cells)).collect::<Vec<_>>(),"files":view.files.iter().map(|r|(&r.path,&r.cells)).collect::<Vec<_>>(),"insights_heading":locale::text(language,"Insights.Title"),"recommendations":view.recommendations.iter().map(|r|(&r.path,&r.cells)).collect::<Vec<_>>() }));
        }
    }
    let tiles = runtime.map_tiles(650., 400.);
    ensure!(
        tiles
            .iter()
            .all(|t| t.x.is_finite() && t.y.is_finite() && t.width >= 0. && t.height >= 0.),
        "Invalid map geometry"
    );
    let mut display_projections = vec![];
    for depth in [1, 3, 6] {
        runtime.command(Command::Setting(Setting::MapDepth, depth.to_string()));
        for hidden in [true, false] {
            runtime.command(Command::Setting(Setting::ShowHidden, hidden.to_string()));
            let projection = runtime.map_tiles(650., 400.);
            stage("depth and hidden projected");
            ensure!(
                projection.iter().all(|tile| tile.depth <= depth),
                "Map depth was not bounded"
            );
            display_projections.push(serde_json::json!({"depth":depth,"show_hidden":hidden,"tiles":projection.len(),"summary":runtime.view().map_visible_summary}));
        }
    }
    runtime.command(Command::Setting(Setting::ShowHidden, "true".into()));
    runtime.command(Command::Setting(Setting::MapDepth, "3".into()));
    runtime.command(Command::MapColor(1));
    let aged = runtime.map_tiles(650., 400.);
    ensure!(aged.iter().all(|tile| tile.age <= 5), "Unknown age band");
    runtime.command(Command::Setting(Setting::Theme, "system".into()));
    runtime.command(Command::Setting(Setting::UiScale, "150".into()));
    let display_settings = serde_json::json!({"theme":runtime.view().settings.theme,"system_dark":crate::appearance::system_dark(),"ui_scale_percent":runtime.view().settings.ui_scale_percent,"map_depth":runtime.view().settings.map_depth,"sidebar_width":runtime.view().settings.sidebar_width});
    stage("display checks completed");
    let mut selected_count = 0;
    if let Some(file) = snapshot.largest_files.first() {
        runtime.command(Command::Mark(file.path.clone()));
        let selection = runtime.selected_export().expect("Completed index");
        selected_count = selection.count;
        ensure!(
            selected_count == 1 && !selection.prompt.is_empty(),
            "Selection review export missing"
        );
        runtime.command(Command::ExportSelection(
            destination
                .with_extension("selected.txt")
                .to_string_lossy()
                .into_owned(),
        ));
        await_idle(&mut runtime)?;
        stage("selection exported");
    }
    runtime.poll();
    let forecast_deadline = Instant::now() + Duration::from_secs(15);
    while runtime.forecast_pending() {
        ensure!(
            Instant::now() < forecast_deadline,
            "Forecast verification timed out"
        );
        thread::sleep(Duration::from_millis(10));
        runtime.poll();
    }
    ensure!(
        runtime.view().review.is_none(),
        "Read-only forecast created a deletion review"
    );
    stage("forecast completed");
    let map_panel = serde_json::json!({
        "explicit_launch_opened_map": launch_opened_map,
        "marked_roots": runtime.view().map_marked.len(),
        "marked_summary": runtime.view().map_marked_summary,
        "forecast": runtime.view().map_forecast,
        "forecast_worker_finished": !runtime.forecast_pending(),
        "review_created": runtime.view().review.is_some(),
        "recommendations": runtime.view().recommendations.len()
    });
    let report = serde_json::json!({"product_version":env!("CARGO_PKG_VERSION"),"executable":std::env::current_exe()?,"ordinary_user":!crate::helper::is_elevated()?,"scan_root":snapshot.root,"traversal_completed":snapshot.traversal_completed,"directories":snapshot.directories.len(),"largest_files":snapshot.largest_files.len(),"issues":snapshot.issues.len(),"map_tiles":tiles.len(),"reclaim_tiles":tiles.iter().filter(|tile|tile.reclaim).count(),"map_categories":tiles.iter().map(|tile|tile.category).collect::<Vec<_>>(),"appearances":appearances,"display_projections":display_projections,"display_settings":display_settings,"age_bands":aged.iter().map(|tile|tile.age).collect::<Vec<_>>(),"selected_count":selected_count,"snapshot_export":snapshot_path,"verification_scope":"Real executable, ordinary metadata scan, live map geometry, indexed category/reclaim/recommendation/age/depth/hidden projections, system display settings and selected TXT export; no GUI clicks, UAC approval, deletion, tray or autostart"});
    let mut report = report;
    report["map_panel"] = map_panel;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.flush()?;
    file.sync_all()?;
    stage("report completed");
    Ok(())
}
