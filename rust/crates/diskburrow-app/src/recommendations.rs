use crate::contract::Row;
use diskburrow_services::DirectoryObservation;

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
