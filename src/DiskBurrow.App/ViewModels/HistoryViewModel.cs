using DiskBurrow.App.Services;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.History;
namespace DiskBurrow.App.ViewModels;
public sealed class HistoryViewModel(IHistoryStore store,IUiDispatcher dispatcher,Func<string?>? historyDiagnostic=null) : ObservableModel
{
    private long generation;
    public void InvalidateRequests()=>Interlocked.Increment(ref generation);
    public IReadOnlyList<ScanSnapshot> Snapshots {get;private set;}=[];
    public IReadOnlyList<FolderChange> Changes {get;private set;}=[];
    public string? ErrorDetails {get;private set;}
    public DateTimeOffset? ComparedUtc {get;private set;}
    public DateTimeOffset? PreviousUtc {get;private set;}
    public Guid? SelectedSnapshotId {get;private set;}
    public ScanSnapshot? SelectedSnapshot=>Snapshots.FirstOrDefault(s=>s.Id==SelectedSnapshotId);
    public bool ComparisonUnavailable {get;private set;}
    public async Task CompareSelectedAsync(ScanSnapshot current)
    {
        var request=Interlocked.Increment(ref generation);
        var previous=Snapshots.FirstOrDefault(s=>StringComparer.OrdinalIgnoreCase.Equals(s.Root,current.Root) && s.Id!=current.Id && s.CompletedUtc<current.CompletedUtc);
        var changes=await Task.Run(()=>previous is null?[]:new SnapshotComparer().Compare(previous,current).OrderByDescending(c=>Math.Abs(c.LogicalDeltaBytes)).Take(1000).ToArray());
        await dispatcher.InvokeAsync(()=>{if(request!=Interlocked.Read(ref generation))return;SelectedSnapshotId=current.Id;ComparisonUnavailable=false;Changes=changes;ComparedUtc=current.CompletedUtc;PreviousUtc=previous?.CompletedUtc;Refresh();});
    }
    public async Task LoadAsync(ScanSnapshot current,CancellationToken ct)=>await LoadRootAsync(current.Root,ct,current);
    public async Task<ScanSnapshot?> LoadRootAsync(string root,CancellationToken ct,ScanSnapshot? live=null,
        Guid? snapshotId=null,Guid? previousId=null,Func<bool>? canPublish=null)
    {
        var request=Interlocked.Increment(ref generation);
        try
        {
            var read=await Task.Run(async ()=>{var snapshots=await store.LoadRecentAsync(root,30,ct);return (Snapshots:snapshots,Diagnostic:historyDiagnostic?.Invoke());},ct);
            var snapshots=read.Snapshots;var diagnostic=read.Diagnostic;
            var current=live??(snapshotId is {} id?snapshots.FirstOrDefault(s=>s.Id==id):snapshots.FirstOrDefault());
            var previous=previousId is {} prior?snapshots.FirstOrDefault(s=>s.Id==prior):current is null?null:snapshots.FirstOrDefault(s=>s.Id!=current.Id && s.CompletedUtc<current.CompletedUtc);
            var changes=await Task.Run(()=>previous is null || current is null ? [] : new SnapshotComparer().Compare(previous,current).OrderByDescending(c=>Math.Abs(c.LogicalDeltaBytes)).Take(1000).ToArray(),ct);
            await dispatcher.InvokeAsync(()=>{if(request!=Interlocked.Read(ref generation)||canPublish?.Invoke()==false)return;Snapshots=snapshots;Changes=changes;ComparedUtc=current?.CompletedUtc;PreviousUtc=previous?.CompletedUtc;SelectedSnapshotId=current?.Id;ComparisonUnavailable=previousId is not null&&previous is null;ErrorDetails=diagnostic;Refresh();});
            return request==Interlocked.Read(ref generation)&&canPublish?.Invoke()!=false?current:null;
        }
        catch(OperationCanceledException) when(ct.IsCancellationRequested) { }
        catch(Exception error) { await dispatcher.InvokeAsync(()=>{if(request!=Interlocked.Read(ref generation)||canPublish?.Invoke()==false)return;Snapshots=[];Changes=[];ComparedUtc=null;PreviousUtc=null;SelectedSnapshotId=null;ComparisonUnavailable=false;ErrorDetails=error.Message;Refresh();}); }
        return null;
    }
}
