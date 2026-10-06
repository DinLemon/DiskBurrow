using DiskBurrow.App.Services;
using System.Windows.Threading;
using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using System.Windows.Controls;
using System.Windows.Data;
namespace DiskBurrow.Tests;
public class ShutdownDrainTests
{
    [Fact] public async Task RuntimeDisposalWaitsForForegroundCancellationAndFinally()
    {
        using var tree=new TempTree();
        await OnRunningStaDispatcher(async dispatcher=>
        {
            var runtime=await AppRuntime.CreateAsync(tree.Root,dispatcher,new NoRegistry());
            var ui=new WpfDispatcher(dispatcher);
            var cancelled=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            var release=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            bool finallyCompleted=false,foregroundFinalizedOnDispatcher=false;
            runtime.Model.StateChanged+=()=>{if(!runtime.Model.Busy)foregroundFinalizedOnDispatcher=dispatcher.CheckAccess();};
            var action=runtime.Model.RunActionAsync("Status.Exported",async ct=>
            {
                try{await Task.Delay(Timeout.Infinite,ct).ConfigureAwait(false);}
                finally
                {
                    cancelled.TrySetResult();await release.Task.ConfigureAwait(false);
                    await ui.InvokeAsync(()=>finallyCompleted=dispatcher.CheckAccess());
                }
            });
            var disposal=runtime.DisposeAsync().AsTask();
            try
            {
                await cancelled.Task;
                Assert.False(disposal.IsCompleted);Assert.False(finallyCompleted);Assert.True(runtime.Model.Busy);
                // Session ending may stop pumping while finalization is held; normal exit keeps awaiting it.
                Assert.False(BoundedDispatcherDrain.Wait(disposal,dispatcher,TimeSpan.FromMilliseconds(20)));
                Assert.False(action.IsCompleted);Assert.False(disposal.IsCompleted);
            }
            finally{release.TrySetResult();await Task.WhenAll(action,disposal);}
            Assert.True(finallyCompleted);Assert.True(foregroundFinalizedOnDispatcher);Assert.False(runtime.Model.Busy);
            bool restarted=false;
            await runtime.Model.RunActionAsync("Status.Exported",_=>{restarted=true;return Task.CompletedTask;});
            Assert.False(restarted);
        });
    }
    [Fact] public async Task DispatcherContinuationCanPublishPartialReportAndJournalDuringBoundedDrain()
    {
        await OnStaThread(()=>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;var ui=new WpfDispatcher(dispatcher);using var cancel=new CancellationTokenSource();
            bool reportPublished=false,journalSaved=false;
            async Task CleanupOperation()
            {
                try{await Task.Delay(Timeout.Infinite,cancel.Token).ConfigureAwait(false);}catch(OperationCanceledException){ }
                await ui.InvokeAsync(()=>reportPublished=dispatcher.CheckAccess());journalSaved=reportPublished;
            }
            var cleanup=CleanupOperation();cancel.Cancel();
            Assert.True(BoundedDispatcherDrain.Wait(cleanup,dispatcher,TimeSpan.FromSeconds(1)));
            Assert.True(reportPublished);Assert.True(journalSaved);
        });
    }
    [Fact] public async Task ShutdownDrainHasFiniteBoundEvenWhenOperationDoesNotFinish()
    {
        await OnStaThread(()=>Assert.False(BoundedDispatcherDrain.Wait(new TaskCompletionSource().Task,Dispatcher.CurrentDispatcher,TimeSpan.FromMilliseconds(20))));
    }
    [Fact] public async Task HistoryStorageReadRunsOutsideDispatcher()
    {
        await OnStaThread(()=>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;var store=new ObservedStore(dispatcher);var vm=new HistoryViewModel(store,new WpfDispatcher(dispatcher));
            var snapshot=new ScanSnapshot(Guid.NewGuid(),@"C:\fixture",DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[],[],[]);
            Assert.True(BoundedDispatcherDrain.Wait(vm.LoadAsync(snapshot,default),dispatcher,TimeSpan.FromSeconds(1)));Assert.False(store.ReadOnDispatcher);
        });
    }
    [Fact] public async Task CleanupJournalWriteRunsOutsideDispatcher()
    {
        await OnStaThread(()=>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;var store=new ObservedStore(dispatcher);var candidate=new CleanupCandidate(Guid.NewGuid(),new("UserTemp",@"C:\fixture",TimeSpan.FromDays(7),null),new(@"C:\fixture\old",null,1,1,DateTimeOffset.UtcNow.AddDays(-8),1,0),"Cleanup.OldTemp");
            var vm=new CleanupViewModel(new TinyPlanner(candidate),new TinyExecutor(),store,new WpfDispatcher(dispatcher));
            Assert.True(BoundedDispatcherDrain.Wait(vm.AnalyzeAsync(default),dispatcher,TimeSpan.FromSeconds(1)));vm.Select(candidate.Id,true);
            Assert.True(BoundedDispatcherDrain.Wait(vm.ExecuteSelectedAsync(true,default),dispatcher,TimeSpan.FromSeconds(1)));Assert.False(store.WriteOnDispatcher);Assert.NotNull(vm.Report);
        });
    }
    [Fact] public async Task FormDetectsAndClearsInvalidSettingsBinding()
    {
        await OnStaThread(()=>
        {
            var panel=new StackPanel();var field=new System.Windows.Controls.TextBox();panel.Children.Add(field);
            field.SetBinding(System.Windows.Controls.TextBox.TextProperty,new Binding("Value"){Source=new NumberValue()});
            var expression=field.GetBindingExpression(System.Windows.Controls.TextBox.TextProperty)!;
            Validation.MarkInvalid(expression,new ValidationError(new ExceptionValidationRule(),expression,"invalid fixture input",null));
            Assert.True(FormValidation.HasErrors(panel));Validation.ClearInvalid(expression);Assert.False(FormValidation.HasErrors(panel));
        });
    }
    private sealed class NumberValue {public int Value {get;set;}=15;}
    private sealed class NoRegistry : DiskBurrow.Windows.System.IRunRegistry {public string? Read(string name)=>null;public void Write(string name,string value)=>throw new InvalidOperationException("No autostart in lifecycle tests.");public void Delete(string name)=>throw new InvalidOperationException("No autostart in lifecycle tests.");}
    private sealed class ObservedStore(Dispatcher dispatcher) : IHistoryStore
    {
        public bool ReadOnDispatcher,WriteOnDispatcher;
        public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct){ReadOnDispatcher=dispatcher.CheckAccess();return Task.FromResult<IReadOnlyList<ScanSnapshot>>([]);}
        public Task AppendCleanupAsync(CleanupReport report,CancellationToken ct){WriteOnDispatcher=dispatcher.CheckAccess();return Task.CompletedTask;}
        public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>Task.FromResult(new HistoryWriteResult(true,null));
    }
    private sealed class TinyPlanner(CleanupCandidate candidate) : ICleanupPlanner {public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excluded,CancellationToken ct)=>Task.FromResult(new CleanupPlan(Guid.NewGuid(),DateTimeOffset.UtcNow,[candidate]));}
    private sealed class TinyExecutor : ICleanupExecutor {public Task<CleanupReport> ExecuteAsync(CleanupPlan plan,IReadOnlySet<Guid> ids,CancellationToken ct)=>Task.FromResult(new CleanupReport(plan.Id,ids.Select(id=>new CleanupItemResult(id,CleanupOutcome.Deleted,null)).ToArray(),0));}
    private static Task OnRunningStaDispatcher(Func<Dispatcher,Task> action)
    {
        var result=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread=new Thread(()=>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;
            SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(dispatcher));
            Exception? failure=null;
            _ = dispatcher.InvokeAsync(async ()=>
            {
                try{await action(dispatcher);}
                catch(Exception error){failure=error;}
                finally{dispatcher.BeginInvokeShutdown(DispatcherPriority.Send);}
            });
            Dispatcher.Run();
            if(failure is null)result.SetResult();else result.SetException(failure);
        }){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();
        // Harness watchdog only: ordinary runtime disposal has no session-ending deadline.
        return result.Task.WaitAsync(TimeSpan.FromSeconds(20));
    }
    private static Task OnStaThread(Action action)
    {
        var result=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread=new Thread(()=>{try{action();result.SetResult();}catch(Exception error){result.SetException(error);}finally{Dispatcher.CurrentDispatcher.InvokeShutdown();}}){IsBackground=true};
        thread.SetApartmentState(ApartmentState.STA);thread.Start();return result.Task.WaitAsync(TimeSpan.FromSeconds(5));
    }
}
