using System.Collections.Concurrent;
using System.Text;

namespace DiskBurrow.Storage;

public sealed class StorageBudget
{
    private static readonly ConcurrentDictionary<string, SemaphoreSlim> Gates = new(StringComparer.OrdinalIgnoreCase);
    public string DirectoryPath { get; }
    public long TotalLimitBytes { get; }
    public long LogLimitBytes { get; }
    internal SemaphoreSlim Gate { get; }

    public StorageBudget(string directory, long totalLimitBytes = 250L * 1024 * 1024,
        long logLimitBytes = 10L * 1024 * 1024)
    {
        ArgumentOutOfRangeException.ThrowIfNegativeOrZero(totalLimitBytes);
        ArgumentOutOfRangeException.ThrowIfNegative(logLimitBytes);
        DirectoryPath = Path.TrimEndingDirectorySeparator(Path.GetFullPath(directory));
        TotalLimitBytes = totalLimitBytes;
        LogLimitBytes = Math.Min(logLimitBytes, totalLimitBytes);
        Gate = Gates.GetOrAdd(DirectoryPath, _ => new(1, 1));
    }

    // The directory is dedicated to local application storage. Every file counts, including unknown sidecars.
    public long UsedBytes => Files().Sum(f => f.Length);
    private IEnumerable<FileInfo> Files() => Directory.Exists(DirectoryPath)
        ? new DirectoryInfo(DirectoryPath).EnumerateFiles("*", SearchOption.AllDirectories) : [];

    internal long DatabaseCapacity(string database, long pageSize)
    {
        var files = Files().ToArray();
        var others = files.Where(f => !f.FullName.Equals(database, StringComparison.OrdinalIgnoreCase)).Sum(f => f.Length);
        var logs = files.Where(f => f.Extension.Equals(".log", StringComparison.OrdinalIgnoreCase)).Sum(f => f.Length);
        // Reserve the entire log allowance, a database-sized rollback journal (8 bytes per page),
        // and 64 KiB for journal sector headers. temp_store=MEMORY prevents disk sort/temp copies.
        var available = TotalLimitBytes - others - Math.Max(0, LogLimitBytes - logs) - 64 * 1024;
        return Math.Max(0, available / (2 * pageSize + 8)) * pageSize;
    }

    public async Task<bool> AppendLogAsync(string message, CancellationToken ct)
    {
        await Gate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            ct.ThrowIfCancellationRequested();
            var bytes = Encoding.UTF8.GetBytes(message + Environment.NewLine);
            var path = Path.Combine(DirectoryPath, "diskburrow.log");
            var files = Files().ToArray();
            var current = files.FirstOrDefault(f => f.FullName.Equals(path, StringComparison.OrdinalIgnoreCase))?.Length ?? 0;
            var otherLogs = files.Where(f => f.Extension.Equals(".log", StringComparison.OrdinalIgnoreCase) &&
                !f.FullName.Equals(path, StringComparison.OrdinalIgnoreCase)).Sum(f => f.Length);
            var allowance = Math.Min(LogLimitBytes - otherLogs, TotalLimitBytes - files.Sum(f => f.Length) + current);
            if (bytes.LongLength > allowance) return false;
            Directory.CreateDirectory(DirectoryPath);
            using var stream = new FileStream(path, FileMode.OpenOrCreate, FileAccess.Write, FileShare.Read);
            // Rotate in place: a second log or temporary copy would consume the shared budget.
            if (stream.Length + bytes.LongLength > allowance) stream.SetLength(0);
            stream.Position = stream.Length;
            await stream.WriteAsync(bytes, ct).ConfigureAwait(false);
            await stream.FlushAsync(ct).ConfigureAwait(false);
            return true;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException) { return false; }
        finally { Gate.Release(); }
    }
}
