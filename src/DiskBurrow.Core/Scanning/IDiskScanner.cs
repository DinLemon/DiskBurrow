namespace DiskBurrow.Core.Scanning;

public interface IDiskScanner
{
    Task<ScanSnapshot> ScanAsync(string root, IProgress<ScanProgress>? progress, CancellationToken cancellationToken);
}
