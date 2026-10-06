using DiskBurrow.App.Services;
using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;

namespace DiskBurrow.Tests;

public class ManualDeletionCoordinationTests
{
    [Fact] public async Task ExplicitFastScanUsesSeparateEngineAndNeverStartsOrdinaryScanner()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;
        model.Root=Path.GetPathRoot(tree.Root)!;
        await model.HandleActionAsync("ScanFast");
        Assert.Equal(1,fixture.FastScanner.Invocations);Assert.Equal(0,fixture.Scanner.Invocations);
        Assert.NotNull(fixture.Coordinator.LiveSnapshot?.Tree);Assert.False(model.Busy);
    }
    [Fact] public async Task RejectedFastScanRestoresIdleWithoutOrdinaryFallback()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;
        model.Root=Path.GetPathRoot(tree.Root)!;fixture.FastScanner.Reject=true;
        await model.HandleActionAsync("ScanFast");
        Assert.Equal(1,fixture.FastScanner.Invocations);Assert.Equal(0,fixture.Scanner.Invocations);
        Assert.False(model.Busy);Assert.Null(fixture.Coordinator.LiveSnapshot);
        Assert.Equal(LocalizationService.ReadText(model.Language,"Status.Cancelled"),model.Status);
    }
    [Fact] public async Task FastScanDoesNotLaunchForIndividualFolderOrInvalidateItsReview()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;
        await model.ShowSnapshotAsync(fixture.Snapshot);model.LargestFolders[0].Selected=true;await model.AnalyzeManualAsync();
        var reviewed=model.Manual.Plan;Assert.False(model.CanFastScan);
        await model.HandleActionAsync("ScanFast");
        Assert.Equal(0,fixture.FastScanner.Invocations);Assert.Equal(0,fixture.Scanner.Invocations);Assert.Same(reviewed,model.Manual.Plan);
    }
    [Fact] public async Task RenderedReadOnlyReviewIsAvailableWhileJournalIsPending()
    {
        var start=new System.Diagnostics.ProcessStartInfo("dotnet"){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true};
        start.ArgumentList.Add(typeof(ManualDeletionCoordinationTests).Assembly.Location);start.ArgumentList.Add("--verify-live-manual-report");
        using var child=System.Diagnostics.Process.Start(start)!;
        var output=child.StandardOutput.ReadToEndAsync();var error=child.StandardError.ReadToEndAsync();
        try{await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(30));}
        catch{if(!child.HasExited)child.Kill();throw;}
        Assert.True(child.ExitCode==0,$"Live report WPF child failed: {await output} {await error}");
    }
    internal static async Task VerifyLiveReportAsync()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;
        fixture.History.HoldJournal=true;
        var view=new DiskBurrow.App.Views.LargestView{DataContext=model};
        var window=new Window{Width=1000,Height=760,Content=view,Left=-10000,Top=-10000,ShowInTaskbar=false,ShowActivated=false};
        Task? deletion=null;
        try
        {
            await model.ShowSnapshotAsync(fixture.Snapshot);model.LargestFolders[0].Selected=true;
            await model.AnalyzeManualAsync();window.Show();
            deletion=model.DeleteManualAsync(true);
            await fixture.History.JournalStarted.Task.WaitAsync(TimeSpan.FromSeconds(10));
            await Dispatcher.Yield(DispatcherPriority.ApplicationIdle);window.UpdateLayout();
            Assert.True(model.Busy);Assert.NotNull(model.Manual.Report);
            var buttons=VisualChildren(view).OfType<Button>().ToArray();
            Assert.True(buttons.Single(button=>Equals(button.Tag,"ReviewManual")).IsEnabled);
            Assert.False(buttons.Single(button=>Equals(button.Tag,"DeleteManual")).IsEnabled);
            Assert.False(buttons.Single(button=>Equals(button.Tag,"AnalyzeManual")).IsEnabled);
            Exception? reviewFailure=null;
            _=window.Dispatcher.BeginInvoke(()=>
            {
                var review=Application.Current.Windows.OfType<DiskBurrow.App.Views.ManualDeletionReviewWindow>().SingleOrDefault();
                try
                {
                    Assert.NotNull(review);review.UpdateLayout();
                    Assert.Single(VisualChildren(review).OfType<DataGrid>().Single().Items);
                    Assert.Contains(VisualChildren(review).OfType<Button>(),button=>button.IsCancel);
                    Assert.DoesNotContain(VisualChildren(review).OfType<Button>(),button=>Equals(button.Tag,"DeleteManual")||Equals(button.Content,LocalizationService.ReadText(model.Language,"Manual.Delete")));
                    Assert.True(model.Busy);
                }
                catch(Exception error){reviewFailure=error;}
                finally{review?.Close();}
            });
            await model.HandleActionAsync("ReviewManual");
            if(reviewFailure is not null)throw reviewFailure;
        }
        finally{fixture.History.ReleaseJournal.TrySetResult();if(deletion is not null)await deletion;window.Close();}
    }
    private static IEnumerable<DependencyObject> VisualChildren(DependencyObject root)
    {
        yield return root;
        for(var index=0;index<VisualTreeHelper.GetChildrenCount(root);index++)
            foreach(var child in VisualChildren(VisualTreeHelper.GetChild(root,index)))yield return child;
    }
    [Fact] public async Task FolderAndFileMarksHaveSeparateReviewAndConfirmationAndBecomeStaleAfterAttempt()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;
        await model.ShowSnapshotAsync(fixture.Snapshot);Assert.False(model.CanAnalyzeManual);
        var focusedRow=model.LargestFolders[0];focusedRow.Selected=true;Assert.Same(focusedRow,model.LargestFolders[0]);
        model.Language="en";Assert.Same(focusedRow,model.LargestFolders[0]);Assert.True(focusedRow.Selected);model.LargestFiles[0].Selected=true;
        await model.AnalyzeManualAsync();Assert.Equal(2,fixture.Service.Reviewed.Length);Assert.True(model.CanDeleteManual);
        await model.DeleteManualAsync(false);Assert.Equal(0,fixture.Service.Executions);
        await model.DeleteManualAsync(true);Assert.Equal(1,fixture.Service.Executions);Assert.NotNull(model.Manual.Report);Assert.True(model.Manual.SnapshotStale);
        Assert.All(model.LargestFolders,row=>Assert.False(row.Selected));Assert.All(model.LargestFiles,row=>Assert.False(row.Selected));Assert.Empty(model.Manual.SelectedPaths);
        Assert.Same(fixture.Snapshot,model.Overview.Snapshot);Assert.Equal(0,fixture.Scanner.Invocations);
        await model.DeleteManualAsync(true);Assert.Equal(1,fixture.Service.Executions);
    }
    [Fact] public async Task RootSwitchDuringPreviewDiscardsPendingPlanAndMarks()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;fixture.Service.PreviewGate=new(TaskCreationOptions.RunContinuationsAsynchronously);
        await model.ShowSnapshotAsync(fixture.Snapshot);model.LargestFolders[0].Selected=true;var preview=model.AnalyzeManualAsync();await fixture.Service.PreviewStarted.Task.WaitAsync(TimeSpan.FromSeconds(20));
        model.Root=@"E:\other-fixture";fixture.Service.PreviewGate.SetResult(fixture.Service.Plan);await preview;
        Assert.Null(model.Manual.Plan);Assert.Empty(model.Manual.SelectedPaths);Assert.False(model.CanDeleteManual);Assert.Equal(0,fixture.Service.Executions);
    }
    [Fact] public async Task ForegroundManualOperationHoldsCoordinatorExclusivityAndDrainWaitsForFinally()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;
        await model.ShowSnapshotAsync(fixture.Snapshot);model.LargestFolders[0].Selected=true;await model.AnalyzeManualAsync();fixture.Service.HoldExecution=true;
        var deletion=model.DeleteManualAsync(true);await fixture.Service.ExecuteStarted.Task.WaitAsync(TimeSpan.FromSeconds(20));Assert.True(model.Busy);Assert.True(fixture.Coordinator.OperationRunning);
        await model.ScanAsync();await model.AnalyzeManualAsync();Assert.Equal(0,fixture.Scanner.Invocations);Assert.Equal(1,fixture.Service.Previews);
        var drain=model.StopAsync();await fixture.Service.Cancelled.Task.WaitAsync(TimeSpan.FromSeconds(20));Assert.False(drain.IsCompleted);Assert.True(model.Busy);
        fixture.Service.ReleaseFinalization.TrySetResult();await Task.WhenAll(deletion,drain).WaitAsync(TimeSpan.FromSeconds(20));Assert.False(model.Busy);Assert.True(model.Manual.Report!.WasCancelled);
    }
    [Fact] public async Task NewSnapshotClearsReviewAndRowsWithoutChangingAutomaticCleanupMarks()
    {
        using var tree=new TempTree();await using var fixture=new Fixture(tree.Root);var model=fixture.Model;
        await model.ShowSnapshotAsync(fixture.Snapshot);model.LargestFolders[0].Selected=true;await model.AnalyzeManualAsync();
        await model.ShowSnapshotAsync(fixture.Snapshot with {Id=Guid.NewGuid()});Assert.Null(model.Manual.Plan);Assert.Empty(model.Manual.SelectedPaths);Assert.False(model.LargestFolders[0].Selected);Assert.Empty(model.Cleanup.SelectedIds);
    }
    private sealed class Fixture : IAsyncDisposable
    {
        public Service Service=new();public Scanner Scanner=new();public FastScanner FastScanner=new();public History History=new();public MonitoringCoordinator Coordinator;public MainViewModel Model;
        public ScanSnapshot Snapshot=new(Guid.NewGuid(),@"E:\fixture",DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[new(@"E:\fixture\folder",42,42,true)],[new(@"E:\fixture\file.bin",null,12,12,DateTimeOffset.UtcNow,1,0)],[]);
        public Fixture(string directory)
        {
            var ui=new InlineDispatcher();var history=History;var settings=new AppSettings();var env=new Environment();Coordinator=new(settings,Scanner,history,env,new SettingsStore(directory));
            Model=new(Coordinator,env,ui,new LocalizationService(),new(ui),new(history,ui),new(new Planner(),new Executor(),history,ui),new(settings,new SettingsStore(directory),new AutostartRegistration(new Registry(),@"E:\fixture\DiskBurrow.exe"),_=>null,_=>{}),settings,new(Service,history,ui),FastScanner);
        }
        public async ValueTask DisposeAsync(){Service.ReleaseFinalization.TrySetResult();await Model.StopAsync();await Coordinator.DisposeAsync();}
    }
    private sealed class Service : IManualDeletionService
    {
        public int Previews,Executions;public string[] Reviewed=[];public bool HoldExecution;
        public ManualDeletePlan Plan=new(Guid.NewGuid(),DateTimeOffset.UtcNow,[@"E:\fixture\folder"],[new(Guid.NewGuid(),@"E:\fixture\folder",new(@"E:\fixture\folder",null,0,0,DateTimeOffset.UtcNow,1,FileAttributes.Directory))],[]);
        public TaskCompletionSource<ManualDeletePlan>? PreviewGate;
        public TaskCompletionSource PreviewStarted=new(TaskCreationOptions.RunContinuationsAsynchronously),ExecuteStarted=new(TaskCreationOptions.RunContinuationsAsynchronously),Cancelled=new(TaskCreationOptions.RunContinuationsAsynchronously),ReleaseFinalization=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task<ManualDeletePlan> PreviewAsync(IEnumerable<string> selectedPaths,CancellationToken ct){Previews++;Reviewed=selectedPaths.ToArray();PreviewStarted.TrySetResult();return PreviewGate?.Task??Task.FromResult(Plan);}
        public async Task<CleanupReport> ExecuteConfirmedAsync(Guid id,bool permanentDeletionConfirmed,CancellationToken ct)
        {
            Assert.True(permanentDeletionConfirmed);Assert.Equal(Plan.Id,id);Executions++;ExecuteStarted.TrySetResult();var cancelled=false;
            if(HoldExecution){try{await Task.Delay(Timeout.Infinite,ct);}catch(OperationCanceledException){cancelled=true;Cancelled.TrySetResult();await ReleaseFinalization.Task;}}
            return new(Plan.Id,[new(Plan.Entries[0].Id,CleanupOutcome.Deleted,null){Audit=new(Plan.Roots[0],"Manual",Plan.Roots[0],DateTimeOffset.UtcNow,0,0,1,null)}],0){WasCancelled=cancelled,FreeSpaceDeltaAvailable=false};
        }
    }
    private sealed class Scanner : IDiskScanner {public int Invocations;public Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? progress,CancellationToken ct){Invocations++;throw new InvalidOperationException("Tests must never start a scan implicitly.");}}
    private sealed class FastScanner : IDiskScanner
    {
        public int Invocations;public bool Reject;
        public Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? progress,CancellationToken ct)
        {
            Invocations++;if(Reject)return Task.FromException<ScanSnapshot>(new OperationCanceledException("UAC rejected"));
            var builder=new ScanTreeBuilder(root);builder.AddFile(0,"fixture.bin",42,42,DateTimeOffset.UtcNow);
            return Task.FromResult(new ScanSnapshot(Guid.NewGuid(),root,DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[new(root,42,42,true)],[],[]){Tree=builder.Build()});
        }
    }
    private sealed class History : IHistoryStore
    {
        public bool HoldJournal;
        public TaskCompletionSource JournalStarted=new(TaskCreationOptions.RunContinuationsAsynchronously),ReleaseJournal=new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>Task.FromResult(new HistoryWriteResult(true,null));
        public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>Task.FromResult<IReadOnlyList<ScanSnapshot>>([]);
        public async Task AppendCleanupAsync(CleanupReport report,CancellationToken ct){if(HoldJournal){JournalStarted.TrySetResult();await ReleaseJournal.Task;}}
    }
    private sealed class Environment : IMonitoringEnvironment {public string SystemRoot=>@"E:\fixture";public bool IsOnBattery=>false;public bool IsLocalRoot(string root)=>true;public long GetFreeBytes(string root)=>20;public VolumeSpace GetVolumeSpace(string root)=>new(40,20);}
    private sealed class Registry : IRunRegistry {public string? Read(string name)=>null;public void Write(string name,string value)=>throw new InvalidOperationException();public void Delete(string name)=>throw new InvalidOperationException();}
    private sealed class Planner : ICleanupPlanner {public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excludedPaths,CancellationToken ct)=>Task.FromResult(new CleanupPlan(Guid.NewGuid(),DateTimeOffset.UtcNow,[]));}
    private sealed class Executor : ICleanupExecutor {public Task<CleanupReport> ExecuteAsync(CleanupPlan plan,IReadOnlySet<Guid> selectedIds,CancellationToken ct)=>throw new InvalidOperationException("Manual marks cannot invoke automatic cleanup.");}
}
