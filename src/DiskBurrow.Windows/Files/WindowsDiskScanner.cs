using DiskBurrow.Core.Scanning;
using System.Diagnostics;

namespace DiskBurrow.Windows.Files;

public sealed class WindowsDiskScanner(NativeFileApi? files = null) : IDiskScanner
{
    private readonly NativeFileApi files = files ?? new NativeFileApi();

    public Task<ScanSnapshot> ScanAsync(string root, IProgress<ScanProgress>? progress, CancellationToken cancellationToken)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(root);
        var fullRoot = Path.TrimEndingDirectorySeparator(Path.GetFullPath(root));
        return Task.Run(() => Scan(fullRoot, progress, cancellationToken), cancellationToken);
    }

    private ScanSnapshot Scan(string root, IProgress<ScanProgress>? progress, CancellationToken ct)
    {
        ct.ThrowIfCancellationRequested();
        var started = DateTimeOffset.UtcNow;
        var directories = new Dictionary<string, Aggregate>(StringComparer.OrdinalIgnoreCase) { [root] = new() };
        var tree = new ScanTreeBuilder(root);
        var treeDirectories = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase) { [root] = 0 };
        var physicalFiles = new Dictionary<FileIdentity, PhysicalFile>();
        var largest = new List<FileObservation>(101);
        var issues = new List<ScanIssue>();
        var pending = new Stack<string>();
        long filesVisited = 0, directoriesVisited = 0;
        var progressClock = Stopwatch.StartNew();
        var lastProgress = TimeSpan.FromMilliseconds(-250);

        IEnumerable<Aggregate> Ancestors(string path)
        {
            for (var current = path; current is not null && directories.TryGetValue(current, out var aggregate); current = Path.GetDirectoryName(current))
                yield return aggregate;
        }

        void Incomplete(string directory, string path, ScanIssueKind kind)
        {
            issues.Add(new(path, kind));
            if (treeDirectories.TryGetValue(directory, out var treeIndex)) tree.MarkIncomplete(treeIndex);
            foreach (var aggregate in Ancestors(directory))
            {
                aggregate.Complete = false;
                aggregate.AllocationKnown = false;
            }
        }

        void Report(string path)
        {
            if (progress is null || progressClock.Elapsed - lastProgress < TimeSpan.FromMilliseconds(250)) return;
            ct.ThrowIfCancellationRequested();
            lastProgress = progressClock.Elapsed;
            progress.Report(new(filesVisited, directoriesVisited, path));
        }

        void ObserveFile(string path, string parent)
        {
            ct.ThrowIfCancellationRequested();
            filesVisited++;
            var observation = files.Inspect(path); // Exactly one metadata handle at a time, disposed by Inspect.
            ct.ThrowIfCancellationRequested();
            if (NativeFileApi.IsCloud(observation.Attributes))
            {
                Incomplete(parent, path, ScanIssueKind.CloudSkipped);
                return;
            }
            if (observation.Attributes.HasFlag(FileAttributes.ReparsePoint))
            {
                Incomplete(parent, path, ScanIssueKind.ReparseSkipped);
                return;
            }

            // A second attribute-only sample detects files changed while their metadata was observed.
            var latest = new FileInfo(path);
            if (!latest.Exists || latest.Length != observation.LogicalBytes || latest.LastWriteTimeUtc != observation.ModifiedUtc.UtcDateTime)
            {
                observation = observation with { AllocatedBytes = null };
                Incomplete(parent, path, ScanIssueKind.ChangedDuringScan);
            }
            else if (observation.AllocatedBytes is null || observation.Identity is null)
                Incomplete(parent, path, ScanIssueKind.MetadataUnavailable);

            foreach (var aggregate in Ancestors(parent)) aggregate.Logical += observation.LogicalBytes;
            largest.Add(observation);
            largest.Sort(CompareLargest);
            if (largest.Count > 100) largest.RemoveAt(100);

            var fileIndex = tree.AddFile(treeDirectories[parent], Path.GetFileName(path), observation.LogicalBytes, observation.Identity is null ? null : 0, observation.ModifiedUtc, observation.Attributes);
            if (observation.Identity is null) return;
            if (!physicalFiles.TryGetValue(observation.Identity, out var physical))
                physicalFiles.Add(observation.Identity, new(path, observation.AllocatedBytes, fileIndex));
            else
            {
                if (physical.AllocatedBytes != observation.AllocatedBytes)
                {
                    Incomplete(Path.GetDirectoryName(physical.Path)!, physical.Path, ScanIssueKind.ChangedDuringScan);
                    Incomplete(parent, path, ScanIssueKind.ChangedDuringScan);
                    physical.AllocatedBytes = null;
                }
                if (ComparePaths(path, physical.Path) < 0) { physical.Path = path; physical.TreeIndex = fileIndex; }
            }
        }

        // Check from the volume inward: querying a child first would already follow a redirected parent.
        var rootComponents = new Stack<string>();
        for (var component = root; component is not null; component = Path.GetDirectoryName(component))
            rootComponents.Push(component);
        var rootAllowed = true;
        while (rootComponents.TryPop(out var component))
        {
            ct.ThrowIfCancellationRequested();
            try
            {
                var attributes = File.GetAttributes(component);
                if (NativeFileApi.IsCloud(attributes))
                {
                    Incomplete(root, component, ScanIssueKind.CloudSkipped);
                    rootAllowed = false;
                    break;
                }
                if (attributes.HasFlag(FileAttributes.ReparsePoint))
                {
                    Incomplete(root, component, ScanIssueKind.ReparseSkipped);
                    rootAllowed = false;
                    break;
                }
            }
            catch (UnauthorizedAccessException)
            {
                Incomplete(root, component, ScanIssueKind.AccessDenied);
                rootAllowed = false;
                break;
            }
            catch (IOException)
            {
                Incomplete(root, component, ScanIssueKind.IoFailure);
                rootAllowed = false;
                break;
            }
        }
        if (rootAllowed) pending.Push(root);
        else Report(root);

        while (pending.TryPop(out var directory))
        {
            ct.ThrowIfCancellationRequested();
            directoriesVisited++;
            try
            {
                var attributes = File.GetAttributes(directory);
                if (NativeFileApi.IsCloud(attributes))
                {
                    Incomplete(directory, directory, ScanIssueKind.CloudSkipped);
                    continue;
                }
                if (attributes.HasFlag(FileAttributes.ReparsePoint))
                {
                    Incomplete(directory, directory, ScanIssueKind.ReparseSkipped);
                    continue;
                }
                if (!attributes.HasFlag(FileAttributes.Directory)) throw new IOException("The scan root must be a directory.");

                var options = new EnumerationOptions { IgnoreInaccessible = false, AttributesToSkip = 0, RecurseSubdirectories = false };
                foreach (var path in Directory.EnumerateFileSystemEntries(directory, "*", options))
                {
                    ct.ThrowIfCancellationRequested();
                    try
                    {
                        var entryAttributes = File.GetAttributes(path);
                        if (entryAttributes.HasFlag(FileAttributes.Directory))
                        {
                            directories.TryAdd(path, new());
                            treeDirectories[path] = tree.AddDirectory(treeDirectories[directory], Path.GetFileName(path), entryAttributes);
                            if (NativeFileApi.IsCloud(entryAttributes)) Incomplete(path, path, ScanIssueKind.CloudSkipped);
                            else if (entryAttributes.HasFlag(FileAttributes.ReparsePoint)) Incomplete(path, path, ScanIssueKind.ReparseSkipped);
                            else pending.Push(path);
                        }
                        else ObserveFile(path, directory);
                    }
                    catch (UnauthorizedAccessException) { Incomplete(directory, path, ScanIssueKind.AccessDenied); }
                    catch (IOException) { Incomplete(directory, path, ScanIssueKind.IoFailure); }
                    Report(path);
                }
            }
            catch (UnauthorizedAccessException) { Incomplete(directory, directory, ScanIssueKind.AccessDenied); }
            catch (IOException) { Incomplete(directory, directory, ScanIssueKind.IoFailure); }
            Report(directory);
        }

        // Charge allocation only after every identity has its deterministic canonical path.
        foreach (var physical in physicalFiles.Values)
        {
            ct.ThrowIfCancellationRequested();
            tree.SetAllocatedBytes(physical.TreeIndex, physical.AllocatedBytes);
            foreach (var aggregate in Ancestors(Path.GetDirectoryName(physical.Path)!))
            {
                if (physical.AllocatedBytes is { } bytes) aggregate.Allocated += bytes;
                else aggregate.AllocationKnown = false;
            }
        }
        ct.ThrowIfCancellationRequested();
        return new(Guid.NewGuid(), root, started, DateTimeOffset.UtcNow, true,
            directories.OrderBy(d => d.Key, StringComparer.OrdinalIgnoreCase)
                .Select(d => new DirectoryObservation(d.Key, d.Value.Logical,
                    d.Value.AllocationKnown ? d.Value.Allocated : null, d.Value.Complete)).ToArray(),
            largest.ToArray(), issues.ToArray()) { Tree = tree.Build() };
    }

    private static int ComparePaths(string first, string second)
    {
        var result = StringComparer.OrdinalIgnoreCase.Compare(first, second);
        return result != 0 ? result : StringComparer.Ordinal.Compare(first, second);
    }

    private static int CompareLargest(FileObservation first, FileObservation second)
    {
        var result = second.LogicalBytes.CompareTo(first.LogicalBytes);
        return result != 0 ? result : ComparePaths(first.Path, second.Path);
    }

    private sealed class Aggregate
    {
        public long Logical, Allocated;
        public bool Complete = true, AllocationKnown = true;
    }

    private sealed class PhysicalFile(string path, long? allocatedBytes, int treeIndex)
    {
        public string Path = path;
        public int TreeIndex = treeIndex;
        public long? AllocatedBytes = allocatedBytes;
    }
}
