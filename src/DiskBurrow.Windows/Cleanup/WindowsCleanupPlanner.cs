using DiskBurrow.Core.Cleanup;
using DiskBurrow.Windows.Files;

namespace DiskBurrow.Windows.Cleanup;

public sealed class WindowsCleanupPlanner(RuleDiscovery discovery, NativeFileApi? files = null,
    TimeProvider? timeProvider = null) : ICleanupPlanner
{
    public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excludedPaths, CancellationToken ct)
    {
        var now = (timeProvider ?? TimeProvider.System).GetUtcNow();
        return Task.Run(() => Preview(excludedPaths, now, ct), ct);
    }

    private CleanupPlan Preview(IReadOnlySet<string> excludedPaths, DateTimeOffset now, CancellationToken ct)
    {
        var native = files ?? new NativeFileApi();
        var exclusions = excludedPaths.Select(path => CleanupPaths.TryNormalize(path, out var normalized) ? normalized : null)
            .Where(path => path is not null).Cast<string>().ToArray();
        bool Excluded(string path) => exclusions.Any(root => CleanupPaths.IsWithin(path, root));
        ct.ThrowIfCancellationRequested();
        var catalog = discovery.Discover();
        var warnings = catalog.Warnings.ToList();
        var candidates = new List<CleanupCandidate>();
        var visited = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        var options = new EnumerationOptions { IgnoreInaccessible = false, AttributesToSkip = 0, RecurseSubdirectories = false };
        foreach (var rule in catalog.Roots)
        {
            var pending = new Stack<string>();
            pending.Push(rule.Path);
            while (pending.TryPop(out var directory))
            {
                ct.ThrowIfCancellationRequested();
                if (Excluded(directory)) continue;
                if (!discovery.IsLocalPath(directory))
                {
                    warnings.Add(new(rule.RuleId, directory, "Cleanup.NonLocalVolume"));
                    continue;
                }
                if (!CleanupPaths.IsSafeDirectory(directory, native))
                {
                    warnings.Add(new(rule.RuleId, directory, "Cleanup.UnsafeRoot"));
                    continue;
                }
                try
                {
                    foreach (var path in Directory.EnumerateFileSystemEntries(directory, "*", options))
                    {
                        ct.ThrowIfCancellationRequested();
                        if (!CleanupPaths.TryNormalize(path, out var normalized) || !CleanupPaths.IsWithin(normalized, rule.Path) || Excluded(normalized)) continue;
                        try
                        {
                            var observation = native.Inspect(normalized);
                            var attributes = observation.Attributes;
                            if (attributes.HasFlag(FileAttributes.ReparsePoint) || NativeFileApi.IsCloud(attributes)) continue;
                            if (attributes.HasFlag(FileAttributes.Directory)) { pending.Push(normalized); continue; }
                            if (attributes.HasFlag(FileAttributes.ReadOnly) || observation.Identity is null || observation.LogicalBytes < 0) continue;
                            if (rule.MinimumAge is { } age && observation.ModifiedUtc >= now - age) continue;
                            if (rule.RuleId == "CrashDumps" && !Path.GetExtension(normalized).Equals(".dmp", StringComparison.OrdinalIgnoreCase)) continue;
                            if (!visited.Add(normalized)) continue;
                            var reason = rule.RuleId switch { "UserTemp" => "Cleanup.OldTemp", "CrashDumps" => "Cleanup.OldCrashDump", _ => "Cleanup.BrowserCache" };
                            candidates.Add(new(Guid.NewGuid(), rule, observation, reason));
                        }
                        catch (Exception error) when (error is IOException or UnauthorizedAccessException)
                        { warnings.Add(new(rule.RuleId, normalized, "Cleanup.FileUnavailable")); }
                    }
                }
                catch (Exception error) when (error is IOException or UnauthorizedAccessException)
                { warnings.Add(new(rule.RuleId, directory, "Cleanup.RootUnavailable")); }
            }
        }
        ct.ThrowIfCancellationRequested();
        return new(Guid.NewGuid(), now, candidates.OrderBy(candidate => candidate.File.Path, StringComparer.OrdinalIgnoreCase).ToArray())
        { Warnings = warnings.Distinct().ToArray() };
    }
}
