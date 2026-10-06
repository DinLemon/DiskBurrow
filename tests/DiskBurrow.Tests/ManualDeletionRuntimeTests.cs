using System.Text.Json;
using System.Windows.Threading;
using DiskBurrow.App.Services;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Storage;
using DiskBurrow.Windows.Files;
using DiskBurrow.Windows.System;

namespace DiskBurrow.Tests;

public class ManualDeletionRuntimeTests
{
    [Fact] public async Task ActualRuntimePreviewsAndDeletesOwnedFolderAndWritesPathAuditToSqlite()
    {
        using var tree=new TempTree();var target=Path.Combine(tree.Root,"selected");Directory.CreateDirectory(Path.Combine(target,"nested"));var path=tree.FileAt("selected/nested/payload.bin",43);var sibling=tree.FileAt("keep.bin",17);var data=Path.Combine(tree.Root,"app-data");
        var snapshot=await new WindowsDiskScanner().ScanAsync(target,null,default);
        await Sta(async dispatcher=>
        {
            var scanner=new NeverScanner();await using var runtime=await AppRuntime.CreateAsync(data,dispatcher,new NoRegistry(),new LocalEnvironment(target),scanner);
            var model=runtime.Model;await model.ShowSnapshotAsync(snapshot);model.LargestFolders.Single(row=>row.Path==target).Selected=true;
            await model.AnalyzeManualAsync();var plan=model.Manual.Plan!;Assert.True(plan.CanExecute,string.Join("; ",plan.Warnings.Select(w=>w.ReasonKey)));Assert.Equal(new[]{target},plan.Roots);Assert.Equal(1,plan.FileCount);Assert.Equal(2,plan.DirectoryCount);Assert.Equal(43,plan.EstimatedDataBytes);
            Assert.True(File.Exists(path));await model.DeleteManualAsync(false);Assert.True(File.Exists(path));Assert.Null(model.Manual.Report);
            await model.DeleteManualAsync(true);Assert.False(Directory.Exists(target));Assert.True(File.Exists(sibling));Assert.All(model.Manual.Report!.Items,item=>Assert.Equal(CleanupOutcome.Deleted,item.Outcome));Assert.Null(model.Manual.JournalError);Assert.True(model.Manual.SnapshotStale);Assert.Equal(0,scanner.Invocations);
            using var db=new Microsoft.Data.Sqlite.SqliteConnection($"Data Source={runtime.History.DatabasePath};Pooling=False");db.Open();using var command=db.CreateCommand();command.CommandText="SELECT payload FROM cleanup_reports";
            var saved=JsonSerializer.Deserialize<CleanupReport>((byte[])command.ExecuteScalar()!)!;Assert.Contains(saved.Items,item=>item.Audit?.Path==path);Assert.Equal(3,saved.Items.Count);
            model.Manual.Select(data,true);await model.AnalyzeManualAsync();Assert.False(model.Manual.Plan!.CanExecute);Assert.Contains(model.Manual.Plan.Warnings,warning=>warning.ReasonKey=="Manual.ProtectedPath");
        });
        Assert.Equal(17,new FileInfo(sibling).Length);
    }
    private static Task Sta(Func<Dispatcher,Task> action)
    {
        var done=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var thread=new Thread(()=>
        {
            var dispatcher=Dispatcher.CurrentDispatcher;SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(dispatcher));
            dispatcher.BeginInvoke(async()=>{try{await action(dispatcher);done.TrySetResult();}catch(Exception error){done.TrySetException(error);}finally{dispatcher.BeginInvokeShutdown(DispatcherPriority.Background);}});
            Dispatcher.Run();
        }){IsBackground=true};thread.SetApartmentState(ApartmentState.STA);thread.Start();return done.Task.WaitAsync(TimeSpan.FromSeconds(40));
    }
    private sealed class NeverScanner : DiskBurrow.Core.Scanning.IDiskScanner
    {
        public int Invocations;public Task<DiskBurrow.Core.Scanning.ScanSnapshot> ScanAsync(string root,IProgress<DiskBurrow.Core.Scanning.ScanProgress>? progress,CancellationToken ct){Invocations++;throw new InvalidOperationException("Runtime must not scan implicitly.");}
    }
    private sealed class LocalEnvironment(string root) : IMonitoringEnvironment {public string SystemRoot=>root;public bool IsOnBattery=>false;public bool IsLocalRoot(string path)=>true;public long GetFreeBytes(string path)=>20;public VolumeSpace GetVolumeSpace(string path)=>new(40,20);}
    private sealed class NoRegistry : IRunRegistry {public string? Read(string name)=>null;public void Write(string name,string value)=>throw new InvalidOperationException("No real or fake autostart writes.");public void Delete(string name)=>throw new InvalidOperationException("No real or fake autostart writes.");}
}
