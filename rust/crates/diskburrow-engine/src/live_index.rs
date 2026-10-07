use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use diskburrow_services::{
    DirectoryObservation, FileObservation, ScanIssue, ScanIssueKind, ScanSnapshot,
    is_fully_qualified, normalize_path,
};
use disktree_core::tree::{Node, NodeKind};
use std::collections::BinaryHeap;
pub const MAXIMUM_ENTRIES: usize = 20_000_000;
pub const MAXIMUM_RESIDENT_BYTES: usize = 1024 * 1024 * 1024;
pub const OTHERS_INDEX: usize = usize::MAX;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveEntry {
    pub parent: Option<usize>,
    pub name: Box<str>,
    pub directory: bool,
    pub logical: i64,
    pub allocated: Option<i64>,
    pub modified: i64,
    pub files: i64,
    pub coverage: bool,
    pub attributes: u32,
    pub children: Vec<usize>,
    pub category: u8,
    pub issue: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveIndex {
    pub root: String,
    pub entries: Vec<LiveEntry>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MapMetric {
    #[default]
    Allocated,
    Logical,
    Files,
}
#[derive(Debug, Clone, PartialEq)]
pub struct IndexTile {
    pub index: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub depth: u32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub indices: Vec<usize>,
    pub total: usize,
}
impl LiveIndex {
    pub fn from_tree(root: String, tree: Node) -> Result<Self> {
        Self::from_tree_with_limits(root, tree, MAXIMUM_ENTRIES, MAXIMUM_RESIDENT_BYTES)
    }
    pub fn from_tree_with_limits(
        root: String,
        tree: Node,
        max_entries: usize,
        max_resident: usize,
    ) -> Result<Self> {
        validate_limits(max_entries, max_resident)?;
        validate_root(&root)?;
        let mut pending = PendingTree(vec![(None, tree)]);
        let mut entries: Vec<LiveEntry> = Vec::new();
        let mut resident = 0_usize;
        while let Some((parent, node)) = pending.0.pop() {
            let Node {
                name,
                kind,
                bytes,
                logical,
                attributes,
                allocation_known,
                files,
                read_error,
                read_error_kind,
                modified,
                category,
                children,
                ..
            } = node;
            let index = entries.len();
            // Move descendants before checking fields, so a rejected/deep tree is drained iteratively.
            for child in children.into_iter().rev() {
                pending.0.push((Some(index), child));
            }
            ensure!(index < max_entries, "Live index exceeds its entry limit.");
            let name = if parent.is_none() {
                Box::<str>::from("")
            } else {
                name
            };
            resident = resident
                .checked_add(entry_reserve(name.len()))
                .ok_or_else(|| anyhow::anyhow!("Resident budget overflow"))?;
            ensure!(
                resident <= max_resident,
                "Live index exceeds its resident memory budget."
            );
            if let Some(parent) = parent {
                ensure!(entries[parent].directory, "A file cannot contain children.");
                validate_name(&name)?;
                entries[parent].children.push(index);
            }
            entries.push(LiveEntry {
                parent,
                name,
                directory: kind == NodeKind::Directory,
                logical: i64::try_from(logical)?,
                allocated: if allocation_known {
                    Some(i64::try_from(bytes)?)
                } else {
                    None
                },
                modified,
                files: i64::try_from(files)?,
                coverage: !read_error
                    && kind != NodeKind::Symlink
                    && attributes & UNSAFE_ATTRIBUTES == 0,
                attributes,
                children: Vec::new(),
                category: category as u8,
                issue: read_error_kind,
            });
        }
        for i in (1..entries.len()).rev() {
            if !entries[i].coverage {
                let parent = entries[i].parent.expect("Nonroot has a parent");
                entries[parent].coverage = false;
            }
        }
        let index = Self {
            root: normalize_path(&root),
            entries,
        };
        index.validate(max_entries, max_resident)?;
        Ok(index)
    }
    pub fn path(&self, index: usize) -> String {
        if index >= self.entries.len() {
            return String::new();
        }
        let mut names = vec![];
        let mut current = index;
        for _ in 0..self.entries.len() {
            if current == 0 {
                let mut path = self.root.clone();
                for name in names.into_iter().rev() {
                    if !path.ends_with('\\') {
                        path.push('\\');
                    }
                    path.push_str(name);
                }
                return path;
            }
            let Some(entry) = self.entries.get(current) else {
                return String::new();
            };
            names.push(entry.name.as_ref());
            let Some(parent) = entry.parent else {
                return String::new();
            };
            current = parent;
        }
        String::new()
    }
    pub fn descendants(&self, index: usize) -> Vec<usize> {
        let Some(entry) = self.entries.get(index) else {
            return vec![];
        };
        let mut stack: Vec<_> = entry.children.iter().rev().copied().collect();
        let mut result = vec![];
        while let Some(next) = stack.pop() {
            if result.len() >= self.entries.len() {
                break;
            }
            result.push(next);
            if let Some(entry) = self.entries.get(next) {
                stack.extend(entry.children.iter().rev());
            }
        }
        result
    }
    pub fn snapshot(&self, started: DateTime<Utc>, completed: DateTime<Utc>) -> ScanSnapshot {
        let mut largest = BinaryHeap::new();
        let mut directories = vec![];
        let mut issues = vec![];
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.directory {
                directories.push(DirectoryObservation {
                    path: self.path(index),
                    logical_bytes: entry.logical,
                    allocated_bytes: entry.allocated,
                    coverage_complete: entry.coverage,
                });
            } else {
                largest.push(std::cmp::Reverse((entry.logical, std::cmp::Reverse(index))));
                if largest.len() > 100 {
                    largest.pop();
                }
            }
            let kind = if entry.attributes & CLOUD_ATTRIBUTES != 0 {
                Some(ScanIssueKind::CloudSkipped)
            } else if entry.attributes & 0x400 != 0 {
                Some(ScanIssueKind::ReparseSkipped)
            } else if entry.issue != 0 {
                Some(match entry.issue {
                    1 => ScanIssueKind::AccessDenied,
                    3 => ScanIssueKind::MetadataUnavailable,
                    _ => ScanIssueKind::IoFailure,
                })
            } else if !entry.coverage && entry.children.iter().all(|i| self.entries[*i].coverage) {
                Some(ScanIssueKind::IoFailure)
            } else if entry.allocated.is_none() && !entry.directory {
                Some(ScanIssueKind::MetadataUnavailable)
            } else {
                None
            };
            if let Some(kind) = kind {
                issues.push(ScanIssue {
                    path: self.path(index),
                    kind,
                });
            }
        }
        let mut ordered: Vec<_> = largest
            .into_iter()
            .map(|std::cmp::Reverse((bytes, std::cmp::Reverse(index)))| (bytes, index))
            .collect();
        ordered.sort_by_key(|(bytes, index)| (std::cmp::Reverse(*bytes), *index));
        let largest_files = ordered
            .into_iter()
            .map(|(_, index)| {
                let entry = &self.entries[index];
                FileObservation {
                    path: self.path(index),
                    identity: None,
                    logical_bytes: entry.logical,
                    allocated_bytes: entry.allocated,
                    modified_utc: DateTime::from_timestamp(entry.modified, 0)
                        .unwrap_or(DateTime::UNIX_EPOCH),
                    link_count: 0,
                    attributes: entry.attributes,
                }
            })
            .collect();
        ScanSnapshot {
            id: uuid::Uuid::new_v4(),
            root: self.root.clone(),
            started_utc: started,
            completed_utc: completed,
            traversal_completed: true,
            directories,
            largest_files,
            issues,
        }
    }
    pub fn search(&self, scope: usize, query: &str, global: bool, limit: usize) -> SearchResults {
        if scope >= self.entries.len() {
            return SearchResults::default();
        }
        if query.is_empty() {
            let mut indices = self.entries[scope].children.clone();
            indices.sort_by_key(|i| (std::cmp::Reverse(self.entries[*i].logical), *i));
            let total = indices.len();
            indices.truncate(limit.min(1000));
            return SearchResults { indices, total };
        }
        let found = self.matches(scope, query, global);
        let mut result = SearchResults::default();
        for (index, matched) in found.into_iter().enumerate() {
            if matched {
                result.total += 1;
                if result.indices.len() < limit.min(1000) {
                    result.indices.push(index);
                }
            }
        }
        result
    }
    fn matches(&self, scope: usize, query: &str, global: bool) -> Vec<bool> {
        let mut in_scope = vec![false; self.entries.len()];
        let mut matches = vec![false; self.entries.len()];
        let query = query.replace('/', "\\").to_lowercase();
        let by_path = query.contains('\\');
        for (i, entry) in self.entries.iter().enumerate() {
            in_scope[i] = global || i == scope || entry.parent.is_some_and(|p| in_scope[p]);
            if !in_scope[i] || query.is_empty() {
                continue;
            }
            matches[i] = if by_path {
                self.path(i).to_lowercase().contains(&query)
            } else {
                entry.name.to_lowercase().contains(&query)
            };
        }
        matches
    }
    pub fn projected_weights(
        &self,
        scope: usize,
        metric: MapMetric,
        query: &str,
        isolate: bool,
        global: bool,
    ) -> Vec<i64> {
        let count = self.entries.len();
        let mut weights = vec![0_i64; count];
        if scope >= count {
            return weights;
        }
        let mut keep = vec![!isolate || query.is_empty(); count];
        if isolate && !query.is_empty() {
            let found = self.matches(scope, query, global);
            let mut whole = vec![false; count];
            for (i, entry) in self.entries.iter().enumerate() {
                whole[i] = (found[i] && entry.directory) || entry.parent.is_some_and(|p| whole[p]);
                keep[i] = found[i] || whole[i];
            }
            for i in (1..count).rev() {
                if keep[i] {
                    keep[self.entries[i].parent.expect("Validated nonroot")] = true;
                }
            }
        }
        for i in (0..count).rev() {
            let entry = &self.entries[i];
            if !entry.directory && keep[i] {
                weights[i] = match metric {
                    MapMetric::Logical => entry.logical,
                    MapMetric::Allocated => entry.allocated.unwrap_or(0),
                    MapMetric::Files => entry.files,
                };
            }
            if let Some(parent) = entry.parent {
                weights[parent] = weights[parent].saturating_add(weights[i]);
            }
        }
        weights
    }
    pub fn layout(
        &self,
        rootindex: usize,
        bounds: (f32, f32),
        metric: MapMetric,
        query: &str,
        isolate: bool,
        global: bool,
    ) -> Vec<IndexTile> {
        let (w, h) = bounds;
        if rootindex >= self.entries.len()
            || !self.entries[rootindex].directory
            || !w.is_finite()
            || !h.is_finite()
            || w <= 0.0
            || h <= 0.0
        {
            return vec![];
        }
        let weights = self.projected_weights(rootindex, metric, query, isolate, global);
        let mut result = vec![];
        self.fill(rootindex, (0.0, 0.0, w, h), 0, &weights, &mut result);
        result
    }
    fn fill(
        &self,
        parent: usize,
        bounds: (f32, f32, f32, f32),
        depth: u32,
        weights: &[i64],
        out: &mut Vec<IndexTile>,
    ) {
        let mut ranked: Vec<_> = self.entries[parent]
            .children
            .iter()
            .copied()
            .filter(|i| weights[*i] > 0)
            .collect();
        ranked.sort_by_key(|i| (std::cmp::Reverse(weights[*i]), *i));
        let mut items: Vec<(usize, f64)> = Vec::new();
        let keep = if ranked.len() > 96 { 95 } else { 96 };
        let mut tail = 0.0;
        for (position, index) in ranked.into_iter().enumerate() {
            if position < keep {
                items.push((index, weights[index] as f64));
            } else {
                tail += weights[index] as f64;
            }
        }
        if tail > 0.0 {
            items.push((OTHERS_INDEX, tail));
        }
        items.sort_by(|a, b| b.1.total_cmp(&a.1));
        for tile in squarify(&items, bounds, depth) {
            let child = tile.index;
            let subdivide = child != OTHERS_INDEX
                && self.entries[child].directory
                && depth < 2
                && tile.width > 65.0
                && tile.height > 55.0;
            let inner = (
                tile.x + 3.0,
                tile.y + 22.0,
                (tile.width - 6.0).max(0.0),
                (tile.height - 25.0).max(0.0),
            );
            out.push(tile);
            if subdivide {
                self.fill(child, inner, depth + 1, weights, out);
            }
        }
    }
    pub fn validate(&self, max_entries: usize, max_resident: usize) -> Result<()> {
        validate_limits(max_entries, max_resident)?;
        validate_root(&self.root)?;
        ensure!(
            !self.entries.is_empty() && self.entries.len() <= max_entries,
            "Invalid live entry count."
        );
        let root = &self.entries[0];
        ensure!(
            root.parent.is_none() && root.directory && root.name.is_empty(),
            "Invalid live root entry."
        );
        let mut seen = vec![false; self.entries.len()];
        let mut path_lengths = vec![self.root.len(); self.entries.len()];
        let mut resident = 0_usize;
        for (i, entry) in self.entries.iter().enumerate() {
            ensure!(
                entry.logical >= 0 && entry.allocated.is_none_or(|n| n >= 0) && entry.files >= 0,
                "Negative metadata quantity."
            );
            ensure!(
                DateTime::<Utc>::from_timestamp(entry.modified, 0).is_some(),
                "Invalid modification time."
            );
            ensure!(entry.category <= 8 && entry.issue <= 4, "Invalid category.");
            if i > 0 {
                validate_name(&entry.name)?;
                let parent = entry
                    .parent
                    .ok_or_else(|| anyhow::anyhow!("Missing parent"))?;
                ensure!(
                    parent < i && self.entries[parent].directory,
                    "Parent must be an earlier directory."
                );
                path_lengths[i] = path_lengths[parent]
                    .checked_add(entry.name.len() + 1)
                    .ok_or_else(|| anyhow::anyhow!("Path size overflow"))?;
                ensure!(path_lengths[i] <= 32767 * 4, "Derived path is too long.");
            }
            ensure!(
                entry.directory || entry.children.is_empty(),
                "File with children."
            );
            for &child in &entry.children {
                ensure!(
                    child > i
                        && child < self.entries.len()
                        && self.entries[child].parent == Some(i)
                        && !seen[child],
                    "Invalid or duplicate child index."
                );
                seen[child] = true;
            }
            resident = resident
                .checked_add(entry_reserve(entry.name.len()) + entry.children.len() * 16)
                .ok_or_else(|| anyhow::anyhow!("Resident budget overflow"))?;
            // Completed snapshots materialize directory paths and diagnostic paths beside the live index.
            if entry.directory || !entry.coverage || entry.allocated.is_none() {
                resident = resident
                    .checked_add(256 + path_lengths[i] * 4)
                    .ok_or_else(|| anyhow::anyhow!("Snapshot budget overflow"))?;
            }
            ensure!(
                resident <= max_resident,
                "Live index exceeds its resident memory budget."
            );
        }
        ensure!(
            seen.iter().skip(1).all(|present| *present),
            "A child is missing from its parent."
        );
        let mut totals = vec![[0_i64; 3]; self.entries.len()];
        for i in (0..self.entries.len()).rev() {
            let entry = &self.entries[i];
            if !entry.directory {
                totals[i] = [entry.logical, entry.allocated.unwrap_or(0), entry.files];
            }
            if let Some(parent) = entry.parent {
                let child = totals[i];
                for (total, value) in totals[parent].iter_mut().zip(child) {
                    *total = total
                        .checked_add(value)
                        .ok_or_else(|| anyhow::anyhow!("Subtree quantity overflow"))?;
                }
            }
        }
        Ok(())
    }
}
const CLOUD_ATTRIBUTES: u32 = 0x1000 | 0x40000 | 0x400000;
const UNSAFE_ATTRIBUTES: u32 = CLOUD_ATTRIBUTES | 0x400;
pub(crate) fn entry_reserve(name_bytes: usize) -> usize {
    2 * std::mem::size_of::<LiveEntry>() + 64 + 2 * name_bytes
}
pub(crate) fn validate_limits(entries: usize, resident: usize) -> Result<()> {
    ensure!(
        entries > 0
            && entries <= MAXIMUM_ENTRIES
            && resident > 0
            && resident <= MAXIMUM_RESIDENT_BYTES,
        "Invalid live index limits."
    );
    Ok(())
}
pub(crate) fn validate_root(root: &str) -> Result<()> {
    ensure!(
        is_fully_qualified(root) && !root.starts_with("\\\\") && !root.contains('\0'),
        "A confirmed local absolute root is required."
    );
    Ok(())
}
pub(crate) fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && name.encode_utf16().count() <= 255
            && !name.contains(['/', '\\', ':', '\0']),
        "Invalid metadata name."
    );
    Ok(())
}
struct PendingTree(Vec<(Option<usize>, Node)>);
impl Drop for PendingTree {
    fn drop(&mut self) {
        while let Some((_, mut node)) = self.0.pop() {
            self.0.extend(
                std::mem::take(&mut node.children)
                    .into_iter()
                    .map(|child| (None, child)),
            );
        }
    }
}

fn squarify(items: &[(usize, f64)], bounds: (f32, f32, f32, f32), depth: u32) -> Vec<IndexTile> {
    let (mut x, mut y, mut w, mut h) = (
        f64::from(bounds.0),
        f64::from(bounds.1),
        f64::from(bounds.2),
        f64::from(bounds.3),
    );
    let total: f64 = items.iter().map(|(_, weight)| weight).sum();
    if total <= 0.0 {
        return vec![];
    }
    let scale = w * h / total;
    let mut out = vec![];
    let mut position = 0;
    while position < items.len() && w > 0.0 && h > 0.0 {
        let side = w.min(h);
        let mut end = position + 1;
        let mut sum = items[position].1 * scale;
        let worst = |sum: f64, minimum: f64, maximum: f64| {
            (side * side * maximum / (sum * sum)).max(sum * sum / (side * side * minimum))
        };
        let mut aspect = worst(sum, sum, sum);
        while end < items.len() {
            let candidate = items[end].1 * scale;
            let next = worst(sum + candidate, candidate, items[position].1 * scale);
            if next > aspect {
                break;
            }
            sum += candidate;
            aspect = next;
            end += 1;
        }
        let vertical = w >= h;
        let thickness = sum / side;
        let mut offset = 0.0;
        for &(index, weight) in &items[position..end] {
            let length = weight * scale / thickness;
            let (tx, ty, tw, th) = if vertical {
                (x, y + offset, thickness, length)
            } else {
                (x + offset, y, length, thickness)
            };
            out.push(IndexTile {
                index,
                x: tx as f32,
                y: ty as f32,
                width: tw as f32,
                height: th as f32,
                depth,
            });
            offset += length;
        }
        if vertical {
            x += thickness;
            w = (w - thickness).max(0.0);
        } else {
            y += thickness;
            h = (h - thickness).max(0.0);
        }
        position = end;
    }
    out
}
#[derive(Debug, Clone)]
pub struct MapNavigation {
    pub current: usize,
    pub focused: Option<usize>,
    history: Vec<usize>,
    position: usize,
}
impl Default for MapNavigation {
    fn default() -> Self {
        Self::new()
    }
}
impl MapNavigation {
    pub fn new() -> Self {
        Self {
            current: 0,
            focused: Some(0),
            history: vec![0],
            position: 0,
        }
    }
    pub fn can_back(&self) -> bool {
        self.position > 0
    }
    pub fn can_forward(&self) -> bool {
        self.position + 1 < self.history.len()
    }
    pub fn navigate(&mut self, index: &LiveIndex, target: usize) {
        let Some(entry) = index.entries.get(target) else {
            return;
        };
        if !entry.directory {
            if let Some(parent) = entry.parent {
                if parent != self.current {
                    self.navigate(index, parent);
                }
                self.focused = Some(target);
            }
            return;
        }
        self.current = target;
        self.focused = Some(target);
        self.history.truncate(self.position + 1);
        if self.history.last() != Some(&target) {
            self.history.push(target);
        }
        self.position = self.history.len() - 1;
    }
    pub fn back(&mut self) {
        if self.can_back() {
            self.position -= 1;
            self.current = self.history[self.position];
            self.focused = Some(self.current);
        }
    }
    pub fn forward(&mut self) {
        if self.can_forward() {
            self.position += 1;
            self.current = self.history[self.position];
            self.focused = Some(self.current);
        }
    }
    pub fn up(&mut self, index: &LiveIndex) {
        if let Some(parent) = index.entries.get(self.current).and_then(|e| e.parent) {
            self.navigate(index, parent);
        }
    }
    pub fn root(&mut self, index: &LiveIndex) {
        self.navigate(index, 0);
    }
    pub fn focus(&mut self, index: &LiveIndex, target: usize) {
        if target < index.entries.len() {
            self.focused = Some(target);
        }
    }
}
