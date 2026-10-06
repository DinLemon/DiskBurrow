using System.Collections.Concurrent;
using System.Collections.ObjectModel;
using System.ComponentModel;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Windows.Files;

namespace DiskBurrow.Windows.Cleanup;

/// <summary>Explicit user-selected deletion, independent of cache-rule authorization and age rules.</summary>
public sealed class WindowsManualDeletionService : IManualDeletionService
{
    private readonly CleanupKnownDirectories known;
    private readonly string applicationDirectory;
    private readonly string dataDirectory;
    private readonly NativeFileApi files;
    private readonly ConcurrentDictionary<Guid, IssuedPlan> issued = new();
    private readonly SemaphoreSlim execution = new(1, 1);
    private long previewGeneration;
    private readonly object previewAuthority = new();
    private sealed record IssuedPlan(ManualDeletePlan Plan, IReadOnlyDictionary<string, FileIdentity> Ancestors);

    public WindowsManualDeletionService(CleanupKnownDirectories known, string applicationDirectory,
        string dataDirectory, NativeFileApi? files = null)
    {
        this.known = known with { UserLibraryRoots = Array.AsReadOnly(known.UserLibraryRoots.ToArray()) };
        this.applicationDirectory = applicationDirectory;
        this.dataDirectory = dataDirectory;
        this.files = files ?? new NativeFileApi();
    }

    public Task<ManualDeletePlan> PreviewAsync(IEnumerable<string> selectedPaths, CancellationToken ct)
    {
        var selections = selectedPaths.ToArray();
        long generation;
        lock (previewAuthority) { generation = ++previewGeneration; issued.Clear(); }
        return Task.Run(() => Preview(selections, generation, ct), ct);
    }

    public async Task<CleanupReport> ExecuteConfirmedAsync(Guid reviewedPlanId, bool permanentDeletionConfirmed, CancellationToken ct)
    {
        if (!permanentDeletionConfirmed) throw new InvalidOperationException("Manual.ConfirmationRequired");
        // A real attempt consumes the ID, including cancellation and partial failure. Never automatically replay a plan.
        if (!issued.TryRemove(reviewedPlanId, out var approved) || !approved.Plan.CanExecute)
            throw new InvalidOperationException("Manual.PlanUnavailable");
        await execution.WaitAsync(CancellationToken.None).ConfigureAwait(false);
        try { return await Task.Run(() => Execute(approved, ct)).ConfigureAwait(false); }
        finally { execution.Release(); }
    }

    private ManualDeletePlan Preview(string[] selections, long generation, CancellationToken ct)
    {
        var warnings = new List<ManualDeleteWarning>();
        var roots = new List<string>();
        foreach (var selected in selections)
        {
            ct.ThrowIfCancellationRequested();
            if (!Canonical(selected, out var path) || !Allowed(path))
            { warnings.Add(new(selected, "Manual.ProtectedPath")); continue; }
            if (!roots.Contains(path, StringComparer.OrdinalIgnoreCase)) roots.Add(path);
        }
        roots = roots.Where(path => !roots.Any(other => !CleanupPaths.EqualsPath(path, other) && CleanupPaths.IsWithin(path, other)))
            .OrderBy(path => path, StringComparer.OrdinalIgnoreCase).ToList();
        var entries = new List<ManualDeleteEntry>();
        var ancestors = new Dictionary<string, FileIdentity>(StringComparer.OrdinalIgnoreCase);
        foreach (var root in roots)
        {
            ct.ThrowIfCancellationRequested();
            try
            {
                using var lease = ParentLease(root);
                foreach (var ancestor in lease.Identities)
                {
                    if (ancestors.TryGetValue(ancestor.Key, out var previous) && previous != ancestor.Value)
                        throw new InvalidDataException("Manual.Changed");
                    ancestors[ancestor.Key] = ancestor.Value;
                }
                Inventory(root, root, entries, ancestors, ct);
                lease.Verify();
            }
            catch (OperationCanceledException) when (ct.IsCancellationRequested) { throw; }
            catch (Exception error) when (Expected(error)) { warnings.Add(new(error.Data["ManualPath"] as string ?? root, Reason(error))); }
        }
        var plan = new ManualDeletePlan(Guid.NewGuid(), DateTimeOffset.UtcNow, ReadOnly(roots), ReadOnly(entries), ReadOnly(warnings));
        // Keep only the current preview. The app requests a fresh preview after changing a selection.
        lock (previewAuthority)
            if (plan.CanExecute && generation == previewGeneration)
                issued[plan.Id] = new(plan, new ReadOnlyDictionary<string, FileIdentity>(ancestors));
        return plan;
    }

    private void Inventory(string root, string path, List<ManualDeleteEntry> entries,
        Dictionary<string, FileIdentity> ancestors, CancellationToken ct)
    {
        // Iterative traversal avoids stack overflow for deeply nested user trees.
        var pending = new Stack<string>();
        pending.Push(path);
        var inspecting = path;
        try
        {
            while (pending.TryPop(out var current))
            {
                inspecting = current;
                ct.ThrowIfCancellationRequested();
                if (!Allowed(current)) throw new InvalidDataException("Manual.ProtectedPath");
                using var lease = ParentLease(current);
                foreach (var ancestor in lease.Identities)
                {
                    if (ancestors.TryGetValue(ancestor.Key, out var identity) && identity != ancestor.Value)
                        throw new InvalidDataException("Manual.Changed");
                    ancestors[ancestor.Key] = ancestor.Value;
                }
                using var handle = files.OpenMetadata(current);
                var observation = files.InspectHandle(handle, current);
                EnsureSafe(current, observation, files.GetFinalPath(handle));
                entries.Add(new(Guid.NewGuid(), root, observation));
                if (observation.Attributes.HasFlag(FileAttributes.Directory))
                {
                    ancestors[current] = observation.Identity!;
                    // Pin directory identity before resolving any children. The metadata handle itself allows sharing.
                    using var directory = files.OpenCleanupDirectory(current);
                    var pinned = files.InspectHandle(directory, current);
                    EnsureSafe(current, pinned, files.GetFinalPath(directory));
                    if (pinned.Identity != observation.Identity) throw new InvalidDataException("Manual.Changed");
                    foreach (var child in Directory.EnumerateFileSystemEntries(current)) pending.Push(child);
                }
                lease.Verify();
            }
        }
        catch (Exception error) when (Expected(error)) { error.Data["ManualPath"] = inspecting; throw; }
    }

    private CleanupReport Execute(IssuedPlan approved, CancellationToken ct)
    {
        var results = new List<CleanupItemResult>();
        foreach (var root in approved.Plan.Roots)
        {
            var entries = approved.Plan.Entries.Where(entry => CleanupPaths.EqualsPath(entry.RootPath, root)).ToArray();
            if (ct.IsCancellationRequested)
            { results.AddRange(entries.Select(entry => Result(entry, CleanupOutcome.SkippedPolicy, "Manual.Cancelled"))); continue; }
            try { RevalidateInventory(root, entries, approved.Ancestors, ct); }
            catch (OperationCanceledException) when (ct.IsCancellationRequested)
            { results.AddRange(entries.Select(entry => Result(entry, CleanupOutcome.SkippedPolicy, "Manual.Cancelled"))); continue; }
            catch (Exception error) when (Expected(error))
            { results.AddRange(entries.Select(entry => Result(entry, Outcome(error), Reason(error)))); continue; }
            // Files first; folders deepest first. Never recursively delete an unreviewed name.
            foreach (var entry in entries.OrderBy(entry => entry.IsDirectory).ThenByDescending(entry => entry.File.Path.Length))
            {
                if (ct.IsCancellationRequested)
                { results.Add(Result(entry, CleanupOutcome.SkippedPolicy, "Manual.Cancelled")); continue; }
                try { Delete(entry, approved.Ancestors, ct); results.Add(Result(entry, CleanupOutcome.Deleted, null)); }
                catch (OperationCanceledException) when (ct.IsCancellationRequested)
                { results.Add(Result(entry, CleanupOutcome.SkippedPolicy, "Manual.Cancelled")); }
                catch (Exception error) when (Expected(error)) { results.Add(Result(entry, Outcome(error), Reason(error))); }
            }
        }
        // Logical reviewed bytes are not a measurement of newly freed volume space (hardlinks/other writers).
        return new(approved.Plan.Id, results.AsReadOnly(), 0) { WasCancelled = ct.IsCancellationRequested, FreeSpaceDeltaAvailable = false };
    }

    private void RevalidateInventory(string root, ManualDeleteEntry[] entries,
        IReadOnlyDictionary<string, FileIdentity> ancestors, CancellationToken ct)
    {
        using var lease = ParentLease(root);
        VerifyAncestors(lease, ancestors);
        var fresh = new List<ManualDeleteEntry>();
        var currentAncestors = new Dictionary<string, FileIdentity>(StringComparer.OrdinalIgnoreCase);
        Inventory(root, root, fresh, currentAncestors, ct);
        if (fresh.Count != entries.Length) throw new InvalidDataException("Manual.Changed");
        var reviewed = entries.ToDictionary(entry => entry.File.Path, StringComparer.OrdinalIgnoreCase);
        foreach (var item in fresh)
            if (!reviewed.TryGetValue(item.File.Path, out var original) || !Same(original.File, item.File, true))
                throw new InvalidDataException("Manual.Changed");
        foreach (var ancestor in currentAncestors)
            if (!ancestors.TryGetValue(ancestor.Key, out var identity) || ancestor.Value != identity)
                throw new InvalidDataException("Manual.Changed");
        lease.Verify();
    }

    private void Delete(ManualDeleteEntry entry, IReadOnlyDictionary<string, FileIdentity> ancestors, CancellationToken ct)
    {
        var path = entry.File.Path;
        if (!Allowed(path)) throw new InvalidDataException("Manual.ProtectedPath");
        using var lease = ParentLease(path);
        VerifyAncestors(lease, ancestors);
        ct.ThrowIfCancellationRequested();
        using var target = files.OpenCleanupTarget(path);
        var current = files.InspectHandle(target, path);
        EnsureSafe(path, current, files.GetFinalPath(target));
        // Parent timestamps can change as our own files are deleted. Directory identity/type remain mandatory.
        if (!Same(entry.File, current, !entry.IsDirectory)) throw new InvalidDataException("Manual.Changed");
        if (current.Identity!.Volume != lease.Volume) throw new InvalidDataException("Manual.UnsafePath");
        if (entry.IsDirectory && Directory.EnumerateFileSystemEntries(path).Any()) throw new InvalidDataException("Manual.NotEmpty");
        lease.Verify();
        var latest = files.InspectHandle(target, path);
        EnsureSafe(path, latest, files.GetFinalPath(target));
        if (!Same(entry.File, latest, !entry.IsDirectory)) throw new InvalidDataException("Manual.Changed");
        ct.ThrowIfCancellationRequested();
        // Kernel disposition on this exact handle refuses non-empty folders, including late-arriving children.
        files.MarkForDeletion(target);
    }

    private AncestorLease ParentLease(string path) => AncestorLease.OpenVerified(new("ManualSelection", Path.GetPathRoot(path)!, null, null), path, files);
    private static void VerifyAncestors(AncestorLease lease, IReadOnlyDictionary<string, FileIdentity> expected)
    {
        foreach (var ancestor in lease.Identities)
            if (!expected.TryGetValue(ancestor.Key, out var identity) || identity != ancestor.Value)
                throw new InvalidDataException("Manual.Changed");
        lease.Verify();
    }

    private bool Allowed(string path)
    {
        if (!known.UserLibraryRootsVerified || !Canonical(path, out _) || CleanupPaths.EqualsPath(path, Path.GetPathRoot(path)!)) return false;
        try { if (new DriveInfo(Path.GetPathRoot(path)!).DriveType is not (DriveType.Fixed or DriveType.Removable or DriveType.Ram)) return false; }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or ArgumentException) { return false; }
        // Fail closed if a protected location cannot be resolved.
        foreach (var protectedPath in new[] { known.Windows, known.ProgramFiles, known.ProgramFilesX86, known.ProgramData,
                     applicationDirectory, dataDirectory })
        {
            if (!Canonical(protectedPath, out var boundary)) return false;
            if (CleanupPaths.IsWithin(path, boundary) || CleanupPaths.IsWithin(boundary, path)) return false;
        }
        foreach (var container in new[] { known.UserProfile, known.LocalAppData, Path.GetDirectoryName(known.LocalAppData)!,
                     Path.Combine(known.UserProfile, "AppData", "Roaming") }.Concat(known.UserLibraryRoots))
        {
            if (!Canonical(container, out var boundary)) return false;
            if (CleanupPaths.IsWithin(boundary, path)) return false;
        }
        return true;
    }

    private static bool Canonical(string? path, out string normalized)
    {
        if (!CleanupPaths.TryNormalize(path, out normalized)) return false;
        // The reviewed path must already be canonical, including no dot segments or mixed separators.
        return CleanupPaths.EqualsPath(Path.TrimEndingDirectorySeparator(path!), normalized);
    }
    private static bool Same(FileObservation reviewed, FileObservation current, bool compareData) =>
        reviewed.Identity == current.Identity && reviewed.Attributes.HasFlag(FileAttributes.Directory) == current.Attributes.HasFlag(FileAttributes.Directory) &&
        (!compareData || (reviewed.LogicalBytes == current.LogicalBytes && reviewed.ModifiedUtc == current.ModifiedUtc));
    private static void EnsureSafe(string path, FileObservation observation, string final)
    {
        if (observation.Identity is null || observation.LogicalBytes < 0 ||
            (observation.Attributes & (FileAttributes.ReparsePoint | FileAttributes.ReadOnly | FileAttributes.System)) != 0 ||
            NativeFileApi.IsCloud(observation.Attributes) || !Canonical(final, out _) || !CleanupPaths.EqualsPath(path, final))
            throw new InvalidDataException("Manual.UnsafePath");
    }
    private static ReadOnlyCollection<T> ReadOnly<T>(IEnumerable<T> values) => Array.AsReadOnly(values.ToArray());
    private static bool Expected(Exception error) => error is IOException or UnauthorizedAccessException or InvalidDataException;
    private static int NativeError(Exception error) => error.InnerException is Win32Exception native ? native.NativeErrorCode : error.HResult & 0xFFFF;
    private static CleanupOutcome Outcome(Exception error) => error is InvalidDataException ? CleanupOutcome.SkippedChanged : NativeError(error) switch
    { 2 or 3 => CleanupOutcome.Missing, 5 or 32 or 33 => CleanupOutcome.SkippedBusy, _ => CleanupOutcome.Failed };
    private static string Reason(Exception error) => error is InvalidDataException && error.Message.StartsWith("Manual.", StringComparison.Ordinal)
        ? error.Message : "Manual.Unavailable";
    private static CleanupItemResult Result(ManualDeleteEntry entry, CleanupOutcome outcome, string? reason) =>
        new(entry.Id, outcome, reason) { Audit = new(entry.File.Path, "ManualSelection", entry.RootPath, entry.File.ModifiedUtc,
            entry.IsDirectory ? 0 : entry.File.LogicalBytes, entry.IsDirectory ? null : entry.File.AllocatedBytes, entry.File.LinkCount, entry.File.Identity) };
}
