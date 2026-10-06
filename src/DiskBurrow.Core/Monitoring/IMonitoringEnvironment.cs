namespace DiskBurrow.Core.Monitoring;

public sealed record VolumeSpace(long TotalBytes, long FreeBytes)
{
    public long UsedBytes => TotalBytes - FreeBytes;
}
public sealed record VolumeObservation(string Root, VolumeSpace Space, DateTimeOffset ObservedUtc);

public interface IMonitoringEnvironment
{
    string SystemRoot { get; }
    bool IsOnBattery { get; }
    long GetFreeBytes(string root);
    VolumeSpace GetVolumeSpace(string root);
    bool IsLocalRoot(string root);
}

public interface IMonitoringStateStore
{
    string? LastUserMessage { get; }
    Task<AlertSuppressionState> LoadAlertStateAsync(CancellationToken ct);
    Task SaveAlertStateAsync(AlertSuppressionState state, CancellationToken ct);
}
