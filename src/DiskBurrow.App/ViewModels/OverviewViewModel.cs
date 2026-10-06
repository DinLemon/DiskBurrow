using DiskBurrow.App.Services;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.Monitoring;
namespace DiskBurrow.App.ViewModels;
public sealed class OverviewViewModel(IUiDispatcher dispatcher) : ObservableModel
{
    private long projectionVersion;
    public const int VisibleLimit = 1000;
    public ScanSnapshot? Snapshot { get; private set; }
    public VolumeSpace? Volume { get; private set; }
    public string? VisibleRoot {get;private set;}
    public DateTimeOffset? VolumeObservedUtc {get;private set;}
    public DirectoryObservation? FocusedDirectory {get;private set;}
    public IReadOnlyList<DirectoryObservation> Directories { get; private set; } = [];
    public IReadOnlyList<FileObservation> Files { get; private set; } = [];
    public IReadOnlyList<ScanIssueCount> IssueCounts { get; private set; }=[];
    public bool HasUnknownAllocatedBytes {get;private set;}
    public bool CoverageComplete {get;private set;}
    public long LogicalBytes {get;private set;}
    public long? AllocatedBytes {get;private set;}
    public double UsedPercent => Volume is {TotalBytes:>0} v ? 100.0*v.UsedBytes/v.TotalBytes : 0;
    public async Task ShowAsync(ScanSnapshot snapshot, VolumeSpace? volume = null,Func<bool>? canPublish=null,string? focusPath=null)
    {
        var version=Interlocked.Increment(ref projectionVersion);
        var rows = await Task.Run(()=>
        {
            var root=snapshot.Directories.FirstOrDefault(d=>string.Equals(d.Path,Path.TrimEndingDirectorySeparator(snapshot.Root),StringComparison.OrdinalIgnoreCase));
            var directories=snapshot.Directories.OrderByDescending(d=>d.LogicalBytes).Take(VisibleLimit).ToArray();
            var focus=snapshot.Directories.FirstOrDefault(d=>string.Equals(d.Path,focusPath,StringComparison.OrdinalIgnoreCase));
            if(focus is not null&&!directories.Contains(focus))directories=directories.Take(VisibleLimit-1).Prepend(focus).ToArray();
            return new Projection(directories,snapshot.LargestFiles.OrderByDescending(f=>f.LogicalBytes).Take(100).ToArray(),snapshot.Issues.GroupBy(i=>i.Kind).Select(g=>new ScanIssueCount(g.Key,g.LongCount())).ToArray(),root?.LogicalBytes??0,root?.AllocatedBytes,snapshot.Directories.Any(d=>d.AllocatedBytes is null),snapshot.TraversalCompleted&&snapshot.Directories.All(d=>d.CoverageComplete));
        });
        await dispatcher.InvokeAsync(()=>{if(version!=Interlocked.Read(ref projectionVersion)||canPublish?.Invoke()==false)return;if(!SameRoot(VisibleRoot,snapshot.Root)){Volume=null;VolumeObservedUtc=null;}Snapshot=snapshot;VisibleRoot=snapshot.Root;if(volume is not null){Volume=volume;VolumeObservedUtc=DateTimeOffset.UtcNow;}Directories=rows.Directories;FocusedDirectory=rows.Directories.FirstOrDefault(d=>string.Equals(d.Path,focusPath,StringComparison.OrdinalIgnoreCase));Files=rows.Files;IssueCounts=rows.Issues;LogicalBytes=rows.Logical;AllocatedBytes=rows.Allocated;HasUnknownAllocatedBytes=rows.Unknown;CoverageComplete=rows.Complete;Refresh();});
    }
    public Task ClearAsync(string root,Func<bool>? canPublish=null)
    {
        Interlocked.Increment(ref projectionVersion);
        return dispatcher.InvokeAsync(()=>{if(canPublish?.Invoke()==false)return;VisibleRoot=root;Snapshot=null;Volume=null;VolumeObservedUtc=null;FocusedDirectory=null;Directories=[];Files=[];IssueCounts=[];LogicalBytes=0;AllocatedBytes=null;HasUnknownAllocatedBytes=false;CoverageComplete=false;Refresh();});
    }
    public Task SetObservationAsync(VolumeObservation observation,Func<bool>? canPublish=null)=>dispatcher.InvokeAsync(()=>
    {
        if(canPublish?.Invoke()==false||!SameRoot(VisibleRoot,observation.Root)||VolumeObservedUtc>observation.ObservedUtc)return;
        Volume=observation.Space;VolumeObservedUtc=observation.ObservedUtc;Refresh();
    });
    private static bool SameRoot(string? left,string right)=>left is not null&&string.Equals(DiskBurrow.Core.History.SnapshotComparer.NormalizePath(left),DiskBurrow.Core.History.SnapshotComparer.NormalizePath(right),StringComparison.OrdinalIgnoreCase);
    public Task SetVolumeAsync(VolumeSpace volume)=>dispatcher.InvokeAsync(()=>{Volume=volume;Refresh();});
    public void RefreshLanguage(){Directories=Directories.ToArray();Files=Files.ToArray();Refresh();}
    private sealed record Projection(DirectoryObservation[] Directories,FileObservation[] Files,ScanIssueCount[] Issues,long Logical,long? Allocated,bool Unknown,bool Complete);
}
public sealed record ScanIssueCount(ScanIssueKind Kind,long Count);
