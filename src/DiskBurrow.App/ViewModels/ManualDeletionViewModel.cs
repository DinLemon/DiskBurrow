using DiskBurrow.App.Services;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;

namespace DiskBurrow.App.ViewModels;

// Manual selection is deliberately independent of automatic cleanup's category/file marks.
public sealed class ManualDeletionViewModel(IManualDeletionService service,IHistoryStore? history,
    IUiDispatcher dispatcher,Func<string?>? historyDiagnostic=null) : ObservableModel
{
    private readonly HashSet<string> selected=new(StringComparer.OrdinalIgnoreCase);
    private readonly SemaphoreSlim gate=new(1,1);
    private long generation;
    private bool executing;
    public IReadOnlyList<string> SelectedPaths=>selected.Order(StringComparer.OrdinalIgnoreCase).ToArray();
    public ManualDeletePlan? Plan {get;private set;}
    public CleanupReport? Report {get;private set;}
    public string? JournalError {get;private set;}
    public bool Busy {get;private set;}
    public bool SnapshotStale {get;private set;}
    public bool CanAnalyze=>!Busy&&selected.Count>0;
    public bool CanExecute=>!Busy&&Plan?.CanExecute==true;
    public bool IsSelected(string path)=>selected.Contains(SnapshotComparer.NormalizePath(path));
    public void Select(string path,bool value)
    {
        if(executing)return;
        path=SnapshotComparer.NormalizePath(path);
        if(value&&selected.Any(parent=>IsWithin(path,parent)))return;
        var absorbed=value?selected.RemoveWhere(child=>IsWithin(child,path)):0;
        var changed=(value?selected.Add(path):selected.Remove(path))||absorbed>0;
        if(!changed)return;
        Interlocked.Increment(ref generation);Plan=null;Report=null;JournalError=null;Refresh();
    }
    private static bool IsWithin(string path,string root)
    {
        var prefix=Path.EndsInDirectorySeparator(root)?root:root+Path.DirectorySeparatorChar;
        return string.Equals(path,root,StringComparison.OrdinalIgnoreCase)||path.StartsWith(prefix,StringComparison.OrdinalIgnoreCase);
    }
    public void InvalidateContext()
    {
        Interlocked.Increment(ref generation);selected.Clear();Plan=null;SnapshotStale=false;
        if(!executing){Report=null;JournalError=null;}
        Refresh();
    }
    public async Task AnalyzeAsync(CancellationToken ct)
    {
        if(!CanAnalyze||!await gate.WaitAsync(0,ct))return;
        var request=Interlocked.Read(ref generation);var paths=SelectedPaths;
        try
        {
            await dispatcher.InvokeAsync(()=>{Busy=true;Plan=null;Report=null;JournalError=null;Refresh();});
            var plan=await service.PreviewAsync(paths,ct);
            await dispatcher.InvokeAsync(()=>{if(request==Interlocked.Read(ref generation)){Plan=plan;Refresh();}});
        }
        finally {await dispatcher.InvokeAsync(()=>{Busy=false;Refresh();});gate.Release();}
    }
    public async Task ExecuteAsync(bool permanentDeletionConfirmed,CancellationToken ct)
    {
        if(!permanentDeletionConfirmed||!CanExecute||!await gate.WaitAsync(0,ct))return;
        var plan=Plan!;
        try
        {
            // Consume the UI plan before the first await. Service additionally enforces one-shot identity.
            executing=true;Plan=null;Busy=true;Refresh();
            var report=await service.ExecuteConfirmedAsync(plan.Id,true,ct);
            // A partial/cancelled report is published before journaling and can never be replayed.
            await dispatcher.InvokeAsync(()=>{Report=report;selected.Clear();SnapshotStale=true;Refresh();});
            if(history is not null)
            {
                try
                {
                    var diagnostic=await Task.Run(async()=>{await history.AppendCleanupAsync(report,CancellationToken.None);return historyDiagnostic?.Invoke();});
                    if(diagnostic is not null)await dispatcher.InvokeAsync(()=>{JournalError=diagnostic;Refresh();});
                }
                catch(Exception error){await dispatcher.InvokeAsync(()=>{JournalError=error.Message;Refresh();});}
            }
        }
        finally
        {
            await dispatcher.InvokeAsync(()=>{selected.Clear();SnapshotStale=true;Busy=false;executing=false;Refresh();});gate.Release();
        }
    }
}
