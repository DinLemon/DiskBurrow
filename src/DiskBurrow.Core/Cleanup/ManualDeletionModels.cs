using DiskBurrow.Core.Scanning;

namespace DiskBurrow.Core.Cleanup;

public sealed record ManualDeleteEntry(Guid Id, string RootPath, FileObservation File)
{
    public bool IsDirectory => File.Attributes.HasFlag(FileAttributes.Directory);
}

public sealed record ManualDeleteWarning(string? Path, string ReasonKey);

// Collections supplied by the Windows service are read-only snapshots. Execution uses the service's own inventory.
public sealed record ManualDeletePlan(Guid Id, DateTimeOffset CreatedUtc, IReadOnlyList<string> Roots,
    IReadOnlyList<ManualDeleteEntry> Entries, IReadOnlyList<ManualDeleteWarning> Warnings)
{
    public bool CanExecute => Roots.Count > 0 && Entries.Count > 0 && Warnings.Count == 0;
    public int FileCount => Entries.Count(entry => !entry.IsDirectory);
    public int DirectoryCount => Entries.Count(entry => entry.IsDirectory);
    public long EstimatedDataBytes => Entries.Where(entry => !entry.IsDirectory).Sum(entry => entry.File.LogicalBytes);
}

public interface IManualDeletionService
{
    Task<ManualDeletePlan> PreviewAsync(IEnumerable<string> selectedPaths, CancellationToken ct);
    Task<CleanupReport> ExecuteConfirmedAsync(Guid reviewedPlanId, bool permanentDeletionConfirmed, CancellationToken ct);
}
