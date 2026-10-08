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
    ensure!(
        destination.is_absolute() && !destination.exists(),
        "A new absolute report path is required"
    );
    let mut runtime = Runtime::new(data)?;
    await_idle(&mut runtime)?;
    runtime.command(Command::SetRoot(root));
    runtime.command(Command::Scan(false));
    await_idle(&mut runtime)?;
    ensure!(
        runtime.view().map_has_data,
        "A completed live index is required"
    );
    let snapshot_path = destination.with_extension("snapshot.json");
    runtime.command(Command::Export(
        snapshot_path.to_string_lossy().into_owned(),
    ));
    await_idle(&mut runtime)?;
    let snapshot: diskburrow_services::ScanSnapshot =
        serde_json::from_slice(&fs::read(&snapshot_path)?)?;
    let mut appearances = vec![];
    for language in ["ru", "en"] {
        for theme in ["light", "dark"] {
            runtime.command(Command::Setting(Setting::Language, language.into()));
            runtime.command(Command::Setting(Setting::Theme, theme.into()));
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
    let report = serde_json::json!({"product_version":env!("CARGO_PKG_VERSION"),"executable":std::env::current_exe()?,"ordinary_user":!crate::helper::is_elevated()?,"scan_root":snapshot.root,"traversal_completed":snapshot.traversal_completed,"directories":snapshot.directories.len(),"largest_files":snapshot.largest_files.len(),"issues":snapshot.issues.len(),"map_tiles":tiles.len(),"reclaim_tiles":tiles.iter().filter(|tile|tile.reclaim).count(),"map_categories":tiles.iter().map(|tile|tile.category).collect::<Vec<_>>(),"appearances":appearances,"snapshot_export":snapshot_path,"verification_scope":"Real executable, ordinary metadata scan, live map geometry, indexed category/reclaim/recommendation projections and export; no GUI clicks, UAC approval, deletion, tray or autostart"});
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}
