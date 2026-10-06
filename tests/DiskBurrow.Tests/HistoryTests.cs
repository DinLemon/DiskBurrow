using System.Text.Json;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Storage;
using Microsoft.Data.Sqlite;

namespace DiskBurrow.Tests;

public sealed class HistoryTests
{
    internal static ScanSnapshot Snapshot(long bytes = 10, bool completed = true,
        IReadOnlyList<DirectoryObservation>? directories = null, int files = 0) => new(
        Guid.NewGuid(), @"C:\", DateTimeOffset.UtcNow.AddSeconds(-1), DateTimeOffset.UtcNow, completed,
        directories ?? [new(@"C:\cache", bytes, bytes / 2, true)],
        Enumerable.Range(0, files).Select(i => new FileObservation($@"C:\cache\file{i}",
            new(1, i.ToString()), i + 1, i + 1, DateTimeOffset.UnixEpoch, 1, FileAttributes.Normal)).ToArray(),
        [new(@"C:\Windows", ScanIssueKind.AccessDenied)]);

    [Fact]
    public void GrowthAndShrinkAreCompared()
    {
        var comparer = new SnapshotComparer();
        var changes = comparer.Compare(Snapshot(10L * 1024 * 1024 * 1024), Snapshot(15L * 1024 * 1024 * 1024));
        Assert.Equal(5L * 1024 * 1024 * 1024, changes.Single().LogicalDeltaBytes);
        Assert.Equal(5L * 1024 * 1024 * 1024 / 2, changes.Single().AllocatedDeltaBytes);
        Assert.True(changes.Single().Comparable);
        Assert.Equal(-5L, comparer.Compare(Snapshot(15), Snapshot(10)).Single().LogicalDeltaBytes);
    }

    [Fact]
    public void DeniedSubtreeDoesNotBecomeDeletion()
    {
        var previous = Snapshot(directories: [new(@"C:\", 20, null, true), new(@"C:\Windows", 20, null, true), new(@"C:\Windows\child", 20, null, true)]);
        var current = Snapshot(directories: [new(@"C:\", 0, null, false), new(@"C:\Windows", 0, null, false)]);
        var changes = new SnapshotComparer().Compare(previous, current);
        Assert.Contains(changes, c => c.Path == @"C:\Windows\child" && !c.Comparable && c.Kind == FolderChangeKind.Unavailable);
        Assert.DoesNotContain(changes, c => c.Kind == FolderChangeKind.Removed);
    }

    [Fact]
    public void CoveredSiblingCanBeCompared()
    {
        var previous = Snapshot(directories: [new(@"C:\", 30, null, false), new(@"C:\Windows", 0, null, false), new(@"C:\cache", 10, 6, true)]);
        var current = Snapshot(directories: [new(@"C:\", 40, null, false), new(@"C:\Windows", 0, null, false), new(@"C:\cache", 20, null, true)]);
        var change = new SnapshotComparer().Compare(previous, current).Single(c => c.Path == @"C:\cache");
        Assert.True(change.Comparable);
        Assert.Equal(10, change.LogicalDeltaBytes);
        Assert.Null(change.AllocatedDeltaBytes);
    }

    [Fact]
    public void DisappearedFolderRequiresCoveredParent()
    {
        var previous = Snapshot(directories: [new(@"C:\", 20, 20, true), new(@"C:\gone", 20, 20, true)]);
        var comparer = new SnapshotComparer();
        Assert.Contains(comparer.Compare(previous, Snapshot(directories: [new(@"C:\", 0, 0, true)])),
            c => c.Path == @"C:\gone" && c.Kind == FolderChangeKind.Removed && c.LogicalDeltaBytes == -20);
        Assert.Contains(comparer.Compare(previous, Snapshot(directories: [new(@"C:\", 0, null, false)])),
            c => c.Path == @"C:\gone" && !c.Comparable);
    }

    [Fact]
    public void PathsAreNormalizedAndIncompleteTraversalsCannotCompare()
    {
        var old = Snapshot(directories: [new(@"c:/CACHE/./child/..", 10, 10, true)]);
        var current = Snapshot(directories: [new(@"C:\cache\", 15, 15, true)]);
        Assert.Equal(5, new SnapshotComparer().Compare(old, current).Single().LogicalDeltaBytes);
        Assert.All(new SnapshotComparer().Compare(old, current with { TraversalCompleted = false }), c => Assert.False(c.Comparable));
        Assert.Empty(new SnapshotComparer().Compare(old, current with { Root = @"D:\" }));
    }

    [Fact]
    public async Task OnlyCompletedTraversalsAreStored()
    {
        using var fixture = new TempTree();
        var store = new SqliteHistoryStore(new(fixture.Root));
        Assert.False((await store.SaveAsync(Snapshot(completed: false), default)).Saved);
        Assert.True((await store.SaveAsync(Snapshot(), default)).Saved);
        Assert.Single(await store.LoadRecentAsync(@"c:/", 30, default));
    }

    [Fact]
    public async Task ThirtySnapshotsAndHundredFilesAreRetained()
    {
        using var fixture = new TempTree();
        var store = new SqliteHistoryStore(new(fixture.Root));
        for (var i = 0; i < 35; i++) Assert.True((await store.SaveAsync(Snapshot(i, files: 130), default)).Saved);
        var retained = await store.LoadRecentAsync(@"C:\", 100, default);
        Assert.Equal(30, retained.Count);
        Assert.All(retained, s => Assert.InRange(s.LargestFiles.Count, 0, 100));
        Assert.Equal(34, retained[0].Directories.Single().LogicalBytes);
        Assert.Equal(130, retained[0].LargestFiles[0].LogicalBytes);
        Assert.Equal(ScanIssueKind.AccessDenied, retained[0].Issues.Single().Kind);
    }

    [Fact]
    public async Task StorageFailurePreservesLiveResult()
    {
        using var fixture = new TempTree();
        var store = new SqliteHistoryStore(new(fixture.Root));
        var successful = Snapshot();
        Assert.True((await store.SaveAsync(successful, default)).Saved);
        var live = Snapshot(15);
        using (var locked = new FileStream(store.DatabasePath, FileMode.Open, FileAccess.ReadWrite, FileShare.None))
        {
            var failedWrite = await store.SaveAsync(live, default);
            Assert.False(failedWrite.Saved);
            Assert.False(string.IsNullOrWhiteSpace(failedWrite.UserMessage));
        }
        Assert.Same(successful, store.LastSuccessfulSnapshot);
        Assert.Same(successful, store.LastCompletedTraversal);
        Assert.Equal(15, live.Directories.Single().LogicalBytes);
        Assert.Equal(successful.Id, (await store.LoadRecentAsync(@"C:\", 30, default)).Single().Id);
    }

    [Fact]
    public async Task CorruptHistoryDoesNotCrashMonitoring()
    {
        using var fixture = new TempTree();
        var store = new SqliteHistoryStore(new(fixture.Root));
        byte[] corrupt = [1, 2, 3, 4, 5];
        await File.WriteAllBytesAsync(store.DatabasePath, corrupt);
        Assert.Empty(await store.LoadRecentAsync(@"C:\", 30, default));
        Assert.Contains("reset", store.LastUserMessage!, StringComparison.OrdinalIgnoreCase);
        Assert.False((await store.SaveAsync(Snapshot(), default)).Saved);
        Assert.Equal(corrupt, await File.ReadAllBytesAsync(store.DatabasePath));
    }

    [Theory]
    [InlineData(99, "load")]
    [InlineData(99, "save")]
    [InlineData(99, "cleanup")]
    [InlineData(0, "load")]
    [InlineData(0, "save")]
    [InlineData(0, "cleanup")]
    public async Task IncompatibleSchemaIsReportedWithoutResetForEveryOperation(int version, string operation)
    {
        using var fixture = new TempTree();
        var store = new SqliteHistoryStore(new(fixture.Root));
        var successful = Snapshot();
        var cleanup = new CleanupReport(Guid.NewGuid(), [new(Guid.NewGuid(), CleanupOutcome.Deleted, null)], 12);
        Assert.True((await store.SaveAsync(successful, default)).Saved);
        await store.AppendCleanupAsync(cleanup, default);
        using (var db = new SqliteConnection($"Data Source={store.DatabasePath};Pooling=False"))
        {
            db.Open();
            using var command = db.CreateCommand();
            command.CommandText = version == 99 ? "PRAGMA user_version=99" : "PRAGMA user_version=0";
            command.ExecuteNonQuery();
        }
        var before = await File.ReadAllBytesAsync(store.DatabasePath);

        switch (operation)
        {
            case "load":
                Assert.Empty(await store.LoadRecentAsync(@"C:\", 30, default));
                break;
            case "save":
                var failed = await store.SaveAsync(Snapshot(15), default);
                Assert.False(failed.Saved);
                Assert.Contains("reset", failed.UserMessage!, StringComparison.OrdinalIgnoreCase);
                break;
            case "cleanup":
                await store.AppendCleanupAsync(new(Guid.NewGuid(), [], 0), default);
                break;
            default: throw new InvalidOperationException("Unexpected test operation.");
        }

        Assert.Contains("reset", store.LastUserMessage!, StringComparison.OrdinalIgnoreCase);
        Assert.Same(successful, store.LastSuccessfulSnapshot);
        Assert.Same(successful, store.LastCompletedTraversal);
        Assert.Equal(before, await File.ReadAllBytesAsync(store.DatabasePath));
        using var reopened = new SqliteConnection($"Data Source={store.DatabasePath};Mode=ReadOnly;Pooling=False");
        reopened.Open();
        using var query = reopened.CreateCommand();
        query.CommandText = "SELECT id FROM snapshots";
        Assert.Equal(successful.Id.ToString(), query.ExecuteScalar());
        query.CommandText = "SELECT id FROM cleanup_reports";
        Assert.Equal(cleanup.PlanId.ToString(), query.ExecuteScalar());
        query.CommandText = "PRAGMA user_version";
        Assert.Equal((long)version, query.ExecuteScalar());
    }

    [Theory]
    [InlineData("null")]
    [InlineData("null-directories")]
    [InlineData("incomplete")]
    [InlineData("oversize-file-list")]
    public async Task InvalidStoredPayloadReturnsDiagnosticAndPreservesCommittedRows(string corruption)
    {
        using var fixture = new TempTree();
        var store = new SqliteHistoryStore(new(fixture.Root));
        var previous = Snapshot();
        var successful = Snapshot(15);
        Assert.True((await store.SaveAsync(previous, default)).Saved);
        Assert.True((await store.SaveAsync(successful, default)).Saved);
        var payload = corruption switch
        {
            "null" => "null"u8.ToArray(),
            "null-directories" => JsonSerializer.SerializeToUtf8Bytes(successful with { Directories = null! }),
            "incomplete" => JsonSerializer.SerializeToUtf8Bytes(successful with { TraversalCompleted = false }),
            "oversize-file-list" => JsonSerializer.SerializeToUtf8Bytes(successful with { LargestFiles = Snapshot(files: 101).LargestFiles }),
            _ => throw new InvalidOperationException("Unexpected test corruption.")
        };
        using (var db = new SqliteConnection($"Data Source={store.DatabasePath};Pooling=False"))
        {
            db.Open();
            using var command = db.CreateCommand();
            command.CommandText = "UPDATE snapshots SET payload=$payload WHERE id=$id";
            command.Parameters.AddWithValue("$payload", payload);
            command.Parameters.AddWithValue("$id", successful.Id.ToString());
            Assert.Equal(1, command.ExecuteNonQuery());
        }
        var before = await File.ReadAllBytesAsync(store.DatabasePath);

        Assert.Empty(await store.LoadRecentAsync(@"C:\", 30, default));
        Assert.Contains("reset", store.LastUserMessage!, StringComparison.OrdinalIgnoreCase);
        Assert.Same(successful, store.LastSuccessfulSnapshot);
        Assert.Same(successful, store.LastCompletedTraversal);
        Assert.Equal(before, await File.ReadAllBytesAsync(store.DatabasePath));
        using var reopened = new SqliteConnection($"Data Source={store.DatabasePath};Mode=ReadOnly;Pooling=False");
        reopened.Open();
        using var query = reopened.CreateCommand();
        query.CommandText = "SELECT COUNT(*) FROM snapshots";
        Assert.Equal(2L, query.ExecuteScalar());
        query.CommandText = "SELECT payload FROM snapshots WHERE id=$id";
        query.Parameters.AddWithValue("$id", previous.Id.ToString());
        Assert.Equal(previous.Id, JsonSerializer.Deserialize<ScanSnapshot>((byte[])query.ExecuteScalar()!)!.Id);
    }

    [Fact]
    public async Task SchemaMigrationPreservesSnapshots()
    {
        using var fixture = new TempTree();
        var store = new SqliteHistoryStore(new(fixture.Root));
        var snapshot = Snapshot();
        using (var db = new SqliteConnection($"Data Source={store.DatabasePath};Pooling=False"))
        {
            db.Open();
            using var command = db.CreateCommand();
            command.CommandText = "PRAGMA auto_vacuum=INCREMENTAL; CREATE TABLE snapshots (id TEXT PRIMARY KEY, root TEXT NOT NULL, completed INTEGER NOT NULL, payload BLOB NOT NULL); PRAGMA user_version=1;";
            command.ExecuteNonQuery();
            command.CommandText = "INSERT INTO snapshots VALUES ($id, $root, $time, $payload)";
            command.Parameters.AddWithValue("$id", snapshot.Id.ToString());
            command.Parameters.AddWithValue("$root", @"C:\");
            command.Parameters.AddWithValue("$time", snapshot.CompletedUtc.UtcTicks);
            command.Parameters.AddWithValue("$payload", JsonSerializer.SerializeToUtf8Bytes(snapshot));
            command.ExecuteNonQuery();
        }
        Assert.Equal(snapshot.Id, (await store.LoadRecentAsync(@"C:\", 30, default)).Single().Id);
        await store.AppendCleanupAsync(new(Guid.NewGuid(), [new(Guid.NewGuid(), CleanupOutcome.Deleted, null)], 123), default);
        using var reopened = new SqliteConnection($"Data Source={store.DatabasePath};Pooling=False");
        reopened.Open();
        using var query = reopened.CreateCommand();
        query.CommandText = "PRAGMA user_version";
        Assert.Equal(2L, query.ExecuteScalar());
        query.CommandText = "SELECT COUNT(*) FROM cleanup_reports";
        Assert.Equal(1L, query.ExecuteScalar());
    }

    [Fact]
    public async Task ConcurrentStoresSerializeAndCancellationPropagates()
    {
        using var fixture = new TempTree();
        var stores = Enumerable.Range(0, 4).Select(_ => new SqliteHistoryStore(new(fixture.Root))).ToArray();
        var writes = await Task.WhenAll(Enumerable.Range(0, 40).Select(i => stores[i % 4].SaveAsync(Snapshot(i), default)));
        Assert.All(writes, w => Assert.True(w.Saved, w.UserMessage));
        Assert.Equal(30, (await stores[0].LoadRecentAsync(@"C:\", 30, default)).Count);
        using var canceled = new CancellationTokenSource();
        canceled.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => stores[0].SaveAsync(Snapshot(), canceled.Token));
    }
}
