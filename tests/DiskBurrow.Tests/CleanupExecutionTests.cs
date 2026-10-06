using DiskBurrow.Core.Cleanup;
using DiskBurrow.Windows.Cleanup;
using DiskBurrow.Windows.Files;
using Microsoft.Win32.SafeHandles;
using System.ComponentModel;

namespace DiskBurrow.Tests;

public sealed class CleanupExecutionTests
{
    [Fact]
    public async Task OnlySelectedCandidatesAreDeleted()
    {
        using var f = new CleanupFixture();
        var selected = Old(f, "selected");
        var kept = Old(f, "unselected");
        var plan = await Preview(f);
        var report = await new WindowsCleanupExecutor(f.Discovery()).ExecuteAsync(plan,
            new HashSet<Guid> { plan.Candidates.Single(c => c.File.Path == selected).Id }, default);
        Assert.Equal(CleanupOutcome.Deleted, Assert.Single(report.Items).Outcome);
        Assert.False(File.Exists(selected));
        Assert.True(File.Exists(kept));
        Assert.True(Directory.Exists(Path.GetDirectoryName(selected)));
    }

    [Theory]
    [InlineData("identity")]
    [InlineData("size")]
    [InlineData("mtime")]
    public async Task ChangedIdentitySizeOrMtimeIsSkipped(string change)
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var plan = await Preview(f);
        if (change == "identity") { File.Move(path, path + ".old"); File.WriteAllBytes(path, new byte[17]); }
        if (change == "size") File.WriteAllBytes(path, new byte[18]);
        File.SetLastWriteTimeUtc(path, change == "mtime" ? DateTime.UtcNow.AddDays(-9) : plan.Candidates[0].File.ModifiedUtc.UtcDateTime);
        Assert.Equal(CleanupOutcome.SkippedChanged, Assert.Single((await Run(f, plan)).Items).Outcome);
        Assert.True(File.Exists(path));
    }

    [Fact]
    public async Task LockedAndReadonlyFilesAreSkipped()
    {
        using var f = new CleanupFixture();
        var busy = Old(f, "busy");
        var readOnly = Old(f, "readonly");
        var plan = await Preview(f);
        File.SetAttributes(readOnly, FileAttributes.ReadOnly);
        try
        {
            using var locked = new FileStream(busy, FileMode.Open, FileAccess.ReadWrite, FileShare.None);
            var report = await Run(f, plan);
            Assert.Equal(CleanupOutcome.SkippedBusy, report.Items.Single(i => i.CandidateId == plan.Candidates.Single(c => c.File.Path == busy).Id).Outcome);
            Assert.Equal(CleanupOutcome.SkippedPolicy, report.Items.Single(i => i.CandidateId == plan.Candidates.Single(c => c.File.Path == readOnly).Id).Outcome);
            Assert.True(File.Exists(busy));
            Assert.True(File.Exists(readOnly));
        }
        finally { File.SetAttributes(readOnly, FileAttributes.Normal); }
    }

    [Theory]
    [InlineData(OwnerProcessState.Running)]
    [InlineData(OwnerProcessState.Unavailable)]
    public async Task BrowserReopenedAfterPreviewIsSkipped(OwnerProcessState state)
    {
        using var f = new CleanupFixture();
        var path = f.Tree.FileAt("profile/AppData/Local/Google/Chrome/User Data/Default/Cache/Cache_Data/data", 17);
        var plan = await Preview(f);
        f.Environment.ProcessStates["chrome"] = state;
        Assert.Equal(CleanupOutcome.SkippedPolicy, Assert.Single((await Run(f, plan)).Items).Outcome);
        Assert.True(File.Exists(path));
    }

    [Fact]
    public async Task UnknownSelectedIdIsRejectedBeforeAnyDeletion()
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var plan = await Preview(f);
        await Assert.ThrowsAsync<ArgumentException>(() => new WindowsCleanupExecutor(f.Discovery()).ExecuteAsync(plan,
            new HashSet<Guid> { plan.Candidates[0].Id, Guid.NewGuid() }, default));
        Assert.True(File.Exists(path));
    }

    [Fact]
    public async Task DuplicateCandidateIdsAreRejectedBeforeAnyDeletion()
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var plan = await Preview(f);
        plan = plan with { Candidates = [plan.Candidates[0], plan.Candidates[0]] };
        await Assert.ThrowsAsync<ArgumentException>(() => Run(f, plan));
        Assert.True(File.Exists(path));
    }

    [Fact]
    public async Task AgeIsRevalidatedAgainstExecutionClock()
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var plan = await Preview(f);
        var report = await new WindowsCleanupExecutor(f.Discovery(), timeProvider: new AtTime(plan.Candidates[0].File.ModifiedUtc.AddDays(7)))
            .ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), default);
        Assert.Equal(CleanupOutcome.SkippedPolicy, Assert.Single(report.Items).Outcome);
        Assert.True(File.Exists(path));
    }

    [Fact]
    public async Task CustomRootApprovalMustStillBeValidAtExecution()
    {
        using var f = new CleanupFixture();
        var custom = f.Tree.DirectoryAt("scratch/Temp");
        var path = f.Tree.FileAt("scratch/Temp/file", 17);
        File.SetLastWriteTimeUtc(path, DateTime.UtcNow.AddDays(-10));
        f.Environment.KnownDirectories = f.Environment.KnownDirectories with { TempHint = custom };
        var approval = Assert.IsType<ApprovedTempRoot>(f.Discovery().ApproveCustomTempRoot(custom));
        var discovery = f.Discovery(approved: [approval]);
        var plan = await new WindowsCleanupPlanner(discovery).PreviewAsync(new HashSet<string>(), default);
        f.Environment.KnownDirectories = f.Environment.KnownDirectories with { TempHint = f.Tree.DirectoryAt("new-temp") };
        var report = await new WindowsCleanupExecutor(discovery).ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), default);
        Assert.Equal(CleanupOutcome.SkippedPolicy, Assert.Single(report.Items).Outcome);
        Assert.True(File.Exists(path));
    }

    [Theory]
    [InlineData("root")]
    [InlineData("directory")]
    [InlineData("outside")]
    [InlineData("rule")]
    [InlineData("age")]
    [InlineData("owner")]
    [InlineData("ads")]
    [InlineData("device")]
    public async Task RootDirectoriesAndFabricatedPermissionsAreNeverDeleted(string kind)
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var outside = f.Tree.FileAt("outside/sentinel", 17);
        var directory = f.Tree.DirectoryAt("profile/AppData/Local/Temp/empty");
        var plan = await Preview(f);
        var c = plan.Candidates[0];
        var native = new NativeFileApi();
        c = kind switch
        {
            "root" => c with { File = native.Inspect(c.Rule.Path) },
            "directory" => c with { File = native.Inspect(directory) },
            "outside" => c with { File = native.Inspect(outside) },
            "rule" => c with { Rule = c.Rule with { RuleId = "MadeUp" } },
            "age" => c with { Rule = c.Rule with { MinimumAge = null } },
            "owner" => c with { Rule = c.Rule with { OwnerProcessName = "madeup" } },
            "ads" => c with { File = c.File with { Path = path + ":extra" } },
            _ => c with { File = c.File with { Path = @"\\?\" + path } }
        };
        plan = plan with { Candidates = [c] };
        Assert.Equal(CleanupOutcome.SkippedPolicy, Assert.Single((await Run(f, plan)).Items).Outcome);
        Assert.True(File.Exists(path));
        Assert.True(File.Exists(outside));
        Assert.True(Directory.Exists(directory));
        Assert.True(Directory.Exists(c.Rule.Path));
    }

    [Fact]
    public async Task CancellationStopsFurtherDeletionsAndReturnsCompletedOutcomes()
    {
        using var f = new CleanupFixture();
        var first = Old(f, "a");
        var second = Old(f, "b");
        var plan = await Preview(f);
        using var cts = new CancellationTokenSource();
        var api = new CancelAfterDeletion(cts);
        var report = await new WindowsCleanupExecutor(f.Discovery(), api).ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), cts.Token);
        Assert.Equal(CleanupOutcome.Deleted, Assert.Single(report.Items).Outcome);
        Assert.True(report.WasCancelled);
        Assert.Equal(first,report.Items[0].Audit?.Path);
        Assert.Equal(plan.Candidates[0].Rule.RuleId,report.Items[0].Audit?.RuleId);
        Assert.False(File.Exists(first));
        Assert.True(File.Exists(second));
    }

    [Fact]
    public async Task AlreadyCancelledReturnsEmptyReport()
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var plan = await Preview(f);
        var report = await new WindowsCleanupExecutor(f.Discovery()).ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), new CancellationToken(true));
        Assert.Empty(report.Items);
        Assert.True(report.WasCancelled);
        Assert.True(File.Exists(path));
    }

    [Fact]
    public async Task DeletionReportSeparatesMissingSkippedFailedAndDeleted()
    {
        using var f = new CleanupFixture();
        var deleted = Old(f, "deleted");
        var missing = Old(f, "missing");
        var changed = Old(f, "changed");
        var failed = Old(f, "failed");
        var plan = await Preview(f);
        File.Delete(missing);
        File.SetLastWriteTimeUtc(changed, DateTime.UtcNow);
        var report = await Run(f, plan, new FailDisposition(failed));
        foreach (var (path, outcome) in new[] { (deleted, CleanupOutcome.Deleted), (missing, CleanupOutcome.Missing), (changed, CleanupOutcome.SkippedChanged), (failed, CleanupOutcome.Failed) })
            Assert.Equal(outcome, report.Items.Single(i => i.CandidateId == plan.Candidates.Single(c => c.File.Path == path).Id).Outcome);
        Assert.False(File.Exists(deleted));
        Assert.True(File.Exists(failed));
        Assert.Equal(plan.Id, report.PlanId);
    }

    [Fact]
    public async Task NewlyRemoteVolumeIsRejected()
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var plan = await Preview(f);
        var report = await new WindowsCleanupExecutor(f.Discovery(driveType: _ => DriveType.Network))
            .ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), default);
        Assert.Equal(CleanupOutcome.SkippedPolicy, Assert.Single(report.Items).Outcome);
        Assert.True(File.Exists(path));
    }

    [Theory]
    [InlineData("empty")]
    [InlineData("rejected")]
    [InlineData("cancelled")]
    public async Task NoVolumeObservationIsReportedAsUnavailable(string reason)
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var plan = await Preview(f);
        var observations = 0;
        var discovery = reason == "rejected" ? f.Discovery(driveType: _ => DriveType.Network) : f.Discovery();
        var selected = reason == "empty" ? new HashSet<Guid>() : plan.Candidates.Select(c => c.Id).ToHashSet();
        var report = await new WindowsCleanupExecutor(discovery, availableBytes: _ => { observations++; return 12345; })
            .ExecuteAsync(plan, selected, new CancellationToken(reason == "cancelled"));
        Assert.Equal(0, observations);
        Assert.False(report.FreeSpaceDeltaAvailable);
        Assert.Equal(0, report.FreeSpaceDeltaBytes);
        Assert.True(File.Exists(path));
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(2)]
    public async Task FreeSpaceFailureIsDifferentFromMeasuredZero(int failObservation)
    {
        using var f = new CleanupFixture();
        var path = Old(f, "file");
        var parent = Path.GetDirectoryName(path)!;
        var plan = await Preview(f);
        var observations = 0;
        var report = await new WindowsCleanupExecutor(f.Discovery(), availableBytes: _ =>
        {
            observations++;
            if (observations == 2)
            {
                Assert.False(File.Exists(path));
                // A live ancestor lease would prevent this rename. Measurement must follow disposal.
                Directory.Move(parent, parent + ".check");
                Directory.Move(parent + ".check", parent);
            }
            if (observations == failObservation) throw new IOException("Injected unavailable volume observation");
            return 12345;
        }).ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), default);
        Assert.Equal(CleanupOutcome.Deleted, Assert.Single(report.Items).Outcome);
        Assert.Equal(0, report.FreeSpaceDeltaBytes);
        Assert.Equal(failObservation == 0, report.FreeSpaceDeltaAvailable);
        Assert.Equal(2, observations);
    }

    internal static string Old(CleanupFixture f, string name)
    {
        var path = f.Tree.FileAt("profile/AppData/Local/Temp/" + name, 17);
        File.SetLastWriteTimeUtc(path, DateTime.UtcNow.AddDays(-10));
        return path;
    }
    internal static Task<CleanupPlan> Preview(CleanupFixture f) => new WindowsCleanupPlanner(f.Discovery()).PreviewAsync(new HashSet<string>(), default);
    internal static Task<CleanupReport> Run(CleanupFixture f, CleanupPlan plan, NativeFileApi? api = null) =>
        new WindowsCleanupExecutor(f.Discovery(), api).ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), default);

    private sealed class CancelAfterDeletion(CancellationTokenSource cts) : NativeFileApi
    {
        public override void MarkForDeletion(SafeFileHandle file) { base.MarkForDeletion(file); cts.Cancel(); }
    }
    private sealed class AtTime(DateTimeOffset now) : TimeProvider { public override DateTimeOffset GetUtcNow() => now; }
    private sealed class FailDisposition(string path) : NativeFileApi
    {
        public override void MarkForDeletion(SafeFileHandle file)
        {
            if (GetFinalPath(file).Equals(path, StringComparison.OrdinalIgnoreCase)) throw new IOException("Injected device error", new Win32Exception(1117));
            base.MarkForDeletion(file);
        }
    }
}
