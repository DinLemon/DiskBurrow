using System.Collections.ObjectModel;
namespace DiskBurrow.Core.Scanning;

/// <summary>A compact, live-only index. Parents precede children; names do not contain separators.</summary>
public readonly record struct ScanTreeEntry(int ParentIndex, string Name, bool IsDirectory,
    long LogicalBytes, long? AllocatedBytes, DateTimeOffset ModifiedUtc, long FileCount,
    bool CoverageComplete, FileAttributes Attributes);

public sealed class ScanTree
{
    public string Root { get; }
    public IReadOnlyList<ScanTreeEntry> Entries { get; }
    public ScanTree(string root, IEnumerable<ScanTreeEntry> entries) : this(root, entries.ToArray(), true) {}
    internal ScanTree(string root, ScanTreeEntry[] ownedEntries, bool owned)
    {
        Root = Path.TrimEndingDirectorySeparator(Path.GetFullPath(root));
        var copy = ownedEntries;
        if (copy.Length == 0 || copy[0].ParentIndex != -1 || !copy[0].IsDirectory)
            throw new ArgumentException("The tree must contain a directory root.", nameof(ownedEntries));
        for (var i = 1; i < copy.Length; i++)
            if (copy[i].ParentIndex < 0 || copy[i].ParentIndex >= i || !copy[copy[i].ParentIndex].IsDirectory ||
                string.IsNullOrEmpty(copy[i].Name) || copy[i].Name is "." or ".." || copy[i].Name.IndexOfAny(['/', '\\', ':', '\0']) >= 0 ||
                copy[i].LogicalBytes < 0 || copy[i].AllocatedBytes < 0 || copy[i].FileCount < 0)
                throw new ArgumentException("Invalid parent or entry name.", nameof(ownedEntries));
        Entries = new ReadOnlyCollection<ScanTreeEntry>(copy);
    }
    public string GetPath(int index)
    {
        if ((uint)index >= (uint)Entries.Count) throw new ArgumentOutOfRangeException(nameof(index));
        var names = new Stack<string>();
        while (index > 0) { names.Push(Entries[index].Name); index = Entries[index].ParentIndex; }
        var path = Root;
        while (names.TryPop(out var name)) path = Path.Combine(path, name);
        return path;
    }
    public bool IsDescendantOrSelf(int index, int ancestor)
    {
        while (index >= 0) { if (index == ancestor) return true; index = Entries[index].ParentIndex; }
        return false;
    }
}

/// <summary>Reusable by directory walking and MFT metadata producers. Build aggregates bottom-up.</summary>
public sealed class ScanTreeBuilder
{
    private readonly string root;
    private readonly List<ScanTreeEntry> entries = [];
    public ScanTreeBuilder(string root) { this.root = root; entries.Add(new(-1, "", true, 0, 0, default, 0, true, FileAttributes.Directory)); }
    public int AddDirectory(int parentIndex, string name, FileAttributes attributes = FileAttributes.Directory) => Add(new(parentIndex, name, true, 0, 0, default, 0, true, attributes));
    public int AddFile(int parentIndex, string name, long logicalBytes, long? allocatedBytes, DateTimeOffset modifiedUtc,
        FileAttributes attributes = FileAttributes.Normal) => Add(new(parentIndex, name, false, Math.Max(0, logicalBytes), allocatedBytes, modifiedUtc, 1, true, attributes));
    private int Add(ScanTreeEntry entry)
    {
        if ((uint)entry.ParentIndex >= (uint)entries.Count || !entries[entry.ParentIndex].IsDirectory ||
            string.IsNullOrEmpty(entry.Name) || entry.Name is "." or ".." || entry.Name.IndexOfAny(['/', '\\', ':', '\0']) >= 0 || entry.AllocatedBytes < 0)
            throw new ArgumentException("Invalid parent or name.");
        entries.Add(entry); return entries.Count - 1;
    }
    public void SetAllocatedBytes(int index, long? bytes)
    {
        if(bytes < 0)throw new ArgumentOutOfRangeException(nameof(bytes));
        entries[index] = entries[index] with { AllocatedBytes = bytes };
    }
    public void MarkIncomplete(int index) => entries[index] = entries[index] with { CoverageComplete = false, AllocatedBytes = null };
    public ScanTree Build()
    {
        var result = entries.ToArray();
        for (var i = result.Length - 1; i > 0; i--)
        {
            var child = result[i]; var parent = result[child.ParentIndex];
            result[child.ParentIndex] = parent with {
                LogicalBytes = checked(parent.LogicalBytes + child.LogicalBytes),
                AllocatedBytes = parent.AllocatedBytes is {} a && child.AllocatedBytes is {} b ? checked(a + b) : null,
                FileCount = checked(parent.FileCount + child.FileCount),
                ModifiedUtc = parent.ModifiedUtc > child.ModifiedUtc ? parent.ModifiedUtc : child.ModifiedUtc,
                CoverageComplete = parent.CoverageComplete && child.CoverageComplete };
        }
        return new(root, result, true);
    }
}
