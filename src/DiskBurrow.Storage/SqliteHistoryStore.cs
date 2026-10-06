using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using DiskBurrow.Core.Scanning;
using System.Globalization;
using System.Text.Json;
using Microsoft.Data.Sqlite;

namespace DiskBurrow.Storage;

public sealed class SqliteHistoryStore : IHistoryStore
{
    private readonly StorageBudget budget;
    private const long SchemaReserve = 32 * 1024;
    public string DatabasePath { get; }
    public string? LastUserMessage { get; private set; }
    public ScanSnapshot? LastSuccessfulSnapshot { get; private set; }
    public ScanSnapshot? LastCompletedTraversal { get; private set; }

    public SqliteHistoryStore(StorageBudget budget)
    {
        this.budget = budget;
        DatabasePath = Path.Combine(budget.DirectoryPath, "history.db");
    }

    public async Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot, CancellationToken ct)
    {
        await budget.Gate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            ct.ThrowIfCancellationRequested();
            if (!snapshot.TraversalCompleted) return Failure("Only completed traversals can be saved.");
            var stored = snapshot with { LargestFiles = snapshot.LargestFiles.OrderByDescending(f => f.LogicalBytes).Take(100).ToArray() };
            var payload = JsonSerializer.SerializeToUtf8Bytes(stored);
            CheckCapacity(payload.LongLength);
            using var db = Open(write: true, ct);
            var pageSize = Scalar(db, "PRAGMA page_size");
            var capacity = budget.DatabaseCapacity(DatabasePath, pageSize);
            using (var transaction = db.BeginTransaction())
            {
                Execute(db, "DELETE FROM snapshots WHERE id=$id", transaction, ("$id", snapshot.Id.ToString()));
                Execute(db, "DELETE FROM snapshots WHERE id IN (SELECT id FROM snapshots ORDER BY completed DESC, rowid DESC LIMIT -1 OFFSET 29)", transaction);
                MakeRoom(db, transaction, payload.LongLength, capacity, pageSize);
                ct.ThrowIfCancellationRequested();
                Execute(db, "INSERT INTO snapshots(id,root,completed,payload) VALUES ($id,$root,$time,$payload)", transaction,
                    ("$id", snapshot.Id.ToString()), ("$root", RootKey(snapshot.Root)),
                    ("$time", snapshot.CompletedUtc.UtcTicks), ("$payload", payload));
                ct.ThrowIfCancellationRequested();
                transaction.Commit();
            }
            // A successful commit is authoritative even if optional space reclamation later fails.
            LastSuccessfulSnapshot = snapshot;
            LastCompletedTraversal = snapshot;
            LastUserMessage = null;
            Reclaim(db);
            return new(true, null);
        }
        catch (Exception e) when (IsStorageFailure(e)) { return Failure(Message(e)); }
        finally { budget.Gate.Release(); }
    }

    public async Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root, int limit, CancellationToken ct)
    {
        await budget.Gate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            ct.ThrowIfCancellationRequested();
            if (limit <= 0 || !File.Exists(DatabasePath)) return [];
            using var db = Open(write: false, ct);
            using var query = db.CreateCommand();
            query.CommandText = "SELECT payload FROM snapshots WHERE root=$root ORDER BY completed DESC, rowid DESC LIMIT $limit";
            query.Parameters.AddWithValue("$root", RootKey(root));
            query.Parameters.AddWithValue("$limit", Math.Min(30, limit));
            using var reader = query.ExecuteReader();
            var snapshots = new List<ScanSnapshot>();
            while (reader.Read())
            {
                ct.ThrowIfCancellationRequested();
                var snapshot = JsonSerializer.Deserialize<ScanSnapshot>((byte[])reader[0]);
                if (snapshot is null || !snapshot.TraversalCompleted || snapshot.Directories is null ||
                    snapshot.LargestFiles is null || snapshot.Issues is null || snapshot.LargestFiles.Count > 100)
                    throw new InvalidDataException("Invalid stored snapshot.");
                snapshots.Add(snapshot);
            }
            LastUserMessage = null;
            return snapshots;
        }
        catch (Exception e) when (IsStorageFailure(e)) { LastUserMessage = Message(e); return []; }
        finally { budget.Gate.Release(); }
    }

    public async Task AppendCleanupAsync(CleanupReport report, CancellationToken ct)
    {
        await budget.Gate.WaitAsync(ct).ConfigureAwait(false);
        try
        {
            ct.ThrowIfCancellationRequested();
            var payload = JsonSerializer.SerializeToUtf8Bytes(report);
            CheckCapacity(payload.LongLength);
            using var db = Open(write: true, ct);
            var pageSize = Scalar(db, "PRAGMA page_size");
            using (var transaction = db.BeginTransaction())
            {
                Execute(db, "DELETE FROM cleanup_reports WHERE id=$id", transaction, ("$id", report.PlanId.ToString()));
                Execute(db, "DELETE FROM cleanup_reports WHERE id IN (SELECT id FROM cleanup_reports ORDER BY created DESC, rowid DESC LIMIT -1 OFFSET 99)", transaction);
                MakeRoom(db, transaction, payload.LongLength, budget.DatabaseCapacity(DatabasePath, pageSize), pageSize);
                ct.ThrowIfCancellationRequested();
                Execute(db, "INSERT INTO cleanup_reports(id,created,payload) VALUES ($id,$time,$payload)", transaction,
                    ("$id", report.PlanId.ToString()), ("$time", DateTimeOffset.UtcNow.UtcTicks), ("$payload", payload));
                ct.ThrowIfCancellationRequested();
                transaction.Commit();
            }
            LastUserMessage = null;
            Reclaim(db);
        }
        catch (Exception e) when (IsStorageFailure(e)) { LastUserMessage = Message(e); }
        finally { budget.Gate.Release(); }
    }

    private void CheckCapacity(long payloadBytes)
    {
        var capacity = budget.DatabaseCapacity(DatabasePath, 4096);
        if (payloadBytes > capacity - SchemaReserve || (File.Exists(DatabasePath) && new FileInfo(DatabasePath).Length > capacity))
            throw new StorageLimitException();
    }

    private SqliteConnection Open(bool write, CancellationToken ct)
    {
        ct.ThrowIfCancellationRequested();
        if (write) CheckCapacity(0);
        if (write) Directory.CreateDirectory(budget.DirectoryPath);
        var db = new SqliteConnection(new SqliteConnectionStringBuilder
        {
            DataSource = DatabasePath, Mode = write ? SqliteOpenMode.ReadWriteCreate : SqliteOpenMode.ReadOnly,
            Pooling = false, DefaultTimeout = 1
        }.ToString());
        try
        {
            db.Open();
            Execute(db, "PRAGMA temp_store=MEMORY; PRAGMA trusted_schema=OFF;");
            var version = Scalar(db, "PRAGMA user_version");
            if (version is < 0 or > 2) throw new InvalidDataException("Unsupported history schema.");
            if (version == 0 && Scalar(db, "SELECT count(*) FROM sqlite_master WHERE type='table'") != 0)
                throw new InvalidDataException("Unrecognized history schema.");
            if (!write && version < 2)
            {
                db.Dispose();
                return Open(write: true, ct);
            }
            if (write || version < 2)
            {
                CheckCapacity(0);
                var pageSize = Scalar(db, "PRAGMA page_size");
                var maxPages = budget.DatabaseCapacity(DatabasePath, pageSize) / pageSize;
                if (maxPages < Scalar(db, "PRAGMA page_count") || maxPages < 8) throw new StorageLimitException();
                // Disable cache spills so the rollback journal has one header sector instead of
                // accumulating an unbounded number of sector-padded headers while a large blob is written.
                Execute(db, "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA cache_spill=OFF;");
                Execute(db, "PRAGMA max_page_count=" + maxPages.ToString(CultureInfo.InvariantCulture));
                Execute(db, "PRAGMA journal_size_limit=0;");
                if (version < 2)
                {
                    using var transaction = db.BeginTransaction();
                    if (version == 0)
                        Execute(db, "CREATE TABLE snapshots(id TEXT PRIMARY KEY,root TEXT NOT NULL,completed INTEGER NOT NULL,payload BLOB NOT NULL)", transaction);
                    Execute(db, "CREATE TABLE cleanup_reports(id TEXT PRIMARY KEY,created INTEGER NOT NULL,payload BLOB NOT NULL); PRAGMA user_version=2;", transaction);
                    ct.ThrowIfCancellationRequested();
                    transaction.Commit();
                }
            }
            return db;
        }
        catch { db.Dispose(); throw; }
    }

    private static void MakeRoom(SqliteConnection db, SqliteTransaction transaction, long incoming, long capacity, long pageSize)
    {
        // Conservative per-record page rounding also covers B-tree cells and indexes. SQLite's hard
        // max_page_count remains the final backstop if fragmentation defeats this preflight estimate.
        var incomingPages = (incoming + pageSize - 1) / pageSize * pageSize + pageSize;
        while (Scalar(db, "SELECT COALESCE(SUM(((length(payload)+$page-1)/$page+1)*$page),0) FROM (SELECT payload FROM snapshots UNION ALL SELECT payload FROM cleanup_reports)", transaction, ("$page", pageSize))
            + incomingPages > capacity - SchemaReserve)
        {
            // Cleanup audit rows are secondary to completed scan history when space is scarce.
            if (Scalar(db, "SELECT count(*) FROM cleanup_reports", transaction) > 0)
                Execute(db, "DELETE FROM cleanup_reports WHERE id=(SELECT id FROM cleanup_reports ORDER BY created, rowid LIMIT 1)", transaction);
            else if (Scalar(db, "SELECT count(*) FROM snapshots", transaction) > 0)
                Execute(db, "DELETE FROM snapshots WHERE id=(SELECT id FROM snapshots ORDER BY completed, rowid LIMIT 1)", transaction);
            else throw new StorageLimitException();
        }
    }

    private static void Reclaim(SqliteConnection db)
    {
        try { Execute(db, "PRAGMA incremental_vacuum(64)"); }
        catch (SqliteException) { /* Commit already succeeded. Reuse the free pages on the next write. */ }
    }

    private static long Scalar(SqliteConnection db, string sql, SqliteTransaction? transaction = null,
        params (string Name, object Value)[] values)
    {
        using var command = Command(db, sql, transaction, values);
        return Convert.ToInt64(command.ExecuteScalar(), CultureInfo.InvariantCulture);
    }

    private static void Execute(SqliteConnection db, string sql, SqliteTransaction? transaction = null,
        params (string Name, object Value)[] values)
    {
        using var command = Command(db, sql, transaction, values);
        command.ExecuteNonQuery();
    }

    private static SqliteCommand Command(SqliteConnection db, string sql, SqliteTransaction? transaction,
        (string Name, object Value)[] values)
    {
        var command = db.CreateCommand();
        command.CommandText = sql;
        command.Transaction = transaction;
        foreach (var (name, value) in values) command.Parameters.AddWithValue(name, value);
        return command;
    }

    private static string RootKey(string root) => SnapshotComparer.NormalizePath(root).ToUpperInvariant();
    private HistoryWriteResult Failure(string message) { LastUserMessage = message; return new(false, message); }
    private static bool IsStorageFailure(Exception e) => e is SqliteException or IOException or InvalidDataException or UnauthorizedAccessException or JsonException;
    private static string Message(Exception e) => e switch
    {
        StorageLimitException => "Local history storage budget is full. The current scan remains available.",
        InvalidDataException or JsonException => "Local history is corrupt or incompatible. Reset history explicitly to resume saving.",
        SqliteException { SqliteErrorCode: 11 or 26 } => "Local history is corrupt. Reset history explicitly to resume saving.",
        SqliteException { SqliteErrorCode: 13 } => "There is insufficient space for local history. The current scan remains available.",
        _ => "Local history storage is unavailable. The current scan remains available."
    };

    private sealed class StorageLimitException : IOException;
}
