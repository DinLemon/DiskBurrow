use diskburrow_engine::{Category, LiveIndex, Reclaim, RecommendationReason};
use disktree_core::tree::{Metric, Node, NodeKind, aggregate};

const MIB: u64 = 1024 * 1024;
const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

fn file(name: &str, bytes: u64, days: i64) -> Node {
    let mut node = Node::entry(name, NodeKind::File, bytes);
    node.modified = NOW - days * DAY;
    node
}

fn dir(name: &str, children: Vec<Node>) -> Node {
    let mut node = Node::directory(name);
    node.children = children;
    node
}

fn index(children: Vec<Node>) -> LiveIndex {
    let mut root = dir("root", children);
    aggregate(&mut root, Metric::Bytes);
    LiveIndex::from_tree(r"E:\GeneratedAnalytics".into(), root).unwrap()
}

fn named(index: &LiveIndex, name: &str) -> usize {
    index
        .entries
        .iter()
        .position(|entry| entry.name.as_ref() == name)
        .unwrap()
}

// Removing inheritance would leave the arbitrary descendant and file unclassified.
#[test]
fn cache_categories_and_reclaim_reasons_inherit_through_arbitrary_descendants() {
    let index = index(vec![dir(
        ".cache",
        vec![dir("objects", vec![file("blob", 128 * MIB, 1)])],
    )]);
    let classes = index.classifications();
    for name in [".cache", "objects", "blob"] {
        assert_eq!(classes[named(&index, name)].category, Category::Cache);
        assert_eq!(
            classes[named(&index, name)].reclaim,
            Some(Reclaim::Regenerable)
        );
    }
    let recommendations = index.recommendations(NOW, 10);
    assert_eq!(recommendations.len(), 1);
    assert_eq!(recommendations[0].index, named(&index, ".cache"));
    assert_eq!(recommendations[0].known_allocated_bytes, 128 * MIB as i64);
    assert_eq!(recommendations[0].logical_bytes, 128 * MIB as i64);
}

// Trusting target/node_modules without their manifest would suggest unrelated data.
#[test]
fn contextual_build_and_dependency_names_require_indexed_siblings_case_insensitively() {
    let index = index(vec![
        dir(
            "rust",
            vec![
                file("CARGO.TOML", 1, 1),
                dir("TARGET", vec![file("object", 128 * MIB, 1)]),
            ],
        ),
        dir(
            "js",
            vec![
                file("PACKAGE.JSON", 1, 1),
                dir("node_modules", vec![file("dependency", 128 * MIB, 1)]),
            ],
        ),
        dir(
            "unrelated",
            vec![
                dir("target", vec![file("notes", 128 * MIB, 1)]),
                dir("NODE_MODULES", vec![]),
            ],
        ),
    ]);
    let classes = index.classifications();
    assert_eq!(
        classes[named(&index, "TARGET")].reclaim,
        Some(Reclaim::BuildOutput)
    );
    assert_eq!(
        classes[named(&index, "object")].reclaim,
        Some(Reclaim::BuildOutput)
    );
    assert_eq!(
        classes[named(&index, "node_modules")].reclaim,
        Some(Reclaim::Reinstallable)
    );
    assert_eq!(classes[named(&index, "target")].reclaim, None);
    assert_eq!(classes[named(&index, "NODE_MODULES")].reclaim, None);
    assert_eq!(index.recommendations(NOW, 10).len(), 2);
}

// Ignoring parent category would allow a documents directory's layers to be reclaimable.
#[test]
fn sandbox_layers_require_agent_context_and_files_never_classify_by_their_own_names() {
    let index = index(vec![
        dir(
            ".codex",
            vec![dir("layers", vec![file("layer", 128 * MIB, 1)])],
        ),
        dir(
            "Documents",
            vec![
                dir("snapshots", vec![file("photo", 128 * MIB, 1)]),
                file("cache", 128 * MIB, 1),
            ],
        ),
    ]);
    let classes = index.classifications();
    assert_eq!(
        classes[named(&index, "layer")].category,
        Category::AgentScratch
    );
    assert_eq!(
        classes[named(&index, "layers")].reclaim,
        Some(Reclaim::SandboxLayers)
    );
    assert_eq!(classes[named(&index, "snapshots")].reclaim, None);
    assert_eq!(
        classes[named(&index, "cache")].category,
        Category::Documents
    );
    assert_eq!(index.recommendations(NOW, 10).len(), 1);
}

// Selecting the experiments parent would include fresh work and overlap its cache finding.
#[test]
fn stale_experiments_are_individual_roots_and_suppress_their_nested_cache() {
    let index = index(vec![dir(
        "experiments",
        vec![
            dir(
                "old",
                vec![dir("cache", vec![file("stale", 256 * MIB, 90)])],
            ),
            dir(
                "fresh",
                vec![
                    file("Cargo.toml", 1, 1),
                    dir("target", vec![file("new", 128 * MIB, 1)]),
                ],
            ),
        ],
    )]);
    let recommendations = index.recommendations(NOW, 10);
    assert_eq!(recommendations.len(), 2);
    assert_eq!(recommendations[0].index, named(&index, "old"));
    assert_eq!(
        recommendations[0].reason,
        RecommendationReason::StaleExperiment { age_days: 90 }
    );
    assert_eq!(recommendations[1].index, named(&index, "target"));
    assert_eq!(
        recommendations
            .iter()
            .map(|entry| entry.known_allocated_bytes)
            .sum::<i64>(),
        384 * MIB as i64
    );
}

// Descending after recommending worktrees would double-count their caches.
#[test]
fn worktrees_suppress_descendants_and_report_oldest_known_age() {
    let index = index(vec![dir(
        "worktrees",
        vec![
            dir(
                "first",
                vec![dir("cache", vec![file("one", 128 * MIB, 41)])],
            ),
            dir("second", vec![file("two", 128 * MIB, 3)]),
        ],
    )]);
    let recommendations = index.recommendations(NOW, 10);
    assert_eq!(recommendations.len(), 1);
    assert_eq!(
        recommendations[0].reason,
        RecommendationReason::Worktrees {
            count: 2,
            oldest_days: 41
        }
    );
    assert_eq!(recommendations[0].known_allocated_bytes, 256 * MIB as i64);
}

// Summing a directory total or treating an unknown leaf as known overstates observed allocation.
#[test]
fn incomplete_coverage_and_allocation_are_explicit_and_only_known_leaves_count() {
    let mut unknown = file("unknown", 256 * MIB, 1);
    unknown.allocation_known = false;
    let mut cache = dir("cache", vec![file("known", 128 * MIB, 1), unknown]);
    cache.read_error = true;
    let mut index = index(vec![cache]);
    let cache = named(&index, "cache");
    index.entries[cache].allocated = Some(900 * MIB as i64);
    let recommendations = index.recommendations(NOW, 10);
    assert_eq!(recommendations.len(), 1);
    assert_eq!(recommendations[0].known_allocated_bytes, 128 * MIB as i64);
    assert_eq!(recommendations[0].logical_bytes, 384 * MIB as i64);
    assert!(!recommendations[0].coverage_complete);
    assert!(!recommendations[0].allocation_complete);
}

// Dropping unknown-allocation findings would hide large observed caches; unsafe entries stay excluded.
#[test]
fn unknown_allocation_still_has_an_informational_finding_but_cloud_roots_do_not() {
    let mut unknown = file("unknown", 128 * MIB, 1);
    unknown.allocation_known = false;
    let mut cloud = dir("temp", vec![file("cloud", 512 * MIB, 1)]);
    cloud.attributes = 0x400000;
    let index = index(vec![dir("cache", vec![unknown]), cloud]);
    let recommendations = index.recommendations(NOW, 10);
    assert_eq!(recommendations.len(), 1);
    assert_eq!(recommendations[0].index, named(&index, "cache"));
    assert_eq!(recommendations[0].known_allocated_bytes, 0);
    assert!(recommendations[0].coverage_complete);
    assert!(!recommendations[0].allocation_complete);
}

// An unbounded collector or wrong ranking would ignore the output cap or retain small entries.
#[test]
fn recommendations_keep_largest_roots_with_a_hard_cap_and_skip_tiny_caches() {
    let mut children: Vec<_> = (0..150)
        .map(|i| {
            dir(
                &format!("project{i}"),
                vec![dir("cache", vec![file("payload", (65 + i) * MIB, 1)])],
            )
        })
        .collect();
    children.push(dir("tiny", vec![dir("temp", vec![file("small", 1024, 1)])]));
    let index = index(children);
    assert!(index.recommendations(NOW, 0).is_empty());
    let recommendations = index.recommendations(NOW, usize::MAX);
    assert_eq!(recommendations.len(), 100);
    assert_eq!(recommendations[0].known_allocated_bytes, 214 * MIB as i64);
    assert_eq!(recommendations[99].known_allocated_bytes, 115 * MIB as i64);
    assert_eq!(index.recommendations(NOW, 1).len(), 1);
}

// Losing stored classifier metadata when names are unknown breaks native scan equivalence.
#[test]
fn stored_category_is_used_when_unknown_names_have_no_inherited_kind() {
    let mut node = dir("unknown", vec![file("data", 128 * MIB, 1)]);
    node.category = Category::Media;
    let index = index(vec![node]);
    assert_eq!(
        index.classifications()[named(&index, "unknown")].category,
        Category::Media
    );
    assert_eq!(
        index.classifications()[named(&index, "data")].category,
        Category::Media
    );
    assert!(index.recommendations(NOW, 10).is_empty());
}

// Flattening discards the root's name; the indexed scan path still supplies context.
#[test]
fn a_selected_named_scan_root_supplies_category_and_reclaim_context() {
    let mut agent = index(vec![dir("layers", vec![file("layer", 128 * MIB, 1)])]);
    agent.root = r"E:\GeneratedAnalytics\.codex".into();
    assert_eq!(agent.classifications()[0].category, Category::AgentScratch);
    assert_eq!(
        agent.classifications()[named(&agent, "layer")].reclaim,
        Some(Reclaim::SandboxLayers)
    );
    assert_eq!(agent.recommendations(NOW, 10).len(), 1);

    let mut cache = index(vec![dir("objects", vec![file("blob", 128 * MIB, 1)])]);
    cache.root = r"E:\GeneratedAnalytics\.cache\".into();
    assert_eq!(
        cache.classifications()[named(&cache, "blob")].reclaim,
        Some(Reclaim::Regenerable)
    );
    assert_eq!(
        cache.recommendations(NOW, 10)[0].index,
        named(&cache, "objects")
    );
}

// An unclassified unknown top-level directory should inherit its largest recognizable child.
#[test]
fn unknown_top_level_projects_use_dominant_indexed_children_and_git_store_shape() {
    let index = index(vec![
        dir(
            "project",
            vec![
                dir("Documents", vec![file("notes", 64 * MIB, 1)]),
                dir(
                    "store",
                    vec![
                        file("HEAD", 1, 1),
                        dir("refs", vec![]),
                        dir("objects", vec![file("pack", 128 * MIB, 1)]),
                    ],
                ),
                file("README", 1, 1),
            ],
        ),
        dir(
            "SteamLibrary",
            vec![dir(
                "steamapps",
                vec![
                    dir("temp", vec![file("partial", 64 * MIB, 1)]),
                    dir("common", vec![file("game", 256 * MIB, 1)]),
                ],
            )],
        ),
    ]);
    let classes = index.classifications();
    assert_eq!(classes[named(&index, "store")].category, Category::Git);
    assert_eq!(classes[named(&index, "project")].category, Category::Git);
    assert_eq!(classes[named(&index, "README")].category, Category::Git);
    assert_eq!(classes[named(&index, "common")].category, Category::Media);
    assert_eq!(classes[named(&index, "game")].reclaim, None);
}

// A caller may hold valid indexed links with stale ancestor coverage; recompute conservatively.
#[test]
fn descendant_issues_cannot_be_hidden_by_an_ancestor_coverage_flag() {
    let mut index = index(vec![dir("cache", vec![file("known", 128 * MIB, 1)])]);
    let known = named(&index, "known");
    index.entries[known].issue = 3;
    let recommendations = index.recommendations(NOW, 10);
    assert!(!recommendations[0].coverage_complete);
    assert!(!recommendations[0].allocation_complete);
}

// A directory named like a manifest supplies no source-file evidence; unsafe ancestry suppresses descendants.
#[test]
fn manifest_directories_do_not_authorize_build_classification_and_unsafe_ancestry_is_excluded() {
    let mut unsafe_root = dir(
        "foreign",
        vec![dir("cache", vec![file("linked", 256 * MIB, 1)])],
    );
    unsafe_root.attributes = 0x400;
    let index = index(vec![
        dir(
            "unrelated",
            vec![
                dir("Cargo.toml", vec![]),
                dir("target", vec![file("data", 128 * MIB, 1)]),
            ],
        ),
        unsafe_root,
    ]);
    assert_eq!(
        index.classifications()[named(&index, "target")].reclaim,
        None
    );
    assert!(index.recommendations(NOW, 10).is_empty());
}
