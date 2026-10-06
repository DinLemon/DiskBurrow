using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.Scanning;

namespace DiskBurrow.Core.History;

public interface IHistoryStore
{
    Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot, CancellationToken ct);
    Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root, int limit, CancellationToken ct);
    Task AppendCleanupAsync(CleanupReport report, CancellationToken ct);
}

public sealed record HistoryWriteResult(bool Saved, string? UserMessage);
public enum FolderChangeKind { New, Removed, Changed, Unavailable }
public sealed record FolderChange(string Path, long LogicalDeltaBytes, long? AllocatedDeltaBytes,
    FolderChangeKind Kind, bool Comparable);
