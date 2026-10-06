namespace DiskBurrow.Core.Cleanup;

public interface ICleanupPlanner
{
    Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excludedPaths, CancellationToken ct);
}
