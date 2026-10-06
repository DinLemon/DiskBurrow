using DiskBurrow.App.Services;
using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;
namespace DiskBurrow.Tests;
public class PresentationCoordinationTests
{
    [Fact] public async Task ChangingRootInvalidatesSnapshotStillWaitingForVolume()
    {
        using var tree=new TempTree();var history=new MemoryHistory();var environment=new HeldVolumeEnvironment();
        await using var coordinator=new MonitoringCoordinator(new(),new HeldScanner(),history,environment,new SettingsStore(tree.Root));
        var model=Model(coordinator,history,environment,tree.Root);
        var snapshot=new ScanSnapshot(Guid.NewGuid(),environment.SystemRoot,DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[],[],[]);
        var display=model.ShowSnapshotAsync(snapshot);
        try{await environment.Started.Task.WaitAsync(TimeSpan.FromSeconds(2));model.Root=@"C:\fixture\another";}
        finally{environment.Release.TrySetResult();await display.WaitAsync(TimeSpan.FromSeconds(2));}
        Assert.Null(model.Overview.Snapshot);Assert.Null(model.History.ComparedUtc);
    }
    [Fact] public async Task ChangingRootInvalidatesAlreadyComputedHistoryComparison()
    {
        using var tree=new TempTree();var store=new MemoryHistory();var environment=new LocalEnvironment();
        await using var coordinator=new MonitoringCoordinator(new(),new HeldScanner(),store,environment,new SettingsStore(tree.Root));
        var ui=new QueuedDispatcher();var history=new HistoryViewModel(store,ui);var model=Model(coordinator,store,environment,tree.Root,historyModel:history);
        var snapshot=new ScanSnapshot(Guid.NewGuid(),environment.SystemRoot,DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[],[],[]);
        var load=history.LoadAsync(snapshot,default);(await ui.Queue.Reader.ReadAsync()).Apply();await load;
        var comparison=history.CompareSelectedAsync(snapshot with {CompletedUtc=snapshot.CompletedUtc.AddDays(1)});
        var stale=await ui.Queue.Reader.ReadAsync();model.Root=@"C:\fixture\another";stale.Apply();await comparison;
        Assert.Equal(snapshot.CompletedUtc,history.ComparedUtc);
    }
    [Fact] public async Task ScanCancelRequestedBeforeCoordinatorStartsIsHonored()
    {
        using var tree=new TempTree();var scanner=new HeldScanner();var history=new MemoryHistory();var environment=new LocalEnvironment();var state=new HeldState();
        await using var coordinator=new MonitoringCoordinator(new(),scanner,history,environment,state);
        var model=Model(coordinator,history,environment,tree.Root);
        var scan=model.ScanAsync();await state.Started.Task.WaitAsync(TimeSpan.FromSeconds(2));Assert.False(coordinator.ScanRunning);
        model.Cancel();state.Release.TrySetResult();await scan.WaitAsync(TimeSpan.FromSeconds(2));Assert.False(model.Busy);Assert.Null(coordinator.LiveSnapshot);
    }
    [Fact] public async Task BusyUiDoesNotQueueAnotherRootAndCancelStopsUnderlyingScan()
    {
        using var tree=new TempTree();var scanner=new HeldScanner();var history=new MemoryHistory();var environment=new LocalEnvironment();
        await using var coordinator=new MonitoringCoordinator(new(),scanner,history,environment,new SettingsStore(tree.Root));
        var model=Model(coordinator,history,environment,tree.Root);
        var scan=model.ScanAsync();await scanner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.True(model.Busy);await model.ScanAsync(@"C:\other-root");Assert.Equal(1,scanner.Invocations);
        model.Cancel();await scan.WaitAsync(TimeSpan.FromSeconds(5));Assert.True(scanner.Cancelled);Assert.False(model.Busy);
    }
    [Fact] public async Task CleanupPreviewCancelStopsItsOperationToken()
    {
        using var tree=new TempTree();var scanner=new HeldScanner();var history=new MemoryHistory();var environment=new LocalEnvironment();
        await using var coordinator=new MonitoringCoordinator(new(),scanner,history,environment,new SettingsStore(tree.Root));
        var planner=new HeldPlanner();var model=Model(coordinator,history,environment,tree.Root,planner);
        var preview=model.AnalyzeAsync();await planner.Started.Task.WaitAsync(TimeSpan.FromSeconds(5));model.Cancel();await preview.WaitAsync(TimeSpan.FromSeconds(5));Assert.True(planner.Cancelled);Assert.False(model.Busy);
    }
    [Fact] public async Task CancelKeepsPreviousOverviewAndDiagnosticLocalizesLive()
    {
        using var tree=new TempTree();var scanner=new HeldScanner();var history=new MemoryHistory();var environment=new LocalEnvironment();
        await using var coordinator=new MonitoringCoordinator(new(),scanner,history,environment,new SettingsStore(tree.Root));
        var model=Model(coordinator,history,environment,tree.Root);
        var snapshot=new ScanSnapshot(Guid.NewGuid(),environment.SystemRoot,DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[],[],[]);
        await model.ShowSnapshotAsync(snapshot);model.ShowStartup("Settings.Recovered","raw fixture detail");
        var ru=model.StartupMessage;model.Language="en";Assert.NotEqual(ru,model.StartupMessage);Assert.Equal("raw fixture detail",model.StartupDetails);
        var scan=model.ScanAsync();await scanner.Started.Task;model.Cancel();await scan.WaitAsync(TimeSpan.FromSeconds(5));Assert.Same(snapshot,model.Overview.Snapshot);
    }
    private static MainViewModel Model(MonitoringCoordinator coordinator,IHistoryStore history,IMonitoringEnvironment env,string dir,ICleanupPlanner? planner=null,HistoryViewModel? historyModel=null)
    {
        var ui=new InlineDispatcher();var settings=new AppSettings();var locale=new LocalizationService();
        return new(coordinator,env,ui,locale,new(ui),historyModel??new(history,ui),new(planner??new HeldPlanner(),new NoExecutor(),history,ui),new(settings,new SettingsStore(dir),new AutostartRegistration(new NoRegistry(),@"C:\fixture\DiskBurrow.exe"),p=>null,s=>{}),settings);
    }
    private sealed class QueuedDispatcher : IUiDispatcher {public System.Threading.Channels.Channel<Invocation> Queue=System.Threading.Channels.Channel.CreateUnbounded<Invocation>();public Task InvokeAsync(Action action){var invocation=new Invocation(action);Queue.Writer.TryWrite(invocation);return invocation.Done.Task;}}
    private sealed class Invocation(Action action) {public TaskCompletionSource Done=new(TaskCreationOptions.RunContinuationsAsynchronously);public void Apply(){action();Done.SetResult();}}
    private sealed class HeldScanner : IDiskScanner
    {
        public int Invocations;public bool Cancelled;public TaskCompletionSource Started=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public async Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? progress,CancellationToken ct){Invocations++;Started.TrySetResult();try{await Task.Delay(Timeout.Infinite,ct);}catch(OperationCanceledException){Cancelled=true;throw;}throw new InvalidOperationException();}
    }
    private sealed class HeldPlanner : ICleanupPlanner
    {
        public bool Cancelled;public TaskCompletionSource Started=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public async Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excluded,CancellationToken ct){Started.TrySetResult();try{await Task.Delay(Timeout.Infinite,ct);}catch(OperationCanceledException){Cancelled=true;throw;}throw new InvalidOperationException();}
    }
    private sealed class NoExecutor : ICleanupExecutor {public Task<CleanupReport> ExecuteAsync(CleanupPlan plan,IReadOnlySet<Guid> ids,CancellationToken ct)=>throw new InvalidOperationException("Never delete in coordination tests.");}
    private sealed class MemoryHistory : IHistoryStore {public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>Task.FromResult(new HistoryWriteResult(true,null));public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>Task.FromResult<IReadOnlyList<ScanSnapshot>>([]);public Task AppendCleanupAsync(CleanupReport report,CancellationToken ct)=>Task.CompletedTask;}
    private sealed class LocalEnvironment : IMonitoringEnvironment {public string SystemRoot=>@"C:\fixture";public bool IsOnBattery=>false;public long GetFreeBytes(string root)=>20L*1024*1024*1024;public VolumeSpace GetVolumeSpace(string root)=>new(40L*1024*1024*1024,GetFreeBytes(root));public bool IsLocalRoot(string root)=>true;}
    private sealed class HeldVolumeEnvironment : IMonitoringEnvironment
    {
        public TaskCompletionSource Started=new(TaskCreationOptions.RunContinuationsAsynchronously),Release=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public string SystemRoot=>@"C:\fixture";public bool IsOnBattery=>false;public long GetFreeBytes(string root)=>20L*1024*1024*1024;public bool IsLocalRoot(string root)=>true;
        public VolumeSpace GetVolumeSpace(string root){Started.TrySetResult();Release.Task.GetAwaiter().GetResult();return new(40L*1024*1024*1024,GetFreeBytes(root));}
    }
    private sealed class NoRegistry : IRunRegistry {public string? Read(string name)=>null;public void Write(string name,string value)=>throw new InvalidOperationException("No registry writes in coordination tests.");public void Delete(string name)=>throw new InvalidOperationException("No registry writes in coordination tests.");}
    private sealed class HeldState : IMonitoringStateStore
    {
        public string? LastUserMessage=>null;public TaskCompletionSource Started=new(TaskCreationOptions.RunContinuationsAsynchronously),Release=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public async Task<AlertSuppressionState> LoadAlertStateAsync(CancellationToken ct){Started.TrySetResult();await Release.Task.WaitAsync(ct);return new();}
        public Task SaveAlertStateAsync(AlertSuppressionState state,CancellationToken ct)=>Task.CompletedTask;
    }
}
