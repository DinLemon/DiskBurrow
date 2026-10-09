//! A bounded ordinary-scan subtree; never a deletion inventory.
use diskburrow_windows::{
    NativeFileApi, WindowsNativeFileApi, equals_path, is_within, normalize_local_path,
};
use disktree_core::{scan::Known, tree::Node};
use std::{path::PathBuf, sync::Arc};
#[derive(Clone, PartialEq, Eq)]
pub struct ScanStamp {
    root: String,
    identity: (u64, String, i64, u32),
}
pub fn observe(root: &str) -> Option<ScanStamp> {
    let root = normalize_local_path(root)?;
    Some(ScanStamp {
        identity: directory_identity(&root)?,
        root,
    })
}
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone)]
pub struct ScanCache {
    pub root: String,
    tree: Arc<Node>,
    identity: (u64, String, i64, u32),
    pub reserved_bytes: usize,
    pub entries: usize,
}
impl ScanCache {
    pub fn capture(root: &str, tree: &Node, before: Option<ScanStamp>) -> Option<Self> {
        let (reserved_bytes, entries) = metadata_budget(tree)?;
        let root = normalize_local_path(root)?;
        let identity = directory_identity(&root)?;
        let before = before?;
        if before.root != root || before.identity != identity {
            return None;
        }
        Some(Self {
            root,
            tree: Arc::new(tree.clone()),
            identity,
            reserved_bytes,
            entries,
        })
    }
    pub fn still_valid(&self) -> bool {
        directory_identity(&self.root).is_some_and(|identity| identity == self.identity)
    }
    pub fn known_for(&self, root: &str) -> Option<Known> {
        let root = normalize_local_path(root)?;
        if equals_path(&root, &self.root) || !is_within(&self.root, &root) {
            return None;
        }
        if directory_identity(&self.root)? != self.identity
            || directory_identity(&root)?.0 != self.identity.0
        {
            return None;
        }
        let real_root = std::fs::canonicalize(&root).ok()?;
        let real_cached = std::fs::canonicalize(&self.root).ok()?;
        // Keep the walker's raw root prefix and the filesystem's actual child spelling.
        let suffix = real_cached.strip_prefix(real_root).ok()?;
        Some(Known {
            path: PathBuf::from(root).join(suffix),
            tree: self.tree.clone(),
        })
    }
}
fn directory_identity(path: &str) -> Option<(u64, String, i64, u32)> {
    let api = WindowsNativeFileApi;
    let handle = api.open_metadata(path).ok()?;
    if !equals_path(&api.final_path(&handle).ok()?, path) {
        return None;
    }
    let observation = api.inspect_handle(&handle, path).ok()?;
    if observation.attributes & 0x10 == 0
        || observation.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
    {
        return None;
    }
    let identity = observation.identity?;
    Some((
        identity.volume,
        identity.file_id,
        observation.modified_utc.timestamp(),
        observation.modified_utc.timestamp_subsec_nanos(),
    ))
}
fn metadata_budget(tree: &Node) -> Option<(usize, usize)> {
    let mut pending = vec![(tree, 0)];
    let mut bytes = 0usize;
    let mut entries = 0usize;
    while let Some((node, depth)) = pending.pop() {
        entries += 1;
        bytes = bytes.checked_add(
            std::mem::size_of::<Node>()
                + node.name.len()
                + node.children.capacity() * std::mem::size_of::<Node>()
                + 64,
        )?;
        if depth > MAX_DEPTH
            || entries > MAX_ENTRIES
            || bytes > MAX_BYTES
            || node.read_error
            || node.attributes & (0x400 | 0x1000 | 0x40000 | 0x400000) != 0
        {
            return None;
        }
        pending.extend(node.children.iter().map(|child| (child, depth + 1)));
    }
    Some((bytes, entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn bounded_cache_reuses_only_a_strict_same_volume_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("owned-subtree");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("cached.txt"), b"owned cache").unwrap();
        let tree = disktree_core::scan::scan(&child, Default::default()).unwrap();
        let cache = ScanCache::capture(
            child.to_str().unwrap(),
            &tree,
            observe(child.to_str().unwrap()),
        )
        .unwrap();
        let known = cache.known_for(temp.path().to_str().unwrap()).unwrap();
        assert_eq!(known.tree.logical, tree.logical);
        assert!(cache.known_for(child.to_str().unwrap()).is_none());
        assert!(
            cache
                .known_for(child.join("deeper").to_str().unwrap())
                .is_none()
        );
    }
    #[test]
    fn replaced_directory_never_reuses_an_old_tree() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("owned-subtree");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("cached.txt"), b"owned cache").unwrap();
        let tree = disktree_core::scan::scan(&child, Default::default()).unwrap();
        let cache = ScanCache::capture(
            child.to_str().unwrap(),
            &tree,
            observe(child.to_str().unwrap()),
        )
        .unwrap();
        fs::rename(&child, temp.path().join("old-owned-subtree")).unwrap();
        fs::create_dir(&child).unwrap();
        assert!(cache.known_for(temp.path().to_str().unwrap()).is_none());
    }
    #[test]
    fn replacement_after_scan_start_cannot_bind_or_reuse_old_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("owned-subtree");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("old.txt"), b"old").unwrap();
        let before = observe(child.to_str().unwrap());
        let tree = disktree_core::scan::scan(&child, Default::default()).unwrap();
        let cache = ScanCache::capture(child.to_str().unwrap(), &tree, before.clone()).unwrap();
        assert!(cache.known_for(temp.path().to_str().unwrap()).is_some());
        fs::rename(&child, temp.path().join("old-owned-subtree")).unwrap();
        fs::create_dir(&child).unwrap();
        assert!(
            ScanCache::capture(child.to_str().unwrap(), &tree, before).is_none(),
            "Old tree was bound to a replacement root"
        );
        assert!(
            !cache.still_valid(),
            "Reuse admission must be rechecked after traversal"
        );
    }
    #[test]
    fn known_path_is_actually_consumed_by_the_ordinary_raw_path_walker() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("Owned-SubTree");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("old.txt"), b"old").unwrap();
        let tree = disktree_core::scan::scan(&child, Default::default()).unwrap();
        let original = tree.logical;
        let cache = ScanCache::capture(
            &child.to_str().unwrap().to_lowercase(),
            &tree,
            observe(&child.to_str().unwrap().to_lowercase()),
        )
        .unwrap();
        // File contents can change without changing the directory's identity/mtime.
        // Reused observations must be labelled cached rather than silently called fresh.
        fs::write(child.join("old.txt"), vec![0u8; 999]).unwrap();
        let known = cache.known_for(temp.path().to_str().unwrap()).unwrap();
        let handle = disktree_core::scan::ScanHandle::spawn_with(
            temp.path().to_path_buf(),
            Default::default(),
            Some(known),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let widened = loop {
            if let Some(result) = handle.poll() {
                break result.unwrap();
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(
            widened
                .children
                .iter()
                .find(|node| node.name.as_ref() == "Owned-SubTree")
                .unwrap()
                .logical,
            original,
            "The walker did not consume the admitted cached subtree"
        );
    }
    #[test]
    fn deep_or_oversized_metadata_is_not_cached() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_str().unwrap();
        let mut tree = Node::directory("leaf");
        for _ in 0..65 {
            let mut parent = Node::directory("owned");
            parent.children.push(tree);
            tree = parent;
        }
        assert!(ScanCache::capture(root, &tree, observe(root)).is_none());
        let tree = Node::directory("x".repeat(33 * 1024 * 1024));
        assert!(ScanCache::capture(root, &tree, observe(root)).is_none());
    }
}
