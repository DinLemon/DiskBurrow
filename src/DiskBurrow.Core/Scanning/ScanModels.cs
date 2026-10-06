namespace DiskBurrow.Core.Scanning;

public sealed record FileIdentity(ulong Volume, string FileId);
public sealed record FileObservation(string Path, FileIdentity? Identity, long LogicalBytes,
    long? AllocatedBytes, DateTimeOffset ModifiedUtc, int LinkCount, FileAttributes Attributes);
public sealed record DirectoryObservation(string Path, long LogicalBytes, long? AllocatedBytes, bool CoverageComplete);
public sealed record ScanSnapshot(Guid Id, string Root, DateTimeOffset StartedUtc, DateTimeOffset CompletedUtc,
    bool TraversalCompleted, IReadOnlyList<DirectoryObservation> Directories,
    IReadOnlyList<FileObservation> LargestFiles, IReadOnlyList<ScanIssue> Issues);
public sealed record ScanProgress(long FilesVisited, long DirectoriesVisited, string CurrentPath);
public sealed record ScanIssue(string Path, ScanIssueKind Kind);
public enum ScanIssueKind { AccessDenied, ReparseSkipped, CloudSkipped, MetadataUnavailable, ChangedDuringScan, IoFailure }
