using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.History;

namespace DiskBurrow.Core.Monitoring;

public enum AlertKind { LowSpace, FolderGrowth }
public sealed record AlertEvent(AlertKind Kind, string DestinationPath, long Bytes, Guid? SnapshotId,
    string? Root = null, Guid? PreviousSnapshotId = null);
public sealed class AlertSuppressionState
{
    public HashSet<string> LowSpaceRoots { get; set; } = new(StringComparer.OrdinalIgnoreCase);
    public Dictionary<string, DateTimeOffset> LastGrowthNotifiedUtc { get; set; } = new(StringComparer.OrdinalIgnoreCase);
    public Dictionary<string, Guid> LastGrowthSnapshot { get; set; } = new(StringComparer.OrdinalIgnoreCase);
    // The history retains 30 scans: remember evaluated scans even when an alert was suppressed.
    public Dictionary<string, List<Guid>> EvaluatedGrowthSnapshots { get; set; } = new(StringComparer.OrdinalIgnoreCase);
}

public sealed class AlertPolicy
{
    private readonly AppSettings _settings;
    private readonly string _systemRoot;
    public AlertSuppressionState State { get; }

    public AlertPolicy(AppSettings settings, string systemRoot, AlertSuppressionState? state = null)
    {
        settings.Validate();
        _settings = settings;
        _systemRoot = SnapshotComparer.NormalizePath(systemRoot);
        State = state ?? new();
        State.LowSpaceRoots = new(State.LowSpaceRoots.Select(SnapshotComparer.NormalizePath), StringComparer.OrdinalIgnoreCase);
        State.LastGrowthNotifiedUtc = State.LastGrowthNotifiedUtc.ToDictionary(
            p => SnapshotComparer.NormalizePath(p.Key), p => p.Value, StringComparer.OrdinalIgnoreCase);
        State.LastGrowthSnapshot = State.LastGrowthSnapshot.ToDictionary(
            p => SnapshotComparer.NormalizePath(p.Key), p => p.Value, StringComparer.OrdinalIgnoreCase);
        State.EvaluatedGrowthSnapshots = State.EvaluatedGrowthSnapshots.ToDictionary(
            p => SnapshotComparer.NormalizePath(p.Key), p => p.Value, StringComparer.OrdinalIgnoreCase);
    }

    public IReadOnlyList<AlertEvent> Evaluate(ScanSnapshot? previous, ScanSnapshot? current, long freeBytes, DateTimeOffset nowUtc)
    {
        var alerts = new List<AlertEvent>();
        var root = current is null ? _systemRoot : SnapshotComparer.NormalizePath(current.Root);
        if (freeBytes < 0) throw new ArgumentOutOfRangeException(nameof(freeBytes));
        if (freeBytes >= _settings.LowSpaceBytes) State.LowSpaceRoots.Remove(root);
        else if (State.LowSpaceRoots.Add(root)) alerts.Add(new(AlertKind.LowSpace, root, freeBytes, current?.Id, root));
        if (previous is null || current is null || !previous.TraversalCompleted || !current.TraversalCompleted) return alerts;
        if (!State.EvaluatedGrowthSnapshots.TryGetValue(root, out var evaluated))
            State.EvaluatedGrowthSnapshots[root] = evaluated = [];
        if (evaluated.Contains(current.Id)) return alerts;
        evaluated.Add(current.Id);
        if (evaluated.Count > 30) evaluated.RemoveAt(0);
        foreach (var change in new SnapshotComparer().Compare(previous, current))
        {
            if (!change.Comparable || change.LogicalDeltaBytes < _settings.GrowthBytes ||
                State.LastGrowthSnapshot.GetValueOrDefault(change.Path) == current.Id ||
                (State.LastGrowthNotifiedUtc.TryGetValue(change.Path, out var last) && nowUtc - last < TimeSpan.FromDays(1))) continue;
            State.LastGrowthNotifiedUtc[change.Path] = nowUtc;
            State.LastGrowthSnapshot[change.Path] = current.Id;
            alerts.Add(new(AlertKind.FolderGrowth, change.Path, change.LogicalDeltaBytes, current.Id, root, previous.Id));
        }
        return alerts;
    }
}
