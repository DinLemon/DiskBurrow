using System.Text.Json;
using System.Windows.Threading;
using DiskBurrow.App.Services;
using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.History;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;

namespace DiskBurrow.Tests;

public class FinalIntegrationTests
{
    [Fact] public async Task CleanupCategorySummaryLocalizesWithoutDiscardingSelectedCandidate()
    {
        using var tree=new TempTree();await using var fixture=new ModelFixture(tree.Root,new MemoryHistory());await fixture.Model.AnalyzeAsync();
        var model=fixture.Model;var id=model.Cleanup.VisibleCandidates[0].Id;model.Cleanup.Select(id,true);
        model.Language="en";
        Assert.Equal(4,model.CleanupCategorySummaries.Count);
        Assert.Equal(LocalizationService.ReadText("en","UserTemp"),model.CleanupCategorySummaries[0].Category);
        Assert.Equal(LocalizationService.ReadText("en","Cleanup.NoEligible"),model.CleanupCategorySummaries[1].Status);
        Assert.Equal(LocalizationService.ReadText("en","UserTemp"),Assert.Single(model.CleanupRows).Category);
        model.Language="ru";
        Assert.Equal(LocalizationService.ReadText("ru","UserTemp"),model.CleanupCategorySummaries[0].Category);
        Assert.Contains(id,model.Cleanup.SelectedIds);Assert.True(Assert.Single(model.CleanupRows).Selected);
    }
    [Fact] public async Task PausedRuntimeReopensRetainedHistoryWithoutScanning()
    {
        using var tree=new TempTree();var root=new WindowsEnvironment().SystemRoot;
        var snapshot=new ScanSnapshot(Guid.NewGuid(),root,DateTimeOffset.UtcNow.AddDays(-1),DateTimeOffset.UtcNow.AddDays(-1),true,[new(Path.TrimEndingDirectorySeparator(root),42,42,true)],[],[]);
        await new SettingsStore(tree.Root).SaveAsync(new(){Paused=true},default);
        Assert.True((await new SqliteHistoryStore(new(tree.Root)).SaveAsync(snapshot,default)).Saved);
        await Sta(()=>
        {
            var scanner=new NeverScanner();var dispatcher=Dispatcher.CurrentDispatcher;var create=AppRuntime.CreateAsync(tree.Root,dispatcher,new NoRegistry(),scanner:scanner);
            Assert.True(BoundedDispatcherDrain.Wait(create,dispatcher,TimeSpan.FromSeconds(5)));
            var runtime=create.GetAwaiter().GetResult();
            try{Assert.True(runtime.Model.Paused);Assert.Equal(snapshot.Id,runtime.Model.Overview.Snapshot?.Id);Assert.Equal(snapshot.Id,Assert.Single(runtime.Model.History.Snapshots).Id);Assert.Null(runtime.Coordinator.LiveSnapshot);Assert.Equal(0,scanner.Invocations);Assert.False(File.Exists(Path.Combine(tree.Root,"alerts.json")));}
            finally{Assert.True(BoundedDispatcherDrain.Wait(runtime.DisposeAsync().AsTask(),dispatcher,TimeSpan.FromSeconds(5)));}
        });
    }
    [Fact] public async Task PartialDurableAuditIdentifiesFileAfterDiscardingPlan()
    {
        using var tree=new TempTree();var history=new SqliteHistoryStore(new(tree.Root));
        var candidate=new CleanupCandidate(Guid.NewGuid(),new("UserTemp",@"C:\fixture",TimeSpan.FromDays(7),null),new(@"C:\fixture\old.bin",null,12,12,DateTimeOffset.UtcNow.AddDays(-8),1,0),"Cleanup.TempAge");
        var vm=new CleanupViewModel(new Planner(new(Guid.NewGuid(),DateTimeOffset.UtcNow,[candidate])),new PartialExecutor(),history,new InlineDispatcher(),()=>history.LastUserMessage);
        await vm.AnalyzeAsync(default);vm.Select(candidate.Id,true);await vm.ExecuteSelectedAsync(true,default);
        Assert.True(vm.Report!.WasCancelled);Assert.Null(vm.JournalError);
        using var db=new Microsoft.Data.Sqlite.SqliteConnection($"Data Source={history.DatabasePath};Pooling=False");db.Open();
        using var command=db.CreateCommand();command.CommandText="SELECT payload FROM cleanup_reports";
        var retained=JsonSerializer.Deserialize<CleanupReport>((byte[])command.ExecuteScalar()!)!;
        vm=null!;candidate=null!;
        var item=Assert.Single(retained.Items);Assert.Equal(@"C:\fixture\old.bin",item.Audit?.Path);Assert.Equal("UserTemp",item.Audit?.RuleId);Assert.Equal(12,item.Audit?.LogicalBytes);Assert.Equal(CleanupOutcome.Deleted,item.Outcome);Assert.True(retained.WasCancelled);
    }
    [Fact] public void EmittedCleanupReasonsAndRunningStatusesAreMembersOfBothLocales()
    {
        foreach(var language in new[]{"ru","en"})foreach(var key in new[]{"Cleanup.Missing","Cleanup.SkippedBusy","Cleanup.Failed","Status.Saving","Status.Exporting"})
            Assert.Contains(key,LocalizationService.ReadKeys(language));
    }
    [Fact] public async Task OldAlertLoadsCorrespondingRootSnapshotComparisonAndNonTopFolder()
    {
        using var tree=new TempTree();var store=new SqliteHistoryStore(new(tree.Root));var prior=Snapshot(@"C:\fixture",1);
        var folder=@"C:\fixture\not-in-top";
        var current=Snapshot(prior.Root,2) with {Directories=Enumerable.Range(0,1001).Select(i=>new DirectoryObservation($@"C:\fixture\large-{i}",10000+i,10000+i,true)).Append(new(folder,5,5,true)).ToArray()};
        var other=Snapshot(@"E:\fixture",3);foreach(var snapshot in new[]{prior,current,other})Assert.True((await store.SaveAsync(snapshot,default)).Saved);
        await using var fixture=new ModelFixture(tree.Root,store);await fixture.Model.ShowSnapshotAsync(other);
        await fixture.Model.ActivateAlertAsync(new(AlertKind.FolderGrowth,folder,5,current.Id,current.Root,prior.Id));
        Assert.Equal(current.Root,fixture.Model.Root);Assert.Equal(current.Id,fixture.Model.Overview.Snapshot?.Id);
        Assert.Equal(prior.CompletedUtc,fixture.Model.History.PreviousUtc);Assert.Equal(current.Id,fixture.Model.History.SelectedSnapshotId);
        Assert.Equal(folder,fixture.Model.Overview.FocusedDirectory?.Path);Assert.Contains(fixture.Model.Overview.Directories,d=>d.Path==folder);Assert.Equal(1000,fixture.Model.Overview.Directories.Count);
        Assert.Equal(0,fixture.Scanner.Invocations);Assert.False(File.Exists(Path.Combine(tree.Root,"alerts.json")));
    }
    [Fact] public async Task ExpiredAlertClearsUnrelatedResultAndMissingComparisonIsExplicit()
    {
        using var tree=new TempTree();var store=new SqliteHistoryStore(new(tree.Root));var snapshot=Snapshot(@"C:\fixture",2);await store.SaveAsync(snapshot,default);
        await using var fixture=new ModelFixture(tree.Root,store);await fixture.Model.ShowSnapshotAsync(Snapshot(@"E:\fixture",3));
        await fixture.Model.ActivateAlertAsync(new(AlertKind.FolderGrowth,snapshot.Root,5,Guid.NewGuid(),snapshot.Root));
        Assert.Null(fixture.Model.Overview.Snapshot);Assert.Equal(LocalizationService.ReadText("ru","Alert.Unavailable"),fixture.Model.AlertMessage);
        await fixture.Model.ActivateAlertAsync(new(AlertKind.FolderGrowth,snapshot.Root,5,snapshot.Id,snapshot.Root,Guid.NewGuid()));
        Assert.Equal(snapshot.Id,fixture.Model.Overview.Snapshot?.Id);Assert.True(fixture.Model.History.ComparisonUnavailable);Assert.Equal(LocalizationService.ReadText("ru","Alert.ComparisonUnavailable"),fixture.Model.AlertMessage);Assert.Equal(0,fixture.Scanner.Invocations);
    }
    [Fact] public async Task EmptyAndCorruptHistoryLoadsAreExplicitAndDoNotScan()
    {
        using var tree=new TempTree();var store=new SqliteHistoryStore(new(tree.Root));await using var fixture=new ModelFixture(tree.Root,store,()=>store.LastUserMessage);
        await fixture.Model.LoadSelectedRootAsync();Assert.Null(fixture.Model.Overview.Snapshot);Assert.Empty(fixture.Model.History.Snapshots);Assert.Null(fixture.Model.History.ErrorDetails);
        var bytes=System.Text.Encoding.UTF8.GetBytes("corrupt history fixture");await File.WriteAllBytesAsync(store.DatabasePath,bytes);
        await fixture.Model.LoadSelectedRootAsync();Assert.Null(fixture.Model.Overview.Snapshot);Assert.NotNull(fixture.Model.History.ErrorDetails);Assert.NotNull(fixture.Model.PrimaryMessage);Assert.Equal(bytes,await File.ReadAllBytesAsync(store.DatabasePath));Assert.Equal(0,fixture.Scanner.Invocations);
    }
    [Fact] public async Task ChangingRootsDuringInitialHistoryReadCannotPublishOldContext()
    {
        using var tree=new TempTree();var history=new HeldHistory();await using var fixture=new ModelFixture(tree.Root,history);
        var initial=fixture.Model.LoadSelectedRootAsync();await history.Started.Task.WaitAsync(TimeSpan.FromSeconds(3));
        fixture.Model.Root=history.New.Root;await fixture.Model.LoadSelectedRootAsync();
        history.Release.TrySetResult();await initial;
        Assert.Equal(history.New.Id,fixture.Model.Overview.Snapshot?.Id);Assert.Equal(history.New.Id,Assert.Single(fixture.Model.History.Snapshots).Id);Assert.Equal(history.New.Root,fixture.Model.Overview.VisibleRoot);Assert.Equal(0,fixture.Scanner.Invocations);
    }
    [Fact] public async Task PausedFakeClockCountersAndAlertAgreeAndNeverOverwriteAnotherRoot()
    {
        using var tree=new TempTree();var clock=new MonitoringCoordinatorTests.ManualClock();var environment=new CounterEnvironment();
        await using var fixture=new ModelFixture(tree.Root,new MemoryHistory(),environment:environment,clock:clock);
        await fixture.Model.LoadSelectedRootAsync();Assert.Equal(20,fixture.Model.Overview.Volume?.FreeBytes);
        var alert=new TaskCompletionSource<AlertEvent>(TaskCreationOptions.RunContinuationsAsynchronously);fixture.Coordinator.AlertsRaised+=alerts=>alert.TrySetResult(alerts[0]);
        var run=fixture.Coordinator.RunAsync(default);await clock.WaitForTimerAsync();environment.SystemFree=1;clock.Advance(TimeSpan.FromMinutes(5));
        var raised=await alert.Task.WaitAsync(TimeSpan.FromSeconds(3));Assert.Equal(raised.Bytes,fixture.Model.Overview.Volume?.FreeBytes);Assert.Equal(clock.GetUtcNow(),fixture.Model.Overview.VolumeObservedUtc);Assert.Equal(environment.SystemRoot,raised.Root);Assert.Equal(0,fixture.Scanner.Invocations);
        fixture.Model.Root=@"E:\fixture";await fixture.Model.LoadSelectedRootAsync();Assert.Equal(7,fixture.Model.Overview.Volume?.FreeBytes);
        var observed=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);fixture.Coordinator.VolumeObserved+=_=>observed.TrySetResult();
        await clock.WaitForTimerAsync();environment.SystemFree=2;clock.Advance(TimeSpan.FromMinutes(5));await observed.Task.WaitAsync(TimeSpan.FromSeconds(3));
        Assert.Equal(7,fixture.Model.Overview.Volume?.FreeBytes);Assert.Equal(@"E:\fixture",fixture.Model.Overview.VisibleRoot);
        await fixture.Model.ActivateAlertAsync(raised);Assert.Equal(environment.SystemRoot,fixture.Model.Root);Assert.Equal(2,fixture.Model.Overview.Volume?.FreeBytes);Assert.Equal(0,fixture.Scanner.Invocations);
        await fixture.Coordinator.StopAsync();await run;
    }
    [Fact] public async Task NewAnalysisClearsOutcomesAndCancelledAnalysisPreservesCompletedReport()
    {
        using var tree=new TempTree();var planner=new Replanner();await using var fixture=new ModelFixture(tree.Root,new MemoryHistory(),planner:planner,executor:new PartialExecutor());
        await fixture.Model.AnalyzeAsync();fixture.Model.Cleanup.Select(fixture.Model.Cleanup.VisibleCandidates[0].Id,true);await fixture.Model.DeleteAsync(true);
        Assert.Single(fixture.Model.Outcomes);var report=fixture.Model.Cleanup.Report;planner.CancelNext=true;
        await fixture.Model.AnalyzeAsync();Assert.Same(report,fixture.Model.Cleanup.Report);Assert.Single(fixture.Model.Outcomes);
        await fixture.Model.AnalyzeAsync();Assert.Null(fixture.Model.Cleanup.Report);Assert.Empty(fixture.Model.Outcomes);
    }
    [Fact] public async Task SaveAndExportShowRunningThenRetainSuccessOnlyAfterCompletion()
    {
        using var tree=new TempTree();await using var fixture=new ModelFixture(tree.Root,new MemoryHistory());var statuses=new List<string>();fixture.Model.PropertyChanged+=(_,_)=>statuses.Add(fixture.Model.Status);
        await fixture.Model.SaveSettingsAsync();Assert.Contains(LocalizationService.ReadText("ru","Status.Saving"),statuses);Assert.Equal(LocalizationService.ReadText("ru","Status.Saved"),fixture.Model.Status);Assert.True(File.Exists(Path.Combine(tree.Root,"settings.json")));
        var output=Path.Combine(tree.Root,"explicit-export.json");await fixture.Model.ExportAsync(Snapshot(@"C:\fixture",1),output);Assert.Contains(LocalizationService.ReadText("ru","Status.Exporting"),statuses);Assert.Equal(LocalizationService.ReadText("ru","Status.Exported"),fixture.Model.Status);
        await fixture.Model.ExportAsync(Snapshot(@"C:\fixture",1),output);Assert.Equal(LocalizationService.ReadText("ru","Status.Error"),fixture.Model.Status);
    }
    [Fact] public void LegacyGuidOnlyReportsRemainExplicitlyUnmapped()
    {
        var report=JsonSerializer.Deserialize<CleanupReport>("{\"PlanId\":\"00000000-0000-0000-0000-000000000001\",\"Items\":[{\"CandidateId\":\"00000000-0000-0000-0000-000000000002\",\"Outcome\":0,\"ReasonKey\":null}],\"FreeSpaceDeltaBytes\":0}")!;
        Assert.Null(Assert.Single(report.Items).Audit);Assert.Equal(CleanupOutcome.Deleted,report.Items[0].Outcome);
    }
    private static ScanSnapshot Snapshot(string root,int day)=>new(Guid.NewGuid(),root,DateTimeOffset.UnixEpoch.AddDays(day),DateTimeOffset.UnixEpoch.AddDays(day),true,[new(root,day,day,true)],[],[]);
    private sealed class ModelFixture:IAsyncDisposable
    {
        public NeverScanner Scanner {get;}=new();public MonitoringCoordinator Coordinator {get;} public MainViewModel Model {get;}
        public ModelFixture(string directory,IHistoryStore store,Func<string?>? diagnostic=null,IMonitoringEnvironment? environment=null,TimeProvider? clock=null,ICleanupPlanner? planner=null,ICleanupExecutor? executor=null)
        {
            environment??=new CounterEnvironment();var settings=new AppSettings{Paused=true,LowSpaceBytes=15};var state=new SettingsStore(directory);var ui=new InlineDispatcher();
            Coordinator=new(settings,Scanner,store,environment,state,clock);var registry=new FakeRegistry();
            Model=new(Coordinator,environment,ui,new(),new(ui),new(store,ui,diagnostic),new(planner??new Replanner(),executor??new PartialExecutor(),store,ui,diagnostic),new(settings,state,new AutostartRegistration(registry,@"C:\fixture\DiskBurrow.exe"),_=>null,_=>{}),settings);
        }
        public async ValueTask DisposeAsync(){await Model.StopAsync();await Coordinator.DisposeAsync();}
    }
    private sealed class NeverScanner:IDiskScanner {public int Invocations;public Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? progress,CancellationToken ct){Invocations++;throw new InvalidOperationException("No traversal allowed.");}}
    private sealed class CounterEnvironment:IMonitoringEnvironment {public long SystemFree=20;public string SystemRoot=>@"C:\fixture";public bool IsOnBattery=>true;public long GetFreeBytes(string root)=>root.StartsWith("E:")?7:SystemFree;public VolumeSpace GetVolumeSpace(string root)=>new(100,GetFreeBytes(root));public bool IsLocalRoot(string root)=>true;}
    private class MemoryHistory:IHistoryStore {public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>Task.FromResult(new HistoryWriteResult(true,null));public virtual Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>Task.FromResult<IReadOnlyList<ScanSnapshot>>([]);public Task AppendCleanupAsync(CleanupReport report,CancellationToken ct)=>Task.CompletedTask;}
    private sealed class HeldHistory:MemoryHistory
    {
        public ScanSnapshot New=Snapshot(@"E:\fixture",2);public TaskCompletionSource Started=new(TaskCreationOptions.RunContinuationsAsynchronously),Release=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public override async Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct){if(root==New.Root)return [New];Started.TrySetResult();await Release.Task.WaitAsync(ct);return [Snapshot(root,1)];}
    }
    private sealed class Replanner:ICleanupPlanner {public bool CancelNext;public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excluded,CancellationToken ct){if(CancelNext){CancelNext=false;throw new OperationCanceledException();}return Task.FromResult(new CleanupPlan(Guid.NewGuid(),DateTimeOffset.UtcNow,[new(Guid.NewGuid(),new("UserTemp",@"C:\fixture",TimeSpan.FromDays(7),null),new(@"C:\fixture\old.bin",null,12,12,DateTimeOffset.UnixEpoch,1,0),"Cleanup.TempAge")]));}}
    private sealed class FakeRegistry:IRunRegistry {public string? Read(string name)=>null;public void Write(string name,string value){}public void Delete(string name){} }
    private static Task Sta(Action action)
    {
        var done=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread=new Thread(()=>{try{action();done.SetResult();}catch(Exception error){done.SetException(error);}finally{Dispatcher.CurrentDispatcher.InvokeShutdown();}});
        thread.SetApartmentState(ApartmentState.STA);thread.Start();return done.Task.WaitAsync(TimeSpan.FromSeconds(20));
    }
    private sealed class NoRegistry:IRunRegistry {public string? Read(string name)=>null;public void Write(string name,string value)=>throw new InvalidOperationException();public void Delete(string name)=>throw new InvalidOperationException();}
    private sealed class Planner(CleanupPlan plan):ICleanupPlanner {public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excluded,CancellationToken ct)=>Task.FromResult(plan);}
    private sealed class PartialExecutor:ICleanupExecutor {public Task<CleanupReport> ExecuteAsync(CleanupPlan plan,IReadOnlySet<Guid> selected,CancellationToken ct)=>Task.FromResult(new CleanupReport(plan.Id,[new(selected.Single(),CleanupOutcome.Deleted,null)],0){WasCancelled=true,FreeSpaceDeltaAvailable=false});}
}
