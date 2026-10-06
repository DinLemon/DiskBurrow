namespace DiskBurrow.Core.Cleanup;

public sealed record RuleRoot(string RuleId, string Path, TimeSpan? MinimumAge, string? OwnerProcessName);
public sealed record CleanupCandidate(Guid Id, RuleRoot Rule, DiskBurrow.Core.Scanning.FileObservation File, string ReasonKey);
public sealed record CleanupWarning(string RuleId, string? Path, string ReasonKey);
public sealed record CleanupPlan(Guid Id, DateTimeOffset CreatedUtc, IReadOnlyList<CleanupCandidate> Candidates)
{
    public IReadOnlyList<CleanupWarning> Warnings { get; init; } = [];
    public IReadOnlySet<Guid> SelectedIds { get; init; } = new HashSet<Guid>();
    public long EstimatedDataBytes => Candidates.Sum(candidate => candidate.File.LogicalBytes);
}

public enum CleanupOutcome { Deleted, Missing, SkippedChanged, SkippedBusy, SkippedPolicy, Failed }
// Audit describes the reviewed candidate, not an invented post-deletion observation.
public sealed record CleanupAudit(string Path, string RuleId, string RuleRootPath,
    DateTimeOffset ReviewedModifiedUtc, long LogicalBytes, long? AllocatedBytes, int LinkCount,
    DiskBurrow.Core.Scanning.FileIdentity? Identity);
public sealed record CleanupItemResult(Guid CandidateId, CleanupOutcome Outcome, string? ReasonKey)
{
    // Null explicitly identifies older GUID-only/unmapped records.
    public CleanupAudit? Audit { get; init; }
}
public sealed record CleanupReport(Guid PlanId, IReadOnlyList<CleanupItemResult> Items, long FreeSpaceDeltaBytes)
{
    public bool WasCancelled { get; init; }
    public bool FreeSpaceDeltaAvailable { get; init; } = true;
    public CleanupReport WithAudit(CleanupPlan plan)
    {
        if (plan.Id != PlanId) throw new ArgumentException("Report must match the reviewed plan.", nameof(plan));
        var candidates=plan.Candidates.ToDictionary(c=>c.Id);
        return this with { Items=Items.Select(item=>candidates.TryGetValue(item.CandidateId,out var c)
            ? item with { Audit=new(c.File.Path,c.Rule.RuleId,c.Rule.Path,c.File.ModifiedUtc,c.File.LogicalBytes,c.File.AllocatedBytes,c.File.LinkCount,c.File.Identity) }
            : item with { Audit=null }).ToArray() };
    }
}
