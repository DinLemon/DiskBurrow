use diskburrow_engine::{AgeBand, LiveIndex, MapMetric, MapProjection, OTHERS_INDEX};
use disktree_core::tree::{Node, NodeKind};

fn file(name: &str, logical: u64, allocated: u64) -> Node {
    let mut node = Node::entry(name, NodeKind::File, allocated);
    node.logical = logical;
    node
}

fn hidden_fixture() -> LiveIndex {
    let mut root = Node::directory("root");
    let mut public = Node::directory("public");
    public.children = vec![file("needle.txt", 10, 4), file("other.txt", 20, 8)];
    let mut dot = Node::directory(".secret");
    dot.children = vec![file("needle.txt", 100, 40)];
    let mut hidden = Node::directory("hidden");
    hidden.attributes = 0x2;
    let mut nested = Node::directory("nested");
    nested.children = vec![file("needle.txt", 1000, 400)];
    hidden.children = vec![nested];
    let mut system = Node::directory("system");
    system.attributes = 0x4;
    system.children = vec![file("needle.txt", 10000, 4000)];
    root.children = vec![public, dot, hidden, system, file(".dotfile", 5, 2)];
    LiveIndex::from_tree(r"E:\owned".into(), root).unwrap()
}

fn projection(depth: u32, show_hidden: bool) -> MapProjection {
    MapProjection { depth, show_hidden }
}

fn deep_fixture() -> LiveIndex {
    let mut node = file("leaf", 1, 1);
    for name in [
        "level7", "level6", "level5", "level4", "level3", "level2", "level1",
    ] {
        let mut parent = Node::directory(name);
        parent.children = vec![node];
        node = parent;
    }
    let mut root = Node::directory("root");
    root.children = vec![node];
    LiveIndex::from_tree(r"E:\owned".into(), root).unwrap()
}

#[test]
fn projection_draws_requested_levels_and_clamps_invalid_depth() {
    let live = deep_fixture();
    for (depth, expected) in [(0, 1), (1, 1), (3, 3), (6, 6), (7, 6), (u32::MAX, 6)] {
        let tiles = live.layout_with_projection(
            0,
            (1000.0, 1000.0),
            MapMetric::Logical,
            "",
            false,
            false,
            projection(depth, true),
        );
        assert_eq!(tiles.len(), expected, "depth={depth}");
        assert_eq!(tiles.last().unwrap().depth as usize, expected - 1);
    }
}

#[test]
fn legacy_layout_remains_three_levels_and_shows_hidden() {
    for live in [deep_fixture(), hidden_fixture()] {
        assert_eq!(
            live.layout(0, (1000.0, 1000.0), MapMetric::Logical, "", false, false),
            live.layout_with_projection(
                0,
                (1000.0, 1000.0),
                MapMetric::Logical,
                "",
                false,
                false,
                MapProjection::default(),
            ),
        );
    }
    assert_eq!(
        deep_fixture()
            .layout(0, (1000.0, 1000.0), MapMetric::Logical, "", false, false)
            .len(),
        3
    );
}

#[test]
fn hidden_ancestors_remove_descendants_for_every_metric_without_mutation() {
    let live = hidden_fixture();
    let original = live.clone();
    for index in 0..live.entries.len() {
        assert_eq!(live.visible(index, false), index < 4, "index={index}");
        assert!(live.visible(index, true));
    }
    assert!(!live.visible(usize::MAX, true));
    for (metric, full, visible) in [
        (MapMetric::Logical, 11135, 30),
        (MapMetric::Allocated, 4454, 12),
        (MapMetric::Files, 6, 2),
    ] {
        let weights = live.projected_weights_with_projection(
            0,
            metric,
            "",
            false,
            false,
            projection(6, false),
        );
        assert_eq!(weights[0], visible);
        assert_eq!(weights[1], visible);
        assert!(weights[4..].iter().all(|weight| *weight == 0));
        assert_eq!(live.projected_weights(0, metric, "", false, false)[0], full);
        let tiles = live.layout_with_projection(
            0,
            (1000.0, 1000.0),
            metric,
            "",
            false,
            false,
            projection(6, false),
        );
        assert!(tiles.iter().all(|tile| tile.index < 4));
        assert_eq!(tiles[0].index, 1);
        assert_eq!(tiles[0].width * tiles[0].height, 1_000_000.0);
    }
    assert_eq!(live, original);
}

#[test]
fn hidden_matches_do_not_leak_through_search_or_isolation() {
    let live = hidden_fixture();
    let filtered = projection(6, false);
    assert_eq!(
        live.search_with_projection(0, "needle", true, 1000, filtered)
            .indices,
        vec![2]
    );
    assert_eq!(
        live.search_with_projection(0, "needle", true, 0, filtered)
            .total,
        1
    );
    assert_eq!(
        live.search_with_projection(0, "", false, 1000, filtered)
            .indices,
        vec![1]
    );
    assert_eq!(
        live.search_with_projection(0, "hidden/", true, 1000, filtered)
            .total,
        0
    );
    assert_eq!(
        live.search_with_projection(4, "needle", false, 1000, filtered)
            .total,
        0
    );
    for (query, expected) in [
        ("needle", 10),
        ("public", 30),
        ("hidden", 0),
        (".secret", 0),
    ] {
        let weights = live.projected_weights_with_projection(
            0,
            MapMetric::Logical,
            query,
            true,
            true,
            filtered,
        );
        assert_eq!(weights[0], expected, "query={query}");
        assert!(weights[4..].iter().all(|weight| *weight == 0));
    }
    assert!(
        live.layout_with_projection(
            4,
            (1000.0, 1000.0),
            MapMetric::Logical,
            "",
            false,
            true,
            filtered
        )
        .is_empty()
    );
    assert_eq!(live.search(0, "needle", true, 1000).total, 4);
}

#[test]
fn hidden_root_attributes_and_dot_path_ancestors_are_respected() {
    for (root_path, attributes) in [
        (r"E:\owned", 0x2),
        (r"E:\owned", 0x4),
        (r"E:\.parent\owned", 0),
        (r"E:\owned\.root", 0),
    ] {
        let mut live = hidden_fixture();
        live.root = root_path.into();
        live.entries[0].attributes = attributes;
        assert!(!live.visible(2, false));
        assert!(live.visible(2, true));
        assert_eq!(
            live.search_with_projection(0, "", false, 1000, projection(3, false))
                .total,
            0
        );
        assert_eq!(
            live.projected_weights_with_projection(
                0,
                MapMetric::Logical,
                "",
                false,
                false,
                projection(3, false)
            )[0],
            0
        );
    }
}

#[test]
fn hidden_tail_weights_do_not_enter_the_others_tile() {
    let mut root = Node::directory("root");
    root.children = (0..200).map(|i| file(&format!("file{i}"), 1, 1)).collect();
    root.children.push(file(".large", 10000, 10000));
    let live = LiveIndex::from_tree(r"E:\owned".into(), root).unwrap();
    let tiles = live.layout_with_projection(
        0,
        (1000.0, 1000.0),
        MapMetric::Logical,
        "",
        false,
        false,
        projection(1, false),
    );
    assert_eq!(tiles.len(), 96);
    let others = tiles
        .iter()
        .find(|tile| tile.index == OTHERS_INDEX)
        .unwrap();
    assert!((others.width * others.height - 525_000.0).abs() < 20.0);
    assert!(tiles.iter().all(|tile| tile.index != 201));
}

#[test]
fn invalid_projection_bounds_and_scope_return_no_tiles() {
    let live = deep_fixture();
    for (scope, bounds) in [
        (usize::MAX, (1.0, 1.0)),
        (8, (1.0, 1.0)),
        (0, (f32::NAN, 1.0)),
        (0, (1.0, f32::INFINITY)),
        (0, (0.0, 1.0)),
        (0, (1.0, -1.0)),
    ] {
        assert!(
            live.layout_with_projection(
                scope,
                bounds,
                MapMetric::Logical,
                "",
                false,
                false,
                projection(u32::MAX, false)
            )
            .is_empty()
        );
    }
}

#[test]
fn age_bands_use_inclusive_exact_last_write_thresholds() {
    let now = 2_000_000_000;
    for (elapsed, expected) in [
        (0, AgeBand::Within7Days),
        (604800, AgeBand::Within7Days),
        (604801, AgeBand::Within30Days),
        (2592000, AgeBand::Within30Days),
        (2592001, AgeBand::Within180Days),
        (15552000, AgeBand::Within180Days),
        (15552001, AgeBand::Within365Days),
        (31536000, AgeBand::Within365Days),
        (31536001, AgeBand::Older),
    ] {
        assert_eq!(
            AgeBand::from_modified(now - elapsed, now),
            expected,
            "elapsed={elapsed}"
        );
    }
}

#[test]
fn unknown_future_and_invalid_times_are_never_old_files() {
    let now = 2_000_000_000;
    for (modified, clock) in [
        (0, now),
        (-1, now),
        (now + 1, now),
        (i64::MIN, now),
        (i64::MAX, now),
        (1, i64::MAX),
        (1, 0),
        (1, i64::MIN),
    ] {
        assert_eq!(AgeBand::from_modified(modified, clock), AgeBand::Unknown);
    }
}
