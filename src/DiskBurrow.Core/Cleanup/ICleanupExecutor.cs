namespace DiskBurrow.Core.Cleanup;

public interface ICleanupExecutor
{
    Task<CleanupReport> ExecuteAsync(CleanupPlan plan, IReadOnlySet<Guid> selectedIds, CancellationToken ct);
}
