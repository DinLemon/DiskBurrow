using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using DiskBurrow.Core.Scanning;

namespace DiskBurrow.Tests;

public class ManualDeletionPresentationTests
{
    private const string PathA = @"E:\fixture\folder";
    private const string PathB = @"E:\fixture\other.bin";
    private static ManualDeletionViewModel Vm(Service service,IHistoryStore? history=null) => new(service,history,new InlineDispatcher());
    [Fact] public async Task AnalysisDoesNotDeleteAndFalseConfirmationPreservesReviewedPlan()
    {
        var service=new Service();var vm=Vm(service);
        Assert.Empty(vm.SelectedPaths);Assert.False(vm.CanAnalyze);
        vm.Select(PathA,true);await vm.AnalyzeAsync(default);
        Assert.Equal(new[]{PathA},service.Reviewed);Assert.True(vm.CanExecute);
        await vm.ExecuteAsync(false,default);Assert.Equal(0,service.Executions);Assert.NotNull(vm.Plan);
        await vm.ExecuteAsync(true,default);Assert.Equal(1,service.Executions);Assert.Empty(vm.SelectedPaths);Assert.NotNull(vm.Report);Assert.False(vm.CanExecute);Assert.True(vm.SnapshotStale);
        await vm.ExecuteAsync(true,default);Assert.Equal(1,service.Executions);
    }
    [Fact] public async Task SelectionChangeRequiresAnotherPreview()
    {
        var service=new Service();var vm=Vm(service);vm.Select(PathA,true);await vm.AnalyzeAsync(default);
        vm.Select(PathB,true);Assert.Null(vm.Plan);Assert.False(vm.CanExecute);
        await vm.ExecuteAsync(true,default);Assert.Equal(0,service.Executions);
        await vm.AnalyzeAsync(default);Assert.Equal(2,service.Reviewed.Length);Assert.True(vm.CanExecute);
    }
    [Fact] public async Task LatePreviewCannotReturnAfterRootOrSnapshotChanges()
    {
        var service=new Service{PreviewGate=new(TaskCreationOptions.RunContinuationsAsynchronously)};var vm=Vm(service);
        vm.Select(PathA,true);var task=vm.AnalyzeAsync(default);await service.PreviewStarted.Task;
        vm.InvalidateContext();service.PreviewGate.SetResult(Plan(PathA));await task;
        Assert.Null(vm.Plan);Assert.Empty(vm.SelectedPaths);Assert.False(vm.CanExecute);
    }
    [Fact] public async Task PartialReportAndJournalErrorSurviveAndCannotReplay()
    {
        var service=new Service{Partial=true};var vm=Vm(service,new BrokenHistory());vm.Select(PathA,true);await vm.AnalyzeAsync(default);await vm.ExecuteAsync(true,default);
        Assert.True(vm.Report!.WasCancelled);Assert.Equal(PathA,Assert.Single(vm.Report.Items).Audit!.Path);Assert.Equal("journal unavailable",vm.JournalError);
        await vm.ExecuteAsync(true,default);Assert.Equal(1,service.Executions);Assert.Null(vm.Plan);Assert.Empty(vm.SelectedPaths);
    }
    [Fact] public async Task DuplicateAnalysisAndDeleteCannotRunWhileOperationIsBusy()
    {
        var service=new Service{PreviewGate=new(TaskCreationOptions.RunContinuationsAsynchronously)};var vm=Vm(service);vm.Select(PathA,true);
        var preview=vm.AnalyzeAsync(default);await service.PreviewStarted.Task;await vm.AnalyzeAsync(default);await vm.ExecuteAsync(true,default);
        Assert.Equal(1,service.Previews);Assert.Equal(0,service.Executions);service.PreviewGate.SetResult(Plan(PathA));await preview;
        service.ExecuteGate=new(TaskCreationOptions.RunContinuationsAsynchronously);var execution=vm.ExecuteAsync(true,default);await service.ExecuteStarted.Task;
        await vm.ExecuteAsync(true,default);vm.Select(PathB,true);Assert.Equal(1,service.Executions);Assert.DoesNotContain(PathB,vm.SelectedPaths);
        service.ExecuteGate.SetResult(Report(Plan(PathA)));await execution;Assert.False(vm.Busy);
    }
    [Fact] public async Task UnsafeWarningsRetainOriginalReasonAndBlockExecution()
    {
        var service=new Service{Warning="Manual.UnknownFutureReason"};var vm=Vm(service);vm.Select(PathA,true);await vm.AnalyzeAsync(default);
        Assert.Equal("Manual.UnknownFutureReason",Assert.Single(vm.Plan!.Warnings).ReasonKey);Assert.False(vm.CanExecute);await vm.ExecuteAsync(true,default);Assert.Equal(0,service.Executions);
    }
    [Fact] public async Task LiveReportIsPublishedBeforeSlowJournalCompletes()
    {
        var history=new HeldHistory();var vm=Vm(new Service{Partial=true},history);vm.Select(PathA,true);await vm.AnalyzeAsync(default);
        var execution=vm.ExecuteAsync(true,default);await history.Started.Task.WaitAsync(TimeSpan.FromSeconds(20));
        try{Assert.NotNull(vm.Report);Assert.True(vm.Report.WasCancelled);Assert.Null(vm.Plan);Assert.False(vm.CanExecute);}
        finally{history.Release.TrySetResult();await execution.WaitAsync(TimeSpan.FromSeconds(20));}
        Assert.NotNull(vm.Report);Assert.Equal("journal unavailable",vm.JournalError);
    }
    private static ManualDeletePlan Plan(string path) => new(Guid.NewGuid(),DateTimeOffset.UtcNow,[path],[new(Guid.NewGuid(),path,new(path,null,42,42,DateTimeOffset.UtcNow,1,FileAttributes.Directory))],[]);
    private static CleanupReport Report(ManualDeletePlan plan,bool partial=false) => new(plan.Id,[new(plan.Entries[0].Id,CleanupOutcome.Deleted,null){Audit=new(plan.Roots[0],"Manual",plan.Roots[0],DateTimeOffset.UtcNow,0,0,1,null)}],0){WasCancelled=partial,FreeSpaceDeltaAvailable=false};
    private sealed class Service : IManualDeletionService
    {
        public int Previews,Executions;public string[] Reviewed=[];public bool Partial;public string? Warning;private ManualDeletePlan? plan;
        public TaskCompletionSource<ManualDeletePlan>? PreviewGate;public TaskCompletionSource<CleanupReport>? ExecuteGate;
        public TaskCompletionSource PreviewStarted=new(TaskCreationOptions.RunContinuationsAsynchronously),ExecuteStarted=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public async Task<ManualDeletePlan> PreviewAsync(IEnumerable<string> selected,CancellationToken ct){Previews++;Reviewed=selected.ToArray();PreviewStarted.TrySetResult();plan=PreviewGate is {} gate?await gate.Task:Plan(Reviewed[0]);if(Warning is {} w)plan=plan with {Warnings=[new(Reviewed[0],w)]};return plan;}
        public Task<CleanupReport> ExecuteConfirmedAsync(Guid id,bool confirmed,CancellationToken ct){Assert.True(confirmed);Assert.Equal(plan!.Id,id);Executions++;ExecuteStarted.TrySetResult();return ExecuteGate?.Task??Task.FromResult(Report(plan,Partial));}
    }
    private sealed class BrokenHistory : IHistoryStore
    {
        public Task AppendCleanupAsync(CleanupReport report,CancellationToken ct)=>throw new IOException("journal unavailable");
        public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>throw new NotSupportedException();
        public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>throw new NotSupportedException();
    }
    private sealed class HeldHistory : IHistoryStore
    {
        public TaskCompletionSource Started=new(TaskCreationOptions.RunContinuationsAsynchronously),Release=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public async Task AppendCleanupAsync(CleanupReport report,CancellationToken ct){Started.TrySetResult();await Release.Task;throw new IOException("journal unavailable");}
        public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>throw new NotSupportedException();
        public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>throw new NotSupportedException();
    }
}
