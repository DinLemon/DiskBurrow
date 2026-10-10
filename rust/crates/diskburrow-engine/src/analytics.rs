//! Informational classifications derived only from indexed metadata.
use crate::LiveIndex;
pub use disktree_core::classify::{Category, Reclaim};
use disktree_core::classify::{category_of_name, reclaim_of};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

const MIN_BYTES: i64 = 64 * 1024 * 1024;
const MAX_RECOMMENDATIONS: usize = 100;
const DAY: i64 = 86_400;
const UNSAFE_ATTRIBUTES: u32 = 0x1000 | 0x40000 | 0x400000 | 0x400;
const CARGO_MANIFEST: u8 = 1;
const PACKAGE_MANIFEST: u8 = 2;
const APPLICATION_SUPPORT: u8 = 4;
const GIT_HEAD: u8 = 8;
const GIT_OBJECTS: u8 = 16;
const GIT_REFS: u8 = 32;
const GIT_SHAPE: u8 = GIT_HEAD | GIT_OBJECTS | GIT_REFS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryClassification {
    pub category: Category,
    pub reclaim: Option<Reclaim>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecommendationReason {
    Reclaimable(Reclaim),
    Worktrees { count: usize, oldest_days: i64 },
    StaleExperiment { age_days: i64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub index: usize,
    pub reason: RecommendationReason,
    /// Observed allocation of known, ordinary leaves. This is not a deletion
    /// projection: physical identities must be revalidated by native review.
    pub known_allocated_bytes: i64,
    pub logical_bytes: i64,
    pub coverage_complete: bool,
    pub allocation_complete: bool,
}

impl LiveIndex {
    /// Categories and inherited reclaim reasons, in live-entry index order.
    /// Uses the existing scan category plus names and indexed sibling context.
    /// No filesystem access, persisted metadata changes or cleanup authority.
    pub fn classifications(&self) -> Vec<EntryClassification> {
        let mut contexts = vec![0_u8; self.entries.len()];
        for entry in &self.entries {
            if let Some(parent) = entry.parent {
                let flag = if entry.name.eq_ignore_ascii_case("Cargo.toml") && !entry.directory {
                    CARGO_MANIFEST
                } else if entry.name.eq_ignore_ascii_case("package.json") && !entry.directory {
                    PACKAGE_MANIFEST
                } else if entry.name.eq_ignore_ascii_case("Application Support") && entry.directory
                {
                    APPLICATION_SUPPORT
                } else if entry.name.eq_ignore_ascii_case("HEAD") && !entry.directory {
                    GIT_HEAD
                } else if entry.name.eq_ignore_ascii_case("objects") && entry.directory {
                    GIT_OBJECTS
                } else if entry.name.eq_ignore_ascii_case("refs") && entry.directory {
                    GIT_REFS
                } else {
                    0
                };
                contexts[parent] |= flag;
            }
        }
        let mut result: Vec<EntryClassification> = Vec::with_capacity(self.entries.len());
        for (i, entry) in self.entries.iter().enumerate() {
            let Some(parent) = entry.parent else {
                let name = self
                    .root
                    .trim_end_matches('\\')
                    .rsplit('\\')
                    .next()
                    .unwrap_or("");
                result.push(EntryClassification {
                    category: category_of_name(name)
                        .or_else(|| (contexts[i] & GIT_SHAPE == GIT_SHAPE).then_some(Category::Git))
                        .unwrap_or(stored_category(entry.category)),
                    reclaim: reclaim_of(name, Category::Other, |_| false),
                });
                continue;
            };
            let inherited = result[parent];
            if !entry.directory {
                result.push(inherited);
                continue;
            }
            let stored = stored_category(entry.category);
            let category = self
                .named_category(i, &contexts)
                .or_else(|| (stored != Category::Other).then_some(stored))
                .or_else(|| {
                    (parent == 0)
                        .then(|| self.dominant_category(i, &contexts))
                        .flatten()
                })
                .unwrap_or(inherited.category);
            let context = contexts[parent];
            let reclaim = inherited.reclaim.or_else(|| {
                reclaim_of(&entry.name, inherited.category, |wanted| match wanted {
                    "Cargo.toml" => context & CARGO_MANIFEST != 0,
                    "package.json" => context & PACKAGE_MANIFEST != 0,
                    "Application Support" => context & APPLICATION_SUPPORT != 0,
                    _ => false,
                })
            });
            result.push(EntryClassification { category, reclaim });
        }
        result
    }

    fn named_category(&self, index: usize, contexts: &[u8]) -> Option<Category> {
        category_of_name(&self.entries[index].name)
            .or_else(|| (contexts[index] & GIT_SHAPE == GIT_SHAPE).then_some(Category::Git))
    }

    // Mirrors the vendored classifier's three-level top-level inference, using
    // indexed sizes instead of depending on a pre-sorted child vector.
    fn dominant_category(&self, mut index: usize, contexts: &[u8]) -> Option<Category> {
        for _ in 0..3 {
            let children = &self.entries[index].children;
            if let Some((_, category)) = children
                .iter()
                .copied()
                .filter(|&child| self.entries[child].directory)
                .filter_map(|child| {
                    self.named_category(child, contexts)
                        .map(|category| (child, category))
                })
                .max_by_key(|&(child, _)| (self.entries[child].logical, Reverse(child)))
            {
                return Some(category);
            }
            index = children
                .iter()
                .copied()
                .filter(|&child| self.entries[child].directory)
                .max_by_key(|&child| (self.entries[child].logical, Reverse(child)))?;
        }
        None
    }

    /// At most 100 nonoverlapping informational directory roots, ranked by
    /// known allocation, then logical bytes and index. The scanned root itself
    /// is excluded. Unknown allocation/coverage never becomes a known total.
    /// Working storage is linear in the bounded live index; the ranking heap
    /// holds at most `limit.min(100)` entries, rather than every candidate.
    pub fn recommendations(&self, now: i64, limit: usize) -> Vec<Recommendation> {
        let limit = limit.min(MAX_RECOMMENDATIONS);
        if limit == 0 || self.entries.is_empty() {
            return Vec::new();
        }
        let classes = self.classifications();
        let mut known = vec![0_i64; self.entries.len()];
        let mut complete = vec![true; self.entries.len()];
        let mut coverage = vec![true; self.entries.len()];
        let mut suppressed = vec![false; self.entries.len()];
        // Suppress all descendants of an unsafe directory, even if a child's
        // own listing attributes look ordinary.
        for (i, entry) in self.entries.iter().enumerate() {
            suppressed[i] = entry.attributes & UNSAFE_ATTRIBUTES != 0
                || entry.parent.is_some_and(|parent| suppressed[parent]);
            coverage[i] = entry.coverage && entry.issue == 0 && !suppressed[i];
            complete[i] = coverage[i];
            if !entry.directory {
                if !suppressed[i] {
                    known[i] = entry.allocated.unwrap_or(0);
                }
                complete[i] &= entry.allocated.is_some();
            }
        }
        for i in (0..self.entries.len()).rev() {
            if let Some(parent) = self.entries[i].parent {
                known[parent] = known[parent].saturating_add(known[i]);
                complete[parent] &= complete[i];
                coverage[parent] &= coverage[i];
            }
        }
        let mut largest = BinaryHeap::with_capacity(limit);
        for (i, entry) in self.entries.iter().enumerate().skip(1) {
            suppressed[i] |= entry.parent.is_some_and(|parent| suppressed[parent]);
            if suppressed[i] || !entry.directory {
                continue;
            }
            if entry.logical < MIN_BYTES && known[i] < MIN_BYTES {
                continue;
            }
            if self.recommendation_reason(i, &classes, now).is_some() {
                suppressed[i] = true;
                let key = (known[i], entry.logical, Reverse(i));
                if largest.len() < limit {
                    largest.push(Reverse(key));
                } else if let Some(mut smallest) = largest.peek_mut()
                    && key > smallest.0
                {
                    *smallest = Reverse(key);
                }
            }
        }
        let mut ranked: Vec<_> = largest.into_iter().map(|Reverse(key)| key).collect();
        ranked.sort_unstable_by_key(|key| Reverse(*key));
        ranked
            .into_iter()
            .map(
                |(known_allocated_bytes, logical_bytes, Reverse(index))| Recommendation {
                    index,
                    reason: self
                        .recommendation_reason(index, &classes, now)
                        .expect("Candidate has a reason"),
                    known_allocated_bytes,
                    logical_bytes,
                    coverage_complete: coverage[index],
                    allocation_complete: complete[index],
                },
            )
            .collect()
    }

    fn recommendation_reason(
        &self,
        index: usize,
        classes: &[EntryClassification],
        now: i64,
    ) -> Option<RecommendationReason> {
        let entry = &self.entries[index];
        if let Some(reason) = classes[index].reclaim {
            return Some(RecommendationReason::Reclaimable(reason));
        }
        if classes[index].category == Category::AgentScratch
            && entry.name.eq_ignore_ascii_case("worktrees")
        {
            let mut count = 0;
            let mut oldest = now;
            for &child in &entry.children {
                let child = &self.entries[child];
                if child.directory {
                    count += 1;
                    if child.modified > 0 {
                        oldest = oldest.min(child.modified);
                    }
                }
            }
            if count > 0 {
                return Some(RecommendationReason::Worktrees {
                    count,
                    oldest_days: now.saturating_sub(oldest).max(0) / DAY,
                });
            }
        }
        let parent = entry.parent?;
        let experiment_root = &self.entries[parent];
        if classes[parent].category == Category::AgentScratch
            && (experiment_root.name.eq_ignore_ascii_case("experiments")
                || experiment_root.name.eq_ignore_ascii_case("tries"))
            && entry.modified > 0
            && now.saturating_sub(entry.modified) > disktree_core::insights::STALE_DAYS * DAY
        {
            return Some(RecommendationReason::StaleExperiment {
                age_days: now.saturating_sub(entry.modified) / DAY,
            });
        }
        None
    }
}

fn stored_category(category: u8) -> Category {
    match category {
        0 => Category::Code,
        1 => Category::AgentScratch,
        2 => Category::Toolchain,
        3 => Category::Synced,
        4 => Category::Git,
        5 => Category::Media,
        6 => Category::Documents,
        7 => Category::Cache,
        _ => Category::Other,
    }
}
