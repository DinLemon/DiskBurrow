using DiskBurrow.Core.Scanning;
using DiskBurrow.Storage;
using Microsoft.Data.Sqlite;
using Xunit.Abstractions;

namespace DiskBurrow.Tests;

public sealed class StorageBudgetTests(ITestOutputHelper output)
{
    [Fact]
    public async Task BudgetIncludesDatabaseSidecarsAndLogs()
    {
        using var fixture = new TempTree();
        var budget = new StorageBudget(fixture.Root, 256 * 1024, 32 * 1024);
        var store = new SqliteHistoryStore(budget);
        Assert.True((await store.SaveAsync(HistoryTests.Snapshot(), default)).Saved);
        fixture.FileAt("history.db-wal", 80 * 1024);
        fixture.FileAt("history.db-shm", 80 * 1024);
        fixture.FileAt("history.db-journal", 24 * 1024);
        fixture.FileAt("sqlite-temp", 4 * 1024);
        fixture.FileAt("old.log", 32 * 1024);
        var before = Bytes(fixture.Root);
        output.WriteLine($"Database+sidecars+temp+logs: {before} bytes; limit: 262144 bytes.");
        Assert.Equal(before, budget.UsedBytes);
        Assert.False((await store.SaveAsync(HistoryTests.Snapshot(), default)).Saved);
        Assert.False(await budget.AppendLogAsync("Cannot add another log", default));
        Assert.Equal(before, Bytes(fixture.Root));
        Assert.True(before <= 256 * 1024);
        foreach (var filename in new[] { "history.db-wal", "history.db-shm", "history.db-journal", "sqlite-temp", "old.log" })
            File.Delete(Path.Combine(fixture.Root, filename));
        Assert.Single(await store.LoadRecentAsync(@"C:\", 30, default));
    }

    [Fact]
    public async Task InsertionFailureAfterRetentionPruningRollsBackAllOldSnapshots()
    {
        using var fixture=new TempTree();var store=new SqliteHistoryStore(new(fixture.Root));
        var snapshots=Enumerable.Range(0,30).Select(i=>HistoryTests.Snapshot(i)).ToArray();
        foreach(var snapshot in snapshots)Assert.True((await store.SaveAsync(snapshot,default)).Saved);
        using(var db=new SqliteConnection($"Data Source={store.DatabasePath};Pooling=False"))
        {
            db.Open();using var command=db.CreateCommand();
            // Count 29 proves this abort executes after retention pruning, not preflight.
            command.CommandText="CREATE TRIGGER fail_insert AFTER INSERT ON snapshots WHEN (SELECT COUNT(*) FROM snapshots)=30 BEGIN SELECT RAISE(ABORT,'insertion fixture after pruning'); END";
            command.ExecuteNonQuery();
        }
        Assert.False((await store.SaveAsync(HistoryTests.Snapshot(31),default)).Saved);
        var reopened=await new SqliteHistoryStore(new(fixture.Root)).LoadRecentAsync(@"C:\",30,default);
        Assert.Equal(snapshots.Select(s=>s.Id).Order(),reopened.Select(s=>s.Id).Order());
        Assert.Equal(snapshots.Last().Id,store.LastSuccessfulSnapshot?.Id);
    }
    [Fact]
    public async Task OversizeSnapshotIsRejectedWithoutUnboundedGrowth()
    {
        using var fixture = new TempTree();
        var budget = new StorageBudget(fixture.Root, 256 * 1024, 16 * 1024);
        var store = new SqliteHistoryStore(budget);
        var good = HistoryTests.Snapshot();
        Assert.True((await store.SaveAsync(good, default)).Saved);
        var oversized = HistoryTests.Snapshot(directories: Enumerable.Range(0, 4000)
            .Select(i => new DirectoryObservation($@"C:\{new string('x', 200)}\{i}", 1, 1, true)).ToArray());
        var before = Bytes(fixture.Root);
        for (var i = 0; i < 5; i++) Assert.False((await store.SaveAsync(oversized, default)).Saved);
        Assert.Equal(before, Bytes(fixture.Root));
        Assert.Equal(good.Id, (await store.LoadRecentAsync(@"C:\", 30, default)).Single().Id);
        Assert.Same(good, store.LastSuccessfulSnapshot);
    }

    [Fact]
    public async Task TenMiBLogsRotate()
    {
        using var fixture = new TempTree();
        var budget = new StorageBudget(fixture.Root);
        Assert.Equal(250L * 1024 * 1024, budget.TotalLimitBytes);
        Assert.Equal(10L * 1024 * 1024, budget.LogLimitBytes);
        var block = new string('a', 1024 * 1024);
        for (var i = 0; i < 12; i++) Assert.True(await budget.AppendLogAsync(block, default));
        Assert.True(await budget.AppendLogAsync("latest message", default));
        Assert.InRange(Bytes(fixture.Root), 1, 10L * 1024 * 1024);
        Assert.Contains("latest message", await File.ReadAllTextAsync(Path.Combine(fixture.Root, "diskburrow.log")));
        output.WriteLine($"Rotated logs: {Bytes(fixture.Root)} bytes; log limit: 10485760 bytes.");
    }

    [Fact]
    public async Task RestartKeepsPreviousCompleteHistory()
    {
        using var fixture = new TempTree();
        var budget = new StorageBudget(fixture.Root, 256 * 1024, 16 * 1024);
        var store = new SqliteHistoryStore(budget);
        var good = HistoryTests.Snapshot();
        Assert.True((await store.SaveAsync(good, default)).Saved);
        Assert.False((await store.SaveAsync(HistoryTests.Snapshot(completed: false), default)).Saved);
        var restarted = new SqliteHistoryStore(new(fixture.Root, 256 * 1024, 16 * 1024));
        Assert.Equal(good.Id, (await restarted.LoadRecentAsync(@"C:\", 30, default)).Single().Id);
        using var connection = new SqliteConnection($"Data Source={store.DatabasePath};Pooling=False");
        connection.Open();
        using var command = connection.CreateCommand();
        command.CommandText = "PRAGMA journal_mode";
        Assert.Equal("delete", command.ExecuteScalar());
        command.CommandText = "PRAGMA auto_vacuum";
        Assert.Equal(2L, command.ExecuteScalar());
        Assert.DoesNotContain(Directory.EnumerateFiles(fixture.Root), f => f.EndsWith("-wal", StringComparison.Ordinal));
    }

    [Fact]
    public async Task LowLimitRepeatedWritesStayBoundedDuringJournalWrites()
    {
        using var fixture = new TempTree();
        var budget = new StorageBudget(fixture.Root, 384 * 1024, 16 * 1024);
        var store = new SqliteHistoryStore(budget);
        var maximum = 0L;
        var journalSamples = 0;
        using var stop = new CancellationTokenSource();
        var observer = Task.Run(async () =>
        {
            while (!stop.IsCancellationRequested)
            {
                Interlocked.Exchange(ref maximum, Math.Max(Interlocked.Read(ref maximum), Bytes(fixture.Root)));
                if (File.Exists(store.DatabasePath + "-journal")) Interlocked.Increment(ref journalSamples);
                await Task.Delay(1);
            }
        });
        var saved = 0;
        for (var i = 0; i < 80; i++)
        {
            if ((await store.SaveAsync(HistoryTests.Snapshot(i, files: 100), default)).Saved) saved++;
            Assert.InRange(Bytes(fixture.Root), 0, 384 * 1024);
        }
        stop.Cancel();
        await observer;
        Assert.True(saved > 0);
        Assert.InRange(maximum, 0, 384 * 1024);
        Assert.NotEmpty(await new SqliteHistoryStore(budget).LoadRecentAsync(@"C:\", 30, default));
        output.WriteLine($"Saved: {saved}/80; sampled peak: {maximum} bytes; final: {Bytes(fixture.Root)} bytes; limit: 393216 bytes; DELETE journal samples: {journalSamples}.");
    }

    internal static long Bytes(string directory)
    {
        var total = 0L;
        foreach (var path in Directory.EnumerateFiles(directory, "*", SearchOption.AllDirectories))
        {
            try { total += new FileInfo(path).Length; }
            catch (FileNotFoundException) { }
        }
        return total;
    }
}
