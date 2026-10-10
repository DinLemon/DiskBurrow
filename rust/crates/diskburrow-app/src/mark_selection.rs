use diskburrow_windows::{is_within, normalize_local_path};
use std::collections::HashSet;

#[derive(Debug, PartialEq, Eq)]
pub enum MarkChange {
    Added { absorbed: usize },
    Removed,
    Covered { ancestor: String },
    Invalid,
}

/// Toggle an outermost mark using the existing normalized, lowercase path keys.
/// A covered descendant cannot become an independent mark or an exclusion.
/// Only `Added` and `Removed` change the selection.
pub fn toggle(marks: &mut HashSet<String>, path: &str) -> MarkChange {
    let Some(key) = normalize_local_path(path).map(|path| path.to_lowercase()) else {
        return MarkChange::Invalid;
    };
    if marks.remove(&key) {
        return MarkChange::Removed;
    }
    if let Some(ancestor) = covered_ancestor(marks, &key) {
        return MarkChange::Covered { ancestor };
    }
    let before = marks.len();
    marks.retain(|root| !is_within(root, &key));
    let absorbed = before - marks.len();
    marks.insert(key);
    MarkChange::Added { absorbed }
}

/// Return the strict outermost ancestor from a set of normalized lowercase keys.
/// Invalid inputs return `None`; an exact mark is not its own ancestor.
pub fn covered_ancestor(marks: &HashSet<String>, path: &str) -> Option<String> {
    let key = normalize_local_path(path)?.to_lowercase();
    marks
        .iter()
        .filter(|root| **root != key && is_within(&key, root))
        .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)))
        .cloned()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn marks(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    #[test]
    fn parent_absorbs_children_and_grandchildren_but_preserves_neighbors() {
        let mut selection = marks(&[
            r"c:\foo\child",
            r"c:\foo\other\grandchild",
            r"c:\foobar",
            r"d:\foo\child",
        ]);
        assert_eq!(
            toggle(&mut selection, r"C:\Foo"),
            MarkChange::Added { absorbed: 2 }
        );
        assert_eq!(
            selection,
            marks(&[r"c:\foo", r"c:\foobar", r"d:\foo\child"])
        );
    }

    #[test]
    fn covered_child_returns_parent_without_creating_an_exclusion() {
        let mut selection = marks(&[r"c:\foo", r"c:\unrelated"]);
        let before = selection.clone();
        assert_eq!(
            toggle(&mut selection, r"C:\FOO\Child\Grandchild"),
            MarkChange::Covered {
                ancestor: r"c:\foo".into()
            }
        );
        assert_eq!(selection, before);
    }

    #[test]
    fn removing_parent_then_marking_child_is_an_actual_change() {
        let mut selection = marks(&[r"c:\foo"]);
        assert_eq!(toggle(&mut selection, r"C:\FOO"), MarkChange::Removed);
        assert!(selection.is_empty());
        assert_eq!(
            toggle(&mut selection, r"C:\Foo\Child"),
            MarkChange::Added { absorbed: 0 }
        );
        assert_eq!(selection, marks(&[r"c:\foo\child"]));
    }

    #[test]
    fn sibling_prefixes_do_not_cover_or_absorb_each_other() {
        let mut selection = marks(&[r"c:\foo", r"c:\foo-archive\child", r"c:\foobar\child"]);
        assert_eq!(
            toggle(&mut selection, r"C:\Foobar"),
            MarkChange::Added { absorbed: 1 }
        );
        assert_eq!(
            selection,
            marks(&[r"c:\foo", r"c:\foo-archive\child", r"c:\foobar"])
        );
        assert_eq!(covered_ancestor(&selection, r"C:\FOO-archive"), None);
    }

    #[test]
    fn equivalent_path_spelling_toggles_the_same_lowercase_key() {
        let mut selection = HashSet::new();
        assert_eq!(
            toggle(&mut selection, "c:/Folder//Child/../Leaf/"),
            MarkChange::Added { absorbed: 0 }
        );
        assert_eq!(selection, marks(&[r"c:\folder\leaf"]));
        assert_eq!(
            toggle(&mut selection, r"C:\FOLDER\.\LEAF\\"),
            MarkChange::Removed
        );
        assert!(selection.is_empty());
    }

    #[test]
    fn unicode_case_uses_the_existing_windows_path_policy() {
        let mut selection = HashSet::new();
        assert_eq!(
            toggle(&mut selection, r"C:\ПАПКА"),
            MarkChange::Added { absorbed: 0 }
        );
        assert_eq!(selection, marks(&[r"c:\папка"]));
        assert_eq!(
            toggle(&mut selection, r"c:\папка\Дочерняя"),
            MarkChange::Covered {
                ancestor: r"c:\папка".into()
            }
        );
        assert_eq!(toggle(&mut selection, r"c:\Папка"), MarkChange::Removed);
    }

    #[test]
    fn drive_root_absorbs_only_its_own_drive() {
        let mut selection = marks(&[r"c:\foo", r"c:\bar\child", r"d:\foo"]);
        assert_eq!(
            toggle(&mut selection, "C:/"),
            MarkChange::Added { absorbed: 2 }
        );
        assert_eq!(selection, marks(&["c:\\", r"d:\foo"]));
        assert_eq!(
            toggle(&mut selection, r"C:\Another"),
            MarkChange::Covered {
                ancestor: "c:\\".into()
            }
        );
        assert_eq!(covered_ancestor(&selection, r"D:\Another"), None);
        assert_eq!(toggle(&mut selection, "c:\\"), MarkChange::Removed);
        assert_eq!(selection, marks(&[r"d:\foo"]));
    }

    #[test]
    fn invalid_paths_leave_the_selection_unchanged() {
        let mut selection = marks(&[r"c:\foo"]);
        let before = selection.clone();
        for path in [
            "",
            "relative",
            "C:",
            r"C:foo",
            r"\foo",
            r"\\server\share\foo",
            r"\\?\C:\foo",
            r"\\.\C:\foo",
            r"C:\..\foo",
            r"C:\foo:stream",
            r"C:\CON.txt",
            r"C:\LPT1",
            r"C:\foo.",
            "C:\\foo ",
            "C:\\bad\nname",
            r"C:\bad*name",
            r"1:\foo",
        ] {
            assert_eq!(
                toggle(&mut selection, path),
                MarkChange::Invalid,
                "{path:?}"
            );
            assert_eq!(selection, before, "{path:?}");
            assert_eq!(covered_ancestor(&selection, path), None, "{path:?}");
        }
    }

    #[test]
    fn ancestor_lookup_is_strict_and_normalizes_its_input() {
        let selection = marks(&[r"c:\foo"]);
        assert_eq!(covered_ancestor(&selection, r"C:\FOO"), None);
        assert_eq!(
            covered_ancestor(&selection, "C:/FOO//Child/../Leaf/"),
            Some(r"c:\foo".into())
        );
        assert_eq!(covered_ancestor(&selection, r"C:\Foobar\Child"), None);
    }

    #[test]
    fn ancestor_lookup_reports_outermost_parent_when_existing_marks_overlap() {
        let selection = marks(&["c:\\", r"c:\foo", r"c:\foo\child"]);
        assert_eq!(
            covered_ancestor(&selection, r"C:\Foo\Child\Leaf"),
            Some("c:\\".into())
        );
    }
}
