using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;

namespace DiskBurrow.Tests;

public sealed class MonitoringTests
{
    private const long GiB = 1024L * 1024 * 1024;
    private static readonly DateTimeOffset Start = new(2026, 10, 5, 0, 0, 0, TimeSpan.Zero);

    [Fact]
    public void LowSpaceAlertsOnceUntilRecovery()
    {
        var policy = new AlertPolicy(new(), @"C:\");
        Assert.Empty(policy.Evaluate(null, null, 15 * GiB, Start));
        Assert.Equal(@"C:\", Assert.Single(policy.Evaluate(null, null, 15 * GiB - 1, Start)).DestinationPath);
        Assert.Empty(policy.Evaluate(null, null, GiB, Start.AddDays(1)));
        Assert.Empty(policy.Evaluate(null, null, 15 * GiB, Start.AddDays(1)));
        Assert.Single(policy.Evaluate(null, null, GiB, Start.AddDays(1)));
    }

    [Fact]
    public void FiveGiBGrowthAlertsOnCoveredFolder()
    {
        var before = Snapshot(10 * GiB);
        var after = Snapshot(15 * GiB);
        var alert = Assert.Single(new AlertPolicy(new(), @"C:\").Evaluate(before, after, 20 * GiB, Start));
        Assert.Equal(AlertKind.FolderGrowth, alert.Kind);
        Assert.Equal(@"C:\cache", alert.DestinationPath);
        Assert.Empty(new AlertPolicy(new(), @"C:\").Evaluate(before, after with { TraversalCompleted = false }, 20 * GiB, Start));
        Assert.Empty(new AlertPolicy(new(), @"C:\").Evaluate(before, Snapshot(15 * GiB, false), 20 * GiB, Start));
        Assert.Empty(new AlertPolicy(new(), @"C:\").Evaluate(before, Snapshot(15 * GiB - 1), 20 * GiB, Start));
    }

    [Fact]
    public void GrowthDeduplicatesForTwentyFourHoursAndNeverReplaysSnapshot()
    {
        var policy = new AlertPolicy(new(), @"C:\");
        var before = Snapshot(10 * GiB);
        var after = Snapshot(15 * GiB);
        Assert.Single(policy.Evaluate(before, after, 20 * GiB, Start));
        Assert.Empty(policy.Evaluate(after, Snapshot(20 * GiB), 20 * GiB, Start.AddHours(23)));
        Assert.Empty(policy.Evaluate(before, after, 20 * GiB, Start.AddDays(1)));
        Assert.Single(policy.Evaluate(after, Snapshot(20 * GiB), 20 * GiB, Start.AddDays(1)));
    }

    [Fact]
    public void ClockMovedBackDoesNotSpamAndSuppressionSurvivesRestart()
    {
        var state = new AlertSuppressionState();
        var policy = new AlertPolicy(new(), @"C:\", state);
        Assert.Equal(2, policy.Evaluate(Snapshot(10 * GiB), Snapshot(15 * GiB), GiB, Start).Count);
        var restarted = new AlertPolicy(new(), @"C:\", state);
        Assert.Empty(restarted.Evaluate(Snapshot(15 * GiB), Snapshot(20 * GiB), GiB, Start.AddDays(-2)));
        Assert.Empty(restarted.Evaluate(Snapshot(15 * GiB), Snapshot(20 * GiB), GiB, Start.AddHours(23)));
        Assert.Single(restarted.Evaluate(Snapshot(15 * GiB), Snapshot(20 * GiB), GiB, Start.AddDays(1)));
    }

    [Fact]
    public void GrowthDuringQuietPeriodIsNotReplayedAfterQuietPeriod()
    {
        var state = new AlertSuppressionState();
        var policy = new AlertPolicy(new(), @"C:\", state);
        Assert.Single(policy.Evaluate(Snapshot(10 * GiB), Snapshot(15 * GiB), 20 * GiB, Start));
        var before = Snapshot(15 * GiB);
        var quiet = Snapshot(20 * GiB);
        Assert.Empty(policy.Evaluate(before, quiet, 20 * GiB, Start.AddHours(23)));
        Assert.Empty(new AlertPolicy(new(), @"C:\", state).Evaluate(before, quiet, 20 * GiB, Start.AddDays(1)));
        Assert.Single(policy.Evaluate(quiet, Snapshot(25 * GiB), 20 * GiB, Start.AddDays(1)));
    }

    [Fact]
    public void MissedIntervalsCoalesceAfterResume()
    {
        var policy = new SchedulePolicy(new(), Start);
        Assert.False(policy.IsFullScanDue(Start.AddMinutes(4), false, false));
        Assert.True(policy.IsFullScanDue(Start.AddMinutes(5), false, false));
        policy.RecordFullScan(Start.AddDays(2));
        Assert.False(policy.IsFullScanDue(Start.AddDays(2), false, false));
        Assert.False(policy.IsFullScanDue(Start.AddDays(2).AddHours(5), false, false));
        Assert.True(policy.IsFullScanDue(Start.AddDays(2).AddHours(6), false, false));
    }

    [Fact]
    public void BatteryAndPauseDeferBackgroundAndOperationsCannotOverlap()
    {
        var settings = new AppSettings();
        var policy = new SchedulePolicy(settings, Start);
        Assert.False(policy.IsFullScanDue(Start.AddHours(1), true, false));
        Assert.False(policy.IsFullScanDue(Start.AddHours(1), false, true));
        settings.Paused = true;
        Assert.False(policy.IsFullScanDue(Start.AddHours(1), false, false));
        settings.Paused = false;
        settings.AllowOnBattery = true;
        Assert.True(policy.IsFullScanDue(Start.AddHours(1), true, false));
    }

    internal static ScanSnapshot Snapshot(long bytes, bool covered = true, string root = @"C:\") => new(
        Guid.NewGuid(), root, Start, Start.AddSeconds(1), true,
        [new(Path.Combine(root, "cache"), bytes, bytes, covered)], [], []);
}
