use chrono::Utc;
use diskburrow_engine::protocol::*;
use diskburrow_engine::*;
use disktree_core::tree::{Node, NodeKind};
use std::io::Cursor;
fn tree() -> Node {
    let mut root = Node::directory("root");
    let mut a = Node::directory("A");
    a.logical = 1000;
    a.bytes = 1000;
    a.files = 2;
    a.children = vec![
        Node::entry("needle.txt", NodeKind::File, 10),
        Node::entry("other.bin", NodeKind::File, 90),
    ];
    let mut b = Node::directory("B");
    b.logical = 100;
    b.bytes = 100;
    b.files = 1;
    b.children = vec![Node::entry("Needle.bin", NodeKind::File, 100)];
    root.children = vec![a, b];
    root.logical = 1100;
    root.bytes = 1100;
    root.files = 3;
    root
}
#[test]
fn flatten_keeps_all_leaves_and_paths_without_logical_allocated_drift() {
    let mut tree = tree();
    tree.children[0].children[0].logical = 10000;
    tree.children[0].children[0].bytes = 4096;
    let index = LiveIndex::from_tree(r"C:\".into(), tree).unwrap();
    assert_eq!(index.entries.len(), 6);
    assert_eq!(index.entries[2].logical, 10000);
    assert_eq!(index.entries[2].allocated, Some(4096));
    assert_eq!(index.path(2), r"C:\A\needle.txt");
    assert_eq!(index.descendants(1), vec![2, 3]);
    assert_eq!(index.entries[4].parent, Some(0));
}
#[test]
fn denied_cloud_and_reparse_propagate_coverage_to_ancestors() {
    let mut tree = tree();
    tree.children[0].children[0].attributes = 0x00400000;
    tree.children[1].read_error = true;
    let index = LiveIndex::from_tree(r"C:\".into(), tree).unwrap();
    assert!(!index.entries[0].coverage);
    assert!(!index.entries[1].coverage);
    assert!(!index.entries[2].coverage);
    assert!(index.entries[3].coverage);
    assert!(!index.entries[4].coverage);
    let snapshot = index.snapshot(Utc::now(), Utc::now());
    assert!(snapshot.traversal_completed);
    assert!(
        snapshot
            .issues
            .iter()
            .any(|i| i.kind == diskburrow_services::ScanIssueKind::CloudSkipped)
    );
}
#[test]
fn resident_and_entry_limits_reject_without_following_paths() {
    assert!(LiveIndex::from_tree_with_limits(r"C:\".into(), tree(), 3, 1 << 30).is_err());
    assert!(LiveIndex::from_tree_with_limits(r"C:\".into(), tree(), 20_000_000, 100).is_err());
    let mut too_big = tree();
    too_big.children[0].children[0].logical = u64::MAX;
    assert!(LiveIndex::from_tree(r"C:\".into(), too_big).is_err());
    assert!(LiveIndex::from_tree("relative".into(), tree()).is_err());
}
#[test]
fn snapshot_is_complete_directories_with_bounded_largest_files() {
    let mut tree = Node::directory("root");
    tree.children = (0..150)
        .map(|i| Node::entry(i.to_string(), NodeKind::File, i))
        .collect();
    let index = LiveIndex::from_tree(r"C:\".into(), tree).unwrap();
    let snapshot = index.snapshot(Utc::now(), Utc::now());
    assert_eq!(index.entries.len(), 151);
    assert_eq!(snapshot.largest_files.len(), 100);
    assert_eq!(snapshot.largest_files[0].logical_bytes, 149);
    assert_eq!(snapshot.directories.len(), 1);
}
#[test]
fn search_global_scope_and_isolate_keep_only_matched_leaf_weights() {
    let index = LiveIndex::from_tree(r"C:\".into(), tree()).unwrap();
    let local = index.search(1, "NEEDLE", false, 1000);
    assert_eq!(local.indices, vec![2]);
    let global = index.search(1, "needle", true, 1000);
    assert_eq!(global.indices, vec![2, 5]);
    assert_eq!(global.total, 2);
    assert_eq!(index.search(0, "A\\needle", true, 1000).indices, vec![2]);
    let weights = index.projected_weights(0, MapMetric::Logical, "needle", true, true);
    assert_eq!(weights[0], 110);
    assert_eq!(weights[1], 10);
    assert_eq!(weights[3], 0);
    let whole = index.projected_weights(0, MapMetric::Logical, "A", true, true);
    assert_eq!(whole[1], 100);
    assert_eq!(whole[4], 0);
}
#[test]
fn map_metrics_unknown_allocation_zero_entries_and_tail_are_preserved() {
    let mut tree = Node::directory("root");
    tree.children = (0..200)
        .map(|i| Node::entry(format!("file{i}"), NodeKind::File, 1))
        .collect();
    tree.children[0].allocation_known = false;
    let index = LiveIndex::from_tree(r"C:\".into(), tree).unwrap();
    assert_eq!(
        index.projected_weights(0, MapMetric::Allocated, "", false, true)[0],
        199
    );
    assert_eq!(
        index.projected_weights(0, MapMetric::Files, "", false, true)[0],
        200
    );
    let tiles = index.layout(0, (1000.0, 1000.0), MapMetric::Logical, "", false, true);
    assert!(tiles.len() <= 96);
    assert!(tiles.iter().any(|t| t.index == OTHERS_INDEX));
    let area: f32 = tiles.iter().map(|t| t.width * t.height).sum();
    assert!((area - 1_000_000.0).abs() < 20.0);
    assert_eq!(index.entries.len(), 201);
    assert!(
        index
            .layout(0, (f32::NAN, 10.0), MapMetric::Logical, "", false, true)
            .is_empty()
    );
}
#[test]
fn navigation_back_forward_up_and_file_focus_have_separate_histories() {
    let index = LiveIndex::from_tree(r"C:\".into(), tree()).unwrap();
    let mut nav = MapNavigation::new();
    nav.navigate(&index, 1);
    nav.navigate(&index, 4);
    nav.back();
    assert_eq!(nav.current, 1);
    nav.forward();
    assert_eq!(nav.current, 4);
    nav.up(&index);
    assert_eq!(nav.current, 0);
    nav.navigate(&index, 2);
    assert_eq!(nav.current, 1);
    assert_eq!(nav.focused, Some(2));
    nav.back();
    assert_eq!(nav.current, 0);
    nav.navigate(&index, 4);
    assert!(!nav.can_forward());
}
#[test]
fn helper_nonce_and_same_user_are_required() {
    let nonce = "a".repeat(64);
    let mut wire = vec![];
    write_hello(&mut wire, &nonce, r"C:\").unwrap();
    read_hello(&mut Cursor::new(&wire), &nonce, r"C:\").unwrap();
    assert!(read_hello(&mut Cursor::new(&wire), &"b".repeat(64), r"C:\").is_err());
    assert!(read_hello(&mut Cursor::new(&wire), &nonce, r"D:\").is_err());
    assert!(validate_peer("S-1-5-21-1", "S-1-5-21-2", &nonce, &nonce).is_err());
    assert!(validate_peer("S-1-5-21-1", "S-1-5-21-1", &nonce, &nonce).is_ok());
}
#[test]
fn protocol_roundtrip_and_every_truncation_are_bounded() {
    let index = LiveIndex::from_tree(r"C:\".into(), tree()).unwrap();
    let mut wire = vec![];
    write_index(&mut wire, &index).unwrap();
    assert_eq!(read_index(&mut Cursor::new(&wire), r"C:\").unwrap(), index);
    assert!(read_index(&mut Cursor::new(&wire), r"D:\").is_err());
    for end in 0..wire.len() {
        assert!(
            read_index(&mut Cursor::new(&wire[..end]), r"C:\").is_err(),
            "Accepted truncation {end}"
        );
    }
    assert!(
        read_index_with_limits(&mut Cursor::new(&wire), r"C:\", 10, 20_000_000, 1 << 30).is_err()
    );
    assert!(read_index_with_limits(&mut Cursor::new(&wire), r"C:\", 1 << 30, 2, 1 << 30).is_err());
    assert!(
        read_index_with_limits(&mut Cursor::new(&wire), r"C:\", 1 << 30, 20_000_000, 100).is_err()
    );
}
#[test]
fn protocol_rejects_bad_parent_name_negative_quantity_and_duplicate_children() {
    let original = LiveIndex::from_tree(r"C:\".into(), tree()).unwrap();
    for change in 0..5 {
        let mut invalid = original.clone();
        match change {
            0 => invalid.entries[2].parent = Some(5),
            1 => invalid.entries[2].name = "..".into(),
            2 => invalid.entries[2].name = "a\\b".into(),
            3 => invalid.entries[2].logical = -1,
            _ => invalid.entries[0].children.push(1),
        };
        assert!(write_index(&mut vec![], &invalid).is_err());
    }
}

#[test]
fn helper_failure_frame_is_utf8_and_bounded_before_allocation() {
    use diskburrow_engine::protocol::{read_error, write_error};
    let mut wire = vec![];
    write_error(&mut wire, "FastScan.ReadFailed").unwrap();
    assert_eq!(
        read_error(&mut Cursor::new(&wire)).unwrap(),
        "FastScan.ReadFailed"
    );
    assert!(write_error(&mut vec![], &"x".repeat(4097)).is_err());
    assert!(read_error(&mut Cursor::new(u32::MAX.to_le_bytes())).is_err());
    for end in 0..wire.len() {
        assert!(read_error(&mut Cursor::new(&wire[..end])).is_err());
    }
    assert!(read_error(&mut Cursor::new([1, 0, 0, 0, 255])).is_err());
}

#[test]
fn snapshot_does_not_report_io_or_depth_limit_as_access_denied() {
    for (code, expected) in [
        (1, diskburrow_services::ScanIssueKind::AccessDenied),
        (2, diskburrow_services::ScanIssueKind::IoFailure),
        (4, diskburrow_services::ScanIssueKind::IoFailure),
    ] {
        let mut tree = tree();
        tree.children[1].read_error = true;
        tree.children[1].read_error_kind = code;
        let index = LiveIndex::from_tree(r"C:\".into(), tree).unwrap();
        let snapshot = index.snapshot(Utc::now(), Utc::now());
        assert!(
            snapshot
                .issues
                .iter()
                .any(|issue| issue.path == r"C:\B" && issue.kind == expected)
        );
    }
}
