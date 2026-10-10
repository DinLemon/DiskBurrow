use crate::contract::Row;
use crate::git_inspection::{GitInspection, GitUncertainty, RepositoryState};
use crate::locale::{gb, text};
use diskburrow_engine::{
    Category, EntryClassification, LiveIndex, Reclaim, Recommendation, RecommendationReason,
};
use diskburrow_services::DirectoryObservation;

pub fn category_key(category: u8) -> &'static str {
    match category {
        0 => "Category.Code",
        1 => "Category.AgentScratch",
        2 => "Category.Toolchain",
        3 => "Category.Synced",
        4 => "Category.Git",
        5 => "Category.Media",
        6 => "Category.Documents",
        7 => "Category.Cache",
        _ => "Category.Other",
    }
}
pub fn reclaim_key(reclaim: Reclaim) -> &'static str {
    match reclaim {
        Reclaim::Regenerable => "Reclaim.Regenerable",
        Reclaim::SyncHistory => "Reclaim.SyncHistory",
        Reclaim::PackageStore => "Reclaim.PackageStore",
        Reclaim::BuildOutput => "Reclaim.BuildOutput",
        Reclaim::Reinstallable => "Reclaim.Reinstallable",
        Reclaim::SandboxLayers => "Reclaim.SandboxLayers",
        Reclaim::Snapshots => "Reclaim.Snapshots",
        Reclaim::Trash => "Reclaim.Trash",
        Reclaim::Temporary => "Reclaim.Temporary",
    }
}
pub fn insights(
    index: &LiveIndex,
    classifications: &[EntryClassification],
    items: &[Recommendation],
    language: &str,
) -> Vec<Row> {
    items
        .iter()
        .filter_map(|item| {
            let class = classifications.get(item.index)?;
            let mut reason = match item.reason {
                RecommendationReason::Reclaimable(reclaim) => text(language, reclaim_key(reclaim)),
                RecommendationReason::Worktrees { count, oldest_days } => format!(
                    "{}: {count}, {oldest_days} {}",
                    text(language, "Insights.Worktrees"),
                    text(language, "Insights.Days")
                ),
                RecommendationReason::StaleExperiment { age_days } => format!(
                    "{}: {age_days} {}",
                    text(language, "Insights.Stale"),
                    text(language, "Insights.Days")
                ),
            };
            if !item.coverage_complete || !item.allocation_complete {
                reason.push_str(&format!(" · {}", text(language, "Insights.Coverage")));
            }
            Some(Row {
                key: item.index.to_string(),
                path: index.path(item.index),
                cells: vec![
                    text(language, category_key(class.category as u8)),
                    reason,
                    gb(item.known_allocated_bytes, language),
                ],
                ..Default::default()
            })
        })
        .collect()
}
pub fn git_summary(inspection: &GitInspection, language: &str) -> String {
    if inspection.repository == RepositoryState::NotRepository {
        return text(language, "Git.NotRepository");
    }
    let count = |n: Option<u64>| n.map_or_else(|| text(language, "Unknown"), |n| n.to_string());
    let mut summary = if inspection.repository == RepositoryState::Repository {
        format!(
            "Git: {} {} · {} {} · {} {} · {} {}",
            count(inspection.changed),
            text(language, "Git.Changed"),
            count(inspection.untracked),
            text(language, "Git.Untracked"),
            count(inspection.stash),
            text(language, "Git.Stash"),
            count(inspection.ahead),
            text(language, "Git.Ahead")
        )
    } else {
        text(language, "Git.Unknown")
    };
    if let Some(reason) = inspection.uncertainty {
        let key = match reason {
            GitUncertainty::GitUnavailable => "Git.Unavailable",
            GitUncertainty::UnsafeConfiguration => "Git.Unsafe",
            GitUncertainty::CommandFailed => "Git.Failed",
            GitUncertainty::TimedOut => "Git.Timeout",
            GitUncertainty::OutputLimit => "Git.OutputLimit",
            GitUncertainty::Cancelled => "Git.Cancelled",
            GitUncertainty::NoUpstream => "Git.NoUpstream",
            GitUncertainty::DetachedHead => "Git.Detached",
            GitUncertainty::UnsupportedRepository => "Git.Unsupported",
        };
        summary.push_str(&format!(" · {}", text(language, key)));
    }
    summary
}
pub fn legend() -> impl Iterator<Item = u8> {
    Category::LEGEND.into_iter().map(|category| category as u8)
}

#[cfg(test)]
pub fn target(index: usize) -> Option<&'static str> {
    crate::platform::maintenance_target(index)
}
pub fn observed(
    directories: &[DirectoryObservation],
    profile: &str,
    local: &str,
    language: &str,
) -> Vec<Row> {
    let known = [
        (
            format!("{}\\.nuget\\packages", profile.trim_end_matches('\\')),
            "NuGet",
        ),
        (
            format!("{}\\NuGet\\v3-cache", local.trim_end_matches('\\')),
            "NuGet",
        ),
        (
            format!("{}\\pip\\Cache", local.trim_end_matches('\\')),
            "pip",
        ),
    ];
    directories
        .iter()
        .filter_map(|directory| {
            let (_, program) = known.iter().find(|(path, _)| {
                diskburrow_windows::equals_path(path, directory.path.trim_end_matches(['\\', '/']))
            })?;
            Some(Row {
                path: directory.path.clone(),
                cells: vec![
                    (*program).into(),
                    crate::locale::gb(directory.logical_bytes, language),
                ],
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recommendations_only_include_observed_exact_standard_directories() {
        let paths = [
            r"E:\Profile\.nuget\packages\",
            r"e:\profile\appdata\local\PIP\Cache",
            r"E:\Profile\.nuget\packages-neighbor",
            r"E:\Profile\project\pip\Cache",
        ];
        let directories = paths
            .into_iter()
            .map(|path| DirectoryObservation {
                path: path.into(),
                logical_bytes: 1_000_000_000,
                allocated_bytes: None,
                coverage_complete: false,
            })
            .collect::<Vec<_>>();
        let rows = observed(
            &directories,
            r"E:\Profile",
            r"E:\Profile\AppData\Local",
            "en",
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].cells, ["NuGet", "1.00 GB"]);
        assert_eq!(rows[1].cells[0], "pip");
        assert!(rows.iter().all(|row| !row.selectable && !row.selected));
    }
    #[test]
    fn maintenance_targets_are_a_closed_allowlist() {
        assert_eq!(target(0), Some("ms-settings:storagesense"));
        assert!(
            target(1)
                .unwrap()
                .starts_with("https://support.google.com/")
        );
        assert!(
            target(2)
                .unwrap()
                .starts_with("https://support.microsoft.com/")
        );
        assert!(
            target(3)
                .unwrap()
                .starts_with("https://learn.microsoft.com/")
        );
        assert!(target(4).unwrap().starts_with("https://pip.pypa.io/"));
        assert!(target(5).is_none());
        assert!(target(usize::MAX).is_none());
    }
}
