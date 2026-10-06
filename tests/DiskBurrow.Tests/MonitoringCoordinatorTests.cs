using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Storage;

namespace DiskBurrow.Tests;

public sealed class MonitoringCoordinatorTests
{
    [Fact]
    public async Task ExplicitAlternativeEngineIsUsedAndNeverSilentlyJoinedByNormalScan()
    {
        using var tree=new TempTree();var normal=new HeldScanner();var fast=new HeldScanner();
        await using var coordinator=Create(tree,normal);
        var pending=coordinator.RequestScanAsync(@"C:\",fast,default);
        await fast.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await Assert.ThrowsAsync<InvalidOperationException>(()=>coordinator.RequestScanAsync(@"C:\",default));
        var shared=coordinator.RequestScanAsync(@"C:\",fast,default);fast.Release.SetResult();
        Assert.Equal((await pending).Id,(await shared).Id);Assert.Equal(1,fast.InvocationCount);Assert.Equal(0,normal.InvocationCount);
        var regular=coordinator.RequestScanAsync(@"C:\",default);await normal.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));normal.Release.SetResult();await regular;
        Assert.Equal(1,normal.InvocationCount);
    }
    [Fact]
    public async Task AlternativeEngineRetainsExclusiveGateAndStopCancellation()
    {
        using var tree=new TempTree();var normal=new HeldScanner();var fast=new HeldScanner();await using var coordinator=Create(tree,normal);
        var entered=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);var release=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var exclusive=coordinator.RunExclusiveAsync(async ct=>{entered.SetResult();await release.Task.WaitAsync(ct);return 1;},default);await entered.Task;
        var pending=coordinator.RequestScanAsync(@"C:\",fast,default);await Task.Delay(25);Assert.False(fast.Started.Task.IsCompleted);
        release.SetResult();await exclusive;await fast.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await coordinator.StopAsync();await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>pending);Assert.Equal(0,normal.InvocationCount);
    }
    [Fact]
    public async Task ConcurrentRequestsShareOneScan()
    {
        using var tree = new TempTree();
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner);
        var first = coordinator.RequestScanAsync(default);
        var second = coordinator.RequestScanAsync(@"c:/", default);
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        scanner.Release.SetResult();
        Assert.Equal((await first).Id, (await second).Id);
        Assert.Equal(1, scanner.InvocationCount);
        Assert.Same(await first, coordinator.LiveSnapshot);
    }

    [Fact]
    public async Task DifferentRootManualRequestWaitsAndReturnsItsOwnSnapshot()
    {
        using var tree = new TempTree();
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner);
        var first = coordinator.RequestScanAsync(default);
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var second = coordinator.RequestScanAsync(@"D:\", default);
        Assert.False(second.IsCompleted);
        scanner.Release.SetResult();
        Assert.Equal(@"C:\", (await first).Root);
        Assert.Equal(@"D:\", (await second).Root);
        Assert.Equal(2, scanner.InvocationCount);
        Assert.Equal(1, scanner.MaxConcurrent);
    }

    [Fact]
    public async Task RemoteOrUnknownRootsFailBeforeScanner()
    {
        using var tree = new TempTree();
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner);
        await Assert.ThrowsAsync<ArgumentException>(() => coordinator.RequestScanAsync(@"\\server\share", default));
        await Assert.ThrowsAsync<ArgumentException>(() => coordinator.RequestScanAsync(@"Z:\", default));
        Assert.Equal(0, scanner.InvocationCount);
    }

    [Fact]
    public async Task BatteryDefersBackgroundButAllowsManual()
    {
        using var tree = new TempTree();
        var clock = new ManualClock();
        var environment = new FakeEnvironment { IsOnBattery = true };
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner, environment: environment, clock: clock);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromHours(1));
        await environment.Checked.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Equal(0, scanner.InvocationCount);
        var manual = coordinator.RequestScanAsync(default);
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        scanner.Release.SetResult();
        Assert.True((await manual).TraversalCompleted);
        await coordinator.StopAsync();
        await run;
    }

    [Fact]
    public async Task PausedMonitoringDoesNotStartFullScanButLowSpaceContinues()
    {
        using var tree = new TempTree();
        var clock = new ManualClock();
        var environment = new FakeEnvironment { FreeBytes = 1 };
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner, environment: environment, clock: clock);
        coordinator.Pause(true);
        var alert = new TaskCompletionSource<AlertEvent>(TaskCreationOptions.RunContinuationsAsynchronously);
        coordinator.AlertsRaised += alerts => alert.TrySetResult(alerts[0]);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromMinutes(5));
        Assert.Equal(AlertKind.LowSpace, (await alert.Task.WaitAsync(TimeSpan.FromSeconds(5))).Kind);
        Assert.Equal(0, scanner.InvocationCount);
        await coordinator.StopAsync();
        await run;
    }

    [Fact]
    public async Task FreeSpaceCheckContinuesDuringHeldScan()
    {
        using var tree = new TempTree();
        var clock = new ManualClock();
        var environment = new FakeEnvironment();
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner, environment: environment, clock: clock);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromMinutes(5));
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await clock.WaitForTimerAsync();
        environment.Checked = new(TaskCreationOptions.RunContinuationsAsynchronously);
        clock.Advance(TimeSpan.FromMinutes(5));
        await environment.Checked.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.False(scanner.Release.Task.IsCompleted);
        Assert.Equal(2, environment.CheckCount);
        await coordinator.StopAsync();
        await run;
    }

    [Fact]
    public async Task ExclusiveCleanupBlocksScanButNotFreeSpaceCheckAndExitCancelsIt()
    {
        using var tree = new TempTree();
        var clock = new ManualClock();
        var environment = new FakeEnvironment();
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner, environment: environment, clock: clock);
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var operation = coordinator.RunExclusiveAsync(async ct =>
        {
            entered.SetResult();
            await Task.Delay(Timeout.InfiniteTimeSpan, ct);
            return 1;
        }, default);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        var queued = coordinator.RequestScanAsync(default);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromMinutes(5));
        await environment.Checked.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.Equal(0, scanner.InvocationCount);
        Assert.False(queued.IsCompleted);
        await coordinator.StopAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => operation);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => queued);
        await run;
    }

    [Fact]
    public async Task ExitCancelsAndDoesNotSavePartialScan()
    {
        using var tree = new TempTree();
        var history = new FakeHistory();
        var scanner = new HeldScanner { ReturnPartialOnCancel = true };
        await using var coordinator = Create(tree, scanner, history);
        var scan = coordinator.RequestScanAsync(default);
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await coordinator.StopAsync();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => scan);
        Assert.Empty(history.Snapshots);
        Assert.Null(coordinator.LiveSnapshot);
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public async Task StorageFailurePreservesLiveResultAndDoesNotDisableFreeSpaceCheck(bool throws)
    {
        using var tree = new TempTree();
        var history = new FakeHistory { FailWrites = true, ThrowWrites = throws };
        var clock = new ManualClock();
        var environment = new FakeEnvironment();
        var scanner = new HeldScanner();
        scanner.Release.SetResult();
        await using var coordinator = Create(tree, scanner, history, environment, clock);
        var snapshot = await coordinator.RequestScanAsync(default);
        Assert.Same(snapshot, coordinator.LiveSnapshot);
        Assert.Contains("disk full", coordinator.LastUserMessage);
        environment.Checked = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromMinutes(5));
        await environment.Checked.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await coordinator.StopAsync();
        await run;
    }

    [Fact]
    public async Task LoadedPreviousSnapshotAndPersistentSuppressionPreventRestartSpam()
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        var history = new FakeHistory();
        history.Snapshots.Add(MonitoringTests.Snapshot(10L * 1024 * 1024 * 1024));
        var scanner = new HeldScanner { Bytes = 15L * 1024 * 1024 * 1024 };
        scanner.Release.SetResult();
        var first = new List<AlertEvent>();
        await using (var coordinator = Create(tree, scanner, history))
        {
            coordinator.AlertsRaised += alerts => first.AddRange(alerts);
            await coordinator.RequestScanAsync(default);
        }
        Assert.Single(first);
        var state = await store.LoadAlertStateAsync(default);
        Assert.Contains(@"C:\cache", state.LastGrowthNotifiedUtc.Keys);
        scanner.Bytes = 20L * 1024 * 1024 * 1024;
        var second = new List<AlertEvent>();
        await using (var coordinator = Create(tree, scanner, history))
        {
            coordinator.AlertsRaised += alerts => second.AddRange(alerts);
            await coordinator.RequestScanAsync(default);
        }
        Assert.Empty(second);
    }

    [Fact]
    public async Task ApplySettingsValidatesAndCapturesChoicesWithoutCallerMutation()
    {
        using var tree = new TempTree();
        var clock = new ManualClock();
        var scanner = new HeldScanner();
        var environment = new FakeEnvironment { IsOnBattery = true, FreeBytes = 20L << 30 };
        await using var coordinator = Create(tree, scanner, environment: environment, clock: clock);
        Assert.Throws<ArgumentException>(() => coordinator.ApplySettings(new() { IntervalHours = 2 }));
        var settings = new AppSettings { AllowOnBattery = true, LowSpaceBytes = 25L << 30 };
        coordinator.ApplySettings(settings);
        settings.AllowOnBattery = false;
        settings.LowSpaceBytes = 1;
        var alert = new TaskCompletionSource<AlertEvent>(TaskCreationOptions.RunContinuationsAsynchronously);
        coordinator.AlertsRaised += alerts => alert.TrySetResult(alerts[0]);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromMinutes(5));
        Assert.Equal(AlertKind.LowSpace, (await alert.Task.WaitAsync(TimeSpan.FromSeconds(5))).Kind);
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        await coordinator.StopAsync();
        await run;
    }

    [Fact]
    public async Task HistoryReadFailureIsVisibleButScanAndLiveResultContinue()
    {
        using var tree = new TempTree();
        var scanner = new HeldScanner();
        scanner.Release.SetResult();
        await using var coordinator = Create(tree, scanner, new FakeHistory { ThrowReads = true });
        var snapshot = await coordinator.RequestScanAsync(default);
        Assert.Same(snapshot, coordinator.LiveSnapshot);
        Assert.Contains("history unreadable", coordinator.LastUserMessage);
    }

    [Fact]
    public async Task CancelScanKeepsPreviousLiveResultAndReleasesOperationGate()
    {
        using var tree = new TempTree();
        var scanner = new HeldScanner();
        var history = new FakeHistory();
        await using var coordinator = Create(tree, scanner, history);
        var scan = coordinator.RequestScanAsync(default);
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        coordinator.CancelScan();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => scan);
        Assert.Empty(history.Snapshots);
        Assert.False(coordinator.OperationRunning);
        Assert.Equal(7, await coordinator.RunExclusiveAsync(_ => Task.FromResult(7), default));
    }

    [Fact]
    public async Task CancellingOneWaiterDoesNotCancelAnotherSharedRequest()
    {
        using var tree = new TempTree();
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner);
        using var cancelled = new CancellationTokenSource();
        var first = coordinator.RequestScanAsync(cancelled.Token);
        var second = coordinator.RequestScanAsync(default);
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        cancelled.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => first);
        scanner.Release.SetResult();
        Assert.True((await second).TraversalCompleted);
        Assert.Equal(1, scanner.InvocationCount);
    }

    [Fact]
    public async Task StorageFailureDoesNotPreventComparisonWithPreviousLiveSnapshot()
    {
        using var tree = new TempTree();
        var scanner = new HeldScanner { Bytes = 10L << 30 };
        scanner.Release.SetResult();
        await using var coordinator = Create(tree, scanner, new FakeHistory { FailWrites = true });
        await coordinator.RequestScanAsync(default);
        var alerts = new List<AlertEvent>();
        coordinator.AlertsRaised += values => alerts.AddRange(values);
        scanner.Bytes = 15L << 30;
        await coordinator.RequestScanAsync(default);
        Assert.Equal(AlertKind.FolderGrowth, Assert.Single(alerts).Kind);
    }

    [Fact]
    public async Task TwoExclusiveActionsSerializeAndWaitingActionCanCancel()
    {
        using var tree = new TempTree();
        await using var coordinator = Create(tree, new HeldScanner());
        var entered = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var first = coordinator.RunExclusiveAsync(async ct => { entered.SetResult(); await release.Task.WaitAsync(ct); return 1; }, default);
        await entered.Task.WaitAsync(TimeSpan.FromSeconds(5));
        using var cancellation = new CancellationTokenSource();
        var second = coordinator.RunExclusiveAsync(_ => Task.FromResult(2), cancellation.Token);
        Assert.False(second.IsCompleted);
        cancellation.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => second);
        release.SetResult();
        Assert.Equal(1, await first);
        Assert.False(coordinator.OperationRunning);
    }

    [Fact]
    public async Task StatePersistenceFailureIsVisibleAndFreeSpaceChecksContinueWithoutRepeatAlert()
    {
        using var tree = new TempTree();
        var clock = new ManualClock();
        var environment = new FakeEnvironment { FreeBytes = 1 };
        var scanner = new HeldScanner();
        await using var coordinator = new MonitoringCoordinator(new() { Paused = true }, scanner,
            new FakeHistory(), environment, new FailedStateStore(), clock);
        var alerts = new List<AlertEvent>();
        coordinator.AlertsRaised += values => alerts.AddRange(values);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromMinutes(5));
        await clock.WaitForTimerAsync();
        Assert.Single(alerts);
        Assert.Contains("state disk full", coordinator.LastUserMessage);
        clock.Advance(TimeSpan.FromMinutes(5));
        await clock.WaitForTimerAsync();
        Assert.Equal(2, environment.CheckCount);
        Assert.Single(alerts);
        await coordinator.StopAsync();
        await run;
    }

    [Fact]
    public async Task ResumeFromBatteryCoalescesMissedIntervalsIntoOneScan()
    {
        using var tree = new TempTree();
        var clock = new ManualClock();
        var environment = new FakeEnvironment { IsOnBattery = true };
        var scanner = new HeldScanner();
        await using var coordinator = Create(tree, scanner, environment: environment, clock: clock);
        var run = coordinator.RunAsync(default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromDays(2));
        await clock.WaitForTimerAsync();
        Assert.Equal(0, scanner.InvocationCount);
        environment.IsOnBattery = false;
        clock.Advance(TimeSpan.FromMinutes(5));
        await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        scanner.Release.SetResult();
        await coordinator.RunExclusiveAsync(_ => Task.FromResult(0), default);
        await clock.WaitForTimerAsync();
        clock.Advance(TimeSpan.FromMinutes(5));
        await clock.WaitForTimerAsync();
        Assert.Equal(1, scanner.InvocationCount);
        await coordinator.StopAsync();
        await run;
    }

    private sealed class FailedStateStore : IMonitoringStateStore
    {
        public string? LastUserMessage => null;
        public Task<AlertSuppressionState> LoadAlertStateAsync(CancellationToken ct) => throw new IOException("state unreadable");
        public Task SaveAlertStateAsync(AlertSuppressionState state, CancellationToken ct) => throw new IOException("state disk full");
    }

    private static MonitoringCoordinator Create(TempTree tree, HeldScanner scanner, FakeHistory? history = null,
        FakeEnvironment? environment = null, TimeProvider? clock = null) =>
        new(new AppSettings(), scanner, history ?? new FakeHistory(), environment ?? new FakeEnvironment(),
            new SettingsStore(tree.Root), clock ?? TimeProvider.System);

    private sealed class HeldScanner : IDiskScanner
    {
        public TaskCompletionSource Started { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public TaskCompletionSource Release { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public int InvocationCount { get; private set; }
        public int MaxConcurrent { get; private set; }
        private int _concurrent;
        public bool ReturnPartialOnCancel { get; init; }
        public long Bytes { get; set; } = 1;
        public async Task<ScanSnapshot> ScanAsync(string root, IProgress<ScanProgress>? progress, CancellationToken ct)
        {
            InvocationCount++;
            MaxConcurrent = Math.Max(MaxConcurrent, ++_concurrent);
            Started.TrySetResult();
            try
            {
                await Release.Task.WaitAsync(ct);
                return MonitoringTests.Snapshot(Bytes, root: root);
            }
            catch (OperationCanceledException) when (ReturnPartialOnCancel)
            {
                return MonitoringTests.Snapshot(Bytes, root: root) with { TraversalCompleted = false };
            }
            finally { _concurrent--; }
        }
    }

    private sealed class FakeHistory : IHistoryStore
    {
        public List<ScanSnapshot> Snapshots { get; } = [];
        public bool FailWrites { get; init; }
        public bool ThrowWrites { get; init; }
        public bool ThrowReads { get; init; }
        public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot, CancellationToken ct)
        {
            if (ThrowWrites) throw new IOException("disk full");
            if (FailWrites) return Task.FromResult(new HistoryWriteResult(false, "disk full"));
            Snapshots.Add(snapshot);
            return Task.FromResult(new HistoryWriteResult(true, null));
        }
        public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root, int limit, CancellationToken ct)
        {
            if (ThrowReads) throw new IOException("history unreadable");
            return Task.FromResult<IReadOnlyList<ScanSnapshot>>(Snapshots.Where(s => s.Root == root).Reverse().Take(limit).ToArray());
        }
        public Task AppendCleanupAsync(CleanupReport report, CancellationToken ct) => Task.CompletedTask;
    }

    private sealed class FakeEnvironment : IMonitoringEnvironment
    {
        public string SystemRoot => @"C:\";
        public bool IsOnBattery { get; set; }
        public long FreeBytes { get; set; } = 20L * 1024 * 1024 * 1024;
        public int CheckCount { get; private set; }
        public TaskCompletionSource Checked { get; set; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public long GetFreeBytes(string root) { CheckCount++; Checked.TrySetResult(); return FreeBytes; }
        public VolumeSpace GetVolumeSpace(string root) => new(30L * 1024 * 1024 * 1024, GetFreeBytes(root));
        public bool IsLocalRoot(string root) => root.StartsWith("C:", StringComparison.OrdinalIgnoreCase) || root.StartsWith("D:", StringComparison.OrdinalIgnoreCase);
    }

    internal sealed class ManualClock : TimeProvider
    {
        private readonly object _sync = new();
        private DateTimeOffset _now = new(2026, 10, 5, 0, 0, 0, TimeSpan.Zero);
        private long _ticks;
        private readonly List<ManualTimer> _timers = [];
        public override DateTimeOffset GetUtcNow() { lock (_sync) return _now; }
        public override long GetTimestamp() { lock (_sync) return _ticks; }
        public override long TimestampFrequency => TimeSpan.TicksPerSecond;
        public Task WaitForTimerAsync() => WaitUntilAsync(() => { lock (_sync) return _timers.Any(t => !t.Disposed); });
        private static async Task WaitUntilAsync(Func<bool> ready)
        {
            using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(5));
            while (!ready()) await Task.Delay(1, deadline.Token);
        }
        public void Advance(TimeSpan amount)
        {
            ManualTimer[] due;
            lock (_sync)
            {
                _now += amount;
                _ticks += amount.Ticks;
                due = _timers.Where(t => !t.Disposed && t.Due <= _ticks).ToArray();
                foreach (var timer in due) timer.Disposed = true;
            }
            foreach (var timer in due) timer.Callback(timer.State);
        }
        public override ITimer CreateTimer(TimerCallback callback, object? state, TimeSpan dueTime, TimeSpan period)
        {
            lock (_sync)
            {
                var timer = new ManualTimer(this, callback, state, _ticks + dueTime.Ticks);
                _timers.Add(timer);
                return timer;
            }
        }
        private sealed class ManualTimer(ManualClock clock, TimerCallback callback, object? state, long due) : ITimer
        {
            public TimerCallback Callback { get; } = callback;
            public object? State { get; } = state;
            public long Due { get; private set; } = due;
            public bool Disposed { get; set; }
            public bool Change(TimeSpan dueTime, TimeSpan period) { lock (clock._sync) { Due = clock._ticks + dueTime.Ticks; return !Disposed; } }
            public void Dispose() { lock (clock._sync) Disposed = true; }
            public ValueTask DisposeAsync() { Dispose(); return ValueTask.CompletedTask; }
        }
    }
}
