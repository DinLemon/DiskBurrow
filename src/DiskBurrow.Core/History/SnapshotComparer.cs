using DiskBurrow.Core.Scanning;

namespace DiskBurrow.Core.History;

public sealed class SnapshotComparer
{
    public IReadOnlyList<FolderChange> Compare(ScanSnapshot previous, ScanSnapshot current)
    {
        if (!StringComparer.OrdinalIgnoreCase.Equals(NormalizePath(previous.Root), NormalizePath(current.Root))) return [];
        var before = previous.Directories.ToDictionary(d => NormalizePath(d.Path), StringComparer.OrdinalIgnoreCase);
        var after = current.Directories.ToDictionary(d => NormalizePath(d.Path), StringComparer.OrdinalIgnoreCase);
        var changes = new List<FolderChange>();
        foreach (var path in before.Keys.Union(after.Keys, StringComparer.OrdinalIgnoreCase).Order(StringComparer.OrdinalIgnoreCase))
        {
            before.TryGetValue(path, out var old);
            after.TryGetValue(path, out var now);
            var comparable = previous.TraversalCompleted && current.TraversalCompleted &&
                (old?.CoverageComplete ?? CoveredParent(path, before)) &&
                (now?.CoverageComplete ?? CoveredParent(path, after));
            if (!comparable)
            {
                changes.Add(new(path, 0, null, FolderChangeKind.Unavailable, false));
                continue;
            }
            var delta = (now?.LogicalBytes ?? 0) - (old?.LogicalBytes ?? 0);
            long? allocated = (old is null || old.AllocatedBytes.HasValue) && (now is null || now.AllocatedBytes.HasValue)
                ? (now?.AllocatedBytes ?? 0) - (old?.AllocatedBytes ?? 0) : null;
            if (old is not null && now is not null && delta == 0 && allocated is null or 0) continue;
            changes.Add(new(path, delta, allocated,
                old is null ? FolderChangeKind.New : now is null ? FolderChangeKind.Removed : FolderChangeKind.Changed, true));
        }
        return changes;
    }

    // Windows paths form the persistence and comparison key; casing does not identify a different folder.
    public static string NormalizePath(string path) => Path.TrimEndingDirectorySeparator(Path.GetFullPath(path.Replace('/', '\\')));

    private static bool CoveredParent(string path, IReadOnlyDictionary<string, DirectoryObservation> directories)
    {
        for (var parent = Path.GetDirectoryName(path); parent is not null; parent = Path.GetDirectoryName(parent))
            if (directories.TryGetValue(NormalizePath(parent), out var directory)) return directory.CoverageComplete;
        return false;
    }
}
