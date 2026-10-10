use crate::{ManualDeletePlan, NativeFileObservation, normalize_local_path};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ManualReclaimProjection {
    /// Allocation credited once per known physical identity, excluding directories.
    pub known_reclaim_bytes: i64,
    pub reclaimable_files: usize,
    /// Identities observed with multiple hardlinks, even when all aliases are selected.
    pub excluded_hardlink_files: usize,
    /// Missing identity/allocation/link count, invalid paths, or conflicting observations.
    pub excluded_unknown_files: usize,
    pub excluded_foreign_files: usize,
}

/// A read-only projection, not deletion authority or a promise of actual volume change.
/// `target_volume` is the native `FileIdentity.volume` serial, not a drive letter.
/// Counts identify reviewed identity groups; missing identities are counted by unique path.
/// Multiple hardlinks are excluded because execution retains the reviewed link count per name:
/// deleting the first name changes that count and prevents deletion of a later reviewed name.
pub fn project_manual_reclaim(
    plan: &ManualDeletePlan,
    target_volume: u64,
) -> ManualReclaimProjection {
    let mut result = ManualReclaimProjection::default();
    let mut groups: BTreeMap<(u64, String), Vec<&NativeFileObservation>> = BTreeMap::new();
    let mut missing_identity = BTreeSet::new();
    let mut path_identities: BTreeMap<_, BTreeSet<_>> = BTreeMap::new();
    for entry in plan.entries.iter().filter(|e| !e.is_directory()) {
        let file = &entry.file;
        let path = normalize_local_path(&file.path)
            .unwrap_or_else(|| file.path.clone())
            .to_lowercase();
        let key = file
            .identity
            .as_ref()
            .map(|id| (id.volume, id.file_id.clone()));
        path_identities
            .entry(path.clone())
            .or_default()
            .insert(key.clone());
        if let Some(key) = key {
            groups.entry(key).or_default().push(file);
        } else {
            missing_identity.insert(path);
        }
    }
    result.excluded_unknown_files = missing_identity.len();
    for ((volume, id), observations) in groups {
        if volume != target_volume {
            result.excluded_foreign_files += 1;
            continue;
        }
        // Do not credit a complete selected link set: strict executor revalidation will
        // reject a later name after the first deletion changes the reviewed link count.
        if observations.iter().any(|file| file.link_count > 1) {
            result.excluded_hardlink_files += 1;
            continue;
        }
        let first = observations[0];
        let mut paths = BTreeSet::new();
        let consistent = observations.iter().all(|file| {
            let Some(path) = normalize_local_path(&file.path).map(|p| p.to_lowercase()) else {
                return false;
            };
            paths.insert(path.clone());
            path_identities[&path].len() == 1
                && file.allocated_bytes == first.allocated_bytes
                && file.link_count == first.link_count
        });
        let Some(allocation) = first.allocated_bytes.filter(|bytes| *bytes >= 0) else {
            result.excluded_unknown_files += 1;
            continue;
        };
        if id.is_empty()
            || !consistent
            || first.link_count <= 0
            || paths.len() > first.link_count as usize
        {
            result.excluded_unknown_files += 1;
        } else if let Some(total) = result.known_reclaim_bytes.checked_add(allocation) {
            result.known_reclaim_bytes = total;
            result.reclaimable_files += 1;
        } else {
            result.excluded_unknown_files += 1;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FileIdentity, ManualDeleteEntry, NativeFileObservation};
    use chrono::Utc;
    use uuid::Uuid;

    fn file(path: &str, id: &str, bytes: Option<i64>, links: i32) -> ManualDeleteEntry {
        ManualDeleteEntry {
            id: Uuid::new_v4(),
            root_path: r"E:\selected".into(),
            file: NativeFileObservation {
                path: path.into(),
                identity: Some(FileIdentity {
                    volume: 42,
                    file_id: id.into(),
                }),
                logical_bytes: 99,
                allocated_bytes: bytes,
                modified_utc: Utc::now(),
                link_count: links,
                attributes: 0,
            },
        }
    }

    fn plan(entries: Vec<ManualDeleteEntry>) -> ManualDeletePlan {
        ManualDeletePlan {
            id: Uuid::new_v4(),
            created_utc: Utc::now(),
            roots: vec![r"E:\selected".into()],
            entries,
            warnings: vec![],
        }
    }

    #[test]
    fn unique_allocation_ignores_directory_aggregate_and_duplicate_paths() {
        let a = file(r"E:\selected\a", "a", Some(4096), 1);
        let mut duplicate = a.clone();
        duplicate.file.path = r"e:\SELECTED\a".into();
        let mut directory = file(r"E:\selected", "directory", Some(900_000), 1);
        directory.file.attributes = 0x10;
        let estimate = project_manual_reclaim(
            &plan(vec![
                a,
                duplicate,
                directory,
                file(r"E:\selected\b", "b", Some(8192), 1),
            ]),
            42,
        );
        assert_eq!(estimate.known_reclaim_bytes, 12_288);
        assert_eq!(estimate.reclaimable_files, 2);
        assert_eq!(estimate.excluded_unknown_files, 0);
    }

    #[test]
    fn complete_and_external_hardlinks_are_excluded_under_strict_executor_revalidation() {
        let selected = vec![
            file(r"E:\selected\a", "linked", Some(4096), 2),
            file(r"E:\selected\b", "linked", Some(4096), 2),
            file(r"E:\selected\external", "external", Some(8192), 2),
        ];
        let estimate = project_manual_reclaim(&plan(selected), 42);
        assert_eq!(estimate.known_reclaim_bytes, 0);
        assert_eq!(estimate.reclaimable_files, 0);
        assert_eq!(estimate.excluded_hardlink_files, 2);
        let a = file(r"E:\selected\a", "linked", Some(4096), 2);
        let estimate = project_manual_reclaim(&plan(vec![a.clone(), a]), 42);
        assert_eq!(estimate.known_reclaim_bytes, 0);
        assert_eq!(estimate.excluded_hardlink_files, 1);
    }

    #[test]
    fn unknown_allocation_identity_or_link_count_and_foreign_files_are_excluded() {
        let mut missing = file(r"E:\selected\noidentity", "x", Some(4096), 1);
        missing.file.identity = None;
        let mut foreign = file(r"F:\other", "other", Some(900_000), 1);
        foreign.file.identity.as_mut().unwrap().volume = 17;
        let estimate = project_manual_reclaim(
            &plan(vec![
                missing,
                file(r"E:\selected\noallocation", "na", None, 1),
                file(r"E:\selected\nolinks", "nl", Some(8192), 0),
                file(r"E:\selected\negative", "n", Some(-1), 1),
                foreign,
            ]),
            42,
        );
        assert_eq!(estimate.known_reclaim_bytes, 0);
        assert_eq!(estimate.excluded_unknown_files, 4);
        assert_eq!(estimate.excluded_foreign_files, 1);
    }

    #[test]
    fn inconsistent_identity_observations_do_not_claim_allocation() {
        let estimate = project_manual_reclaim(
            &plan(vec![
                file(r"E:\selected\a", "same", Some(4096), 1),
                file(r"E:\selected\a", "same", Some(8192), 1),
                file(r"E:\selected\c", "links", Some(4096), 1),
                file(r"E:\selected\c", "links", Some(4096), 0),
            ]),
            42,
        );
        assert_eq!(estimate.known_reclaim_bytes, 0);
        assert_eq!(estimate.excluded_unknown_files, 2);
    }

    #[test]
    fn one_path_cannot_credit_conflicting_identities_or_duplicate_unknown_files() {
        let mut missing = file(r"E:\selected\missing", "unused", Some(4096), 1);
        missing.file.identity = None;
        let estimate = project_manual_reclaim(
            &plan(vec![
                file(r"E:\selected\a", "first", Some(4096), 1),
                file(r"e:\SELECTED\a", "second", Some(8192), 1),
                missing.clone(),
                missing,
            ]),
            42,
        );
        assert_eq!(estimate.known_reclaim_bytes, 0);
        assert_eq!(estimate.excluded_unknown_files, 3);
    }

    #[test]
    fn invalid_link_inventory_and_overflow_never_inflate_or_wrap_known_bytes() {
        let estimate = project_manual_reclaim(
            &plan(vec![
                file(r"E:\selected\a", "one", Some(i64::MAX), 1),
                file(r"E:\selected\b", "two", Some(1), 1),
                file(r"E:\selected\c", "contradiction", Some(4096), 1),
                file(r"E:\selected\d", "contradiction", Some(4096), 1),
                file(r"\\server\share\file", "unc", Some(4096), 1),
                file(r"E:\selected\zero", "zero", Some(0), 1),
            ]),
            42,
        );
        assert_eq!(estimate.known_reclaim_bytes, i64::MAX);
        assert_eq!(estimate.reclaimable_files, 2);
        assert_eq!(estimate.excluded_unknown_files, 3);
    }
}
