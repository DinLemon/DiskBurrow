using DiskBurrow.Core.Cleanup;
using DiskBurrow.Windows.Files;
using DiskBurrow.Core.Scanning;
using System.ComponentModel;

namespace DiskBurrow.Windows.Cleanup;

public sealed class WindowsCleanupExecutor(RuleDiscovery discovery, NativeFileApi? files = null,
    TimeProvider? timeProvider = null, Func<string, long>? availableBytes = null) : ICleanupExecutor
{
    public Task<CleanupReport> ExecuteAsync(CleanupPlan plan, IReadOnlySet<Guid> selectedIds, CancellationToken ct)
    {
        var candidates = plan.Candidates.ToArray();
        var selected = selectedIds.ToHashSet();
        var ids = candidates.Select(candidate => candidate.Id).ToHashSet();
        if (ids.Count != candidates.Length || ids.Contains(Guid.Empty) || selected.Any(id => !ids.Contains(id)))
            throw new ArgumentException("Selected IDs must uniquely identify candidates in the reviewed plan.", nameof(selectedIds));
        // Do not cancel Task.Run itself: callers must receive outcomes already produced by deletion.
        return Task.Run(() => Execute(plan.Id, candidates.Where(c => selected.Contains(c.Id)).ToArray(), ct).WithAudit(plan));
    }

    private CleanupReport Execute(Guid planId, IReadOnlyList<CleanupCandidate> candidates, CancellationToken ct)
    {
        var items = new List<CleanupItemResult>();
        var native = files ?? new NativeFileApi();
        var before = new Dictionary<string, long?>(StringComparer.OrdinalIgnoreCase);
        if (!ct.IsCancellationRequested && candidates.Count > 0)
        {
            var catalog = discovery.Discover();
            foreach (var candidate in candidates)
            {
                if (ct.IsCancellationRequested) break;
                if (!IsAuthorized(candidate, catalog, out var path))
                {
                    items.Add(new(candidate.Id, CleanupOutcome.SkippedPolicy, "Cleanup.RuleUnavailable"));
                    continue;
                }
                var volume = Path.GetPathRoot(path)!;
                if (!before.ContainsKey(volume)) before[volume] = AvailableBytes(volume);
                try { items.Add(ExecuteCandidate(candidate, path, native, ct)); }
                catch (OperationCanceledException) when (ct.IsCancellationRequested) { break; }
                catch (InvalidDataException) { items.Add(new(candidate.Id, CleanupOutcome.SkippedPolicy, "Cleanup.UnsafePath")); }
                catch (Exception error) when (error is IOException or UnauthorizedAccessException)
                {
                    var outcome = NativeError(error) switch
                    {
                        2 or 3 => CleanupOutcome.Missing,
                        5 or 32 or 33 => CleanupOutcome.SkippedBusy,
                        _ => CleanupOutcome.Failed
                    };
                    items.Add(new(candidate.Id, outcome, "Cleanup." + outcome));
                }
            }
        }
        // All target and ancestor handles are closed before the second observation.
        var delta = 0L;
        var measured = before.Count > 0;
        foreach (var entry in before)
        {
            var after = AvailableBytes(entry.Key);
            if (after is null || entry.Value is null) { measured = false; continue; }
            try { delta = checked(delta + checked(after.Value - entry.Value.Value)); }
            catch (OverflowException) { measured = false; }
        }
        return new(planId, items.ToArray(), measured ? delta : 0)
        { WasCancelled = ct.IsCancellationRequested, FreeSpaceDeltaAvailable = measured };
    }

    private bool IsAuthorized(CleanupCandidate candidate, RuleDiscoveryResult catalog, out string path)
    {
        if (!CleanupPaths.TryNormalize(candidate.File.Path, out path) ||
            !CleanupPaths.TryNormalize(candidate.Rule.Path, out var root) ||
            !CleanupPaths.IsWithin(path, root) || CleanupPaths.EqualsPath(path, root) ||
            !discovery.IsLocalPath(path) || candidate.File.Identity is null || candidate.File.LogicalBytes < 0 ||
            UnsafeFile(candidate.File.Attributes)) return false;
        if (candidate.Rule.RuleId == "CrashDumps" && !Path.GetExtension(path).Equals(".dmp", StringComparison.OrdinalIgnoreCase)) return false;
        return catalog.Roots.Any(rule => rule.RuleId == candidate.Rule.RuleId && CleanupPaths.EqualsPath(rule.Path, root) &&
            rule.MinimumAge == candidate.Rule.MinimumAge && rule.OwnerProcessName == candidate.Rule.OwnerProcessName);
    }

    private CleanupItemResult ExecuteCandidate(CleanupCandidate candidate, string path, NativeFileApi native, CancellationToken ct)
    {
        ct.ThrowIfCancellationRequested();
        using var lease = AncestorLease.OpenVerified(candidate.Rule, path, native);
        ct.ThrowIfCancellationRequested();
        using var target = native.OpenCleanupTarget(path);
        var initial = Validate(candidate, native.InspectHandle(target, path));
        if (initial is not null) return initial;
        if (!CleanupPaths.TryNormalize(native.GetFinalPath(target), out var final) || !CleanupPaths.EqualsPath(path, final) ||
            candidate.File.Identity!.Volume != lease.Volume) return new(candidate.Id, CleanupOutcome.SkippedPolicy, "Cleanup.UnsafePath");
        lease.Verify();
        if (!discovery.IsLocalPath(path)) return new(candidate.Id, CleanupOutcome.SkippedPolicy, "Cleanup.NonLocalVolume");
        if (candidate.Rule.OwnerProcessName is { } owner && discovery.CheckOwnerProcess(owner) != OwnerProcessState.Closed)
            return new(candidate.Id, CleanupOutcome.SkippedPolicy, "Cleanup.OwnerUnavailable");
        // Sharing protects names/data access, not an atomic snapshot of attributes, timestamps or process state.
        // Re-read the same handle immediately before disposition; never fall back to deleting a path.
        var latest = Validate(candidate, native.InspectHandle(target, path));
        if (latest is not null) return latest;
        ct.ThrowIfCancellationRequested();
        native.MarkForDeletion(target);
        return new(candidate.Id, CleanupOutcome.Deleted, null);
    }

    private CleanupItemResult? Validate(CleanupCandidate candidate, FileObservation observed)
    {
        if (UnsafeFile(observed.Attributes)) return new(candidate.Id, CleanupOutcome.SkippedPolicy, "Cleanup.UnsafeFile");
        if (observed.Identity is null || observed.Identity != candidate.File.Identity || observed.LogicalBytes != candidate.File.LogicalBytes ||
            observed.ModifiedUtc != candidate.File.ModifiedUtc)
            return new(candidate.Id, CleanupOutcome.SkippedChanged, "Cleanup.FileChanged");
        if (candidate.Rule.MinimumAge is { } age && observed.ModifiedUtc >= (timeProvider ?? TimeProvider.System).GetUtcNow() - age)
            return new(candidate.Id, CleanupOutcome.SkippedPolicy, "Cleanup.TooRecent");
        return null;
    }

    private static bool UnsafeFile(FileAttributes attributes) =>
        (attributes & (FileAttributes.Directory | FileAttributes.ReparsePoint | FileAttributes.ReadOnly)) != 0 || NativeFileApi.IsCloud(attributes);
    private static int NativeError(Exception error) => error.InnerException is Win32Exception native ? native.NativeErrorCode : error.HResult & 0xFFFF;
    private long? AvailableBytes(string volume)
    {
        try
        {
            var observed = availableBytes is null ? new DriveInfo(volume).AvailableFreeSpace : availableBytes(volume);
            return observed >= 0 ? observed : null;
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or ArgumentException) { return null; }
    }
}
