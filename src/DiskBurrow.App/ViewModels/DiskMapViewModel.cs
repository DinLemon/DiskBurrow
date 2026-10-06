using DiskBurrow.App.Services;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.Visualization;
namespace DiskBurrow.App.ViewModels;

public enum DiskMapMetric { Allocated, Logical, Files }
public sealed record MapSearchResult(int Index,string Path,long LogicalBytes,bool IsDirectory) { public string Name=>System.IO.Path.GetFileName(Path); }
public sealed record MapBreadcrumb(int Index,string Name);
public sealed record DisplayMapTile(int Index,MapRect Bounds,int Depth,bool Matched);

public sealed class DiskMapViewModel(IUiDispatcher dispatcher, Action<string,bool> select, Func<string,bool> isSelected, Func<bool>? canMark=null) : ObservableModel
{
 private ScanTree? tree; private int[] offsets=[],children=[]; private bool[] matches=[],keep=[];
 private double[] filteredWeights=[]; private string query="";private bool isolate,global=true;private DiskMapMetric metric;
 private readonly List<int> history=[];
 private IReadOnlyList<DisplayMapTile>? layoutCache;private double layoutWidth,layoutHeight;private int layoutDepth;
 public string? Error {get;private set;}
 public void ReportError(string message){Error=message;Changed(nameof(Error));}
 private int historyPosition=-1;private long searchVersion;private Task searchTask=Task.CompletedTask;
 private readonly object searchGate=new();
 private readonly HashSet<Task> searches=[];
 private CancellationTokenSource? searchCancellation;
 private Task? searchDrain;
 public bool SearchBusy {get;private set;}
 public bool HasNoVisibleData=>tree is not null&&filteredWeights.Length==tree.Entries.Count&&filteredWeights[CurrentIndex]<=0&&!SearchBusy;
 public ScanTree? Tree=>tree;public int CurrentIndex {get;private set;}
 public int FocusedIndex {get;private set;}=-1;
 public bool IsHistorical {get;private set;} public int MatchCount {get;private set;}
 public string SearchHeadingKey=>query.Length==0?"Map.Items":"Map.Matches";
 public IReadOnlyList<MapSearchResult> SearchResults {get;private set;}=[];
 public IReadOnlyList<MapBreadcrumb> Breadcrumbs {get;private set;}=[];
 public string CurrentPath=>tree?.GetPath(CurrentIndex)??"";
 public string FocusedPath=>tree is not null&&FocusedIndex>=0?tree.GetPath(FocusedIndex):"";
 public bool CanBack=>historyPosition>0;public bool CanForward=>historyPosition<history.Count-1;
 public bool CanUp=>tree is not null&&CurrentIndex!=0;
 public bool HasTree=>tree is not null; public double Zoom {get;private set;}=1;
 public string Query {get=>query;set {if(query==value)return;query=value;QueueSearch();Changed();}}
 public bool Isolate {get=>isolate;set{if(isolate==value)return;isolate=value;QueueSearch();Changed();}}
 public bool GlobalSearch {get=>global;set{if(global==value)return;global=value;QueueSearch();Changed();}}
 public DiskMapMetric Metric {get=>metric;set{if(metric==value)return;metric=value;QueueSearch();Changed();}}
 public IReadOnlyList<DiskMapMetric> Metrics {get;}=Enum.GetValues<DiskMapMetric>();
 public event Action? MapChanged;
 public void Show(ScanSnapshot? snapshot)
 {
  lock(searchGate)searchCancellation?.Cancel();
  Interlocked.Increment(ref searchVersion); IsHistorical=snapshot is not null&&snapshot.Tree is null;
  // History deliberately retains only bounded directories/largest files; it cannot reconstruct a complete live map.
  tree=snapshot?.Tree;layoutCache=null;matches=[];keep=[];filteredWeights=[];Error=null;SearchBusy=false;history.Clear();historyPosition=-1;CurrentIndex=0;FocusedIndex=-1;Zoom=1;SearchResults=[];MatchCount=0;
  if(tree is not null){BuildChildren();Navigate(0);}else {offsets=[];children=[];matches=[];keep=[];filteredWeights=[];Breadcrumbs=[];Refresh();MapChanged?.Invoke();}
 }
 private void BuildChildren()
 {
  var count=tree!.Entries.Count;offsets=new int[count+1];children=new int[count-1];
  for(var i=1;i<count;i++)offsets[tree.Entries[i].ParentIndex+1]++;
  for(var i=1;i<offsets.Length;i++)offsets[i]+=offsets[i-1];var positions=(int[])offsets.Clone();
  for(var i=1;i<count;i++)children[positions[tree.Entries[i].ParentIndex]++]=i;
 }
 public void Navigate(int index,bool record=true)
 {
  if(tree is null||(uint)index>=(uint)tree.Entries.Count)return;
  if(!tree.Entries[index].IsDirectory){var parent=tree.Entries[index].ParentIndex;if(parent!=CurrentIndex)Navigate(parent,record);Focus(index);return;}
  CurrentIndex=index;FocusedIndex=index;Zoom=1;layoutCache=null;
  if(record){if(historyPosition+1<history.Count)history.RemoveRange(historyPosition+1,history.Count-historyPosition-1);if(history.Count==0||history[^1]!=index)history.Add(index);historyPosition=history.Count-1;}
  var crumbs=new List<MapBreadcrumb>();for(var i=index;i>=0;i=tree.Entries[i].ParentIndex)crumbs.Add(new(i,i==0?tree.Root:tree.Entries[i].Name));crumbs.Reverse();Breadcrumbs=crumbs;
  QueueSearch();Refresh();MapChanged?.Invoke();
 }
 public void Back(){if(CanBack)Navigate(history[--historyPosition],false);}
 public void Forward(){if(CanForward)Navigate(history[++historyPosition],false);}
 public void Up(){if(CanUp)Navigate(tree!.Entries[CurrentIndex].ParentIndex);}
 public void Root()=>Navigate(0);
 public void Focus(int index){if(tree is null||(uint)index>=(uint)tree.Entries.Count)return;FocusedIndex=index;Refresh();MapChanged?.Invoke();}
 public void MoveFocus(int delta)
 {
  if(tree is null)return;var start=offsets[CurrentIndex];var count=offsets[CurrentIndex+1]-start;
  if(count==0)return;var position=Array.IndexOf(children,FocusedIndex,start,count);
  var next=position<0?(delta<0?count-1:0):((position-start+delta)%count+count)%count;
  Focus(children[start+next]);
 }
 public void ChangeZoom(double factor){Zoom=Math.Clamp(Zoom*factor,1,16);Changed(nameof(Zoom));MapChanged?.Invoke();}
 public void ResetZoom(){Zoom=1;Changed(nameof(Zoom));MapChanged?.Invoke();}
 public string FocusedSelectionKey=>FocusedIndex<0?"Map.Unselected":IsMarked(FocusedIndex)?"Map.Selected":IsCoveredByMark(FocusedIndex)?"Map.Covered":"Map.Unselected";
 public bool IsCoveredByMark(int index){if(tree is null)return false;for(var parent=tree.Entries[index].ParentIndex;parent>=0;parent=tree.Entries[parent].ParentIndex)if(isSelected(tree.GetPath(parent)))return true;return false;}
 public bool IsMarked(int index)=>tree is not null&&isSelected(tree.GetPath(index));
 public void ToggleMark(int index){if(canMark?.Invoke()==false)return;if(tree is null||(uint)index>=(uint)tree.Entries.Count)return;var path=tree.GetPath(index);select(path,!isSelected(path));Refresh();MapChanged?.Invoke();}
 public void ToggleFocusedMark(){if(FocusedIndex>=0)ToggleMark(FocusedIndex);}
 public void RefreshSelection(){Refresh();MapChanged?.Invoke();}
 public Task WaitForSearchAsync()=>searchTask;
 public Task StopAsync()
 {
  lock(searchGate)
  {
   if(searchDrain is not null)return searchDrain;
   Interlocked.Increment(ref searchVersion);searchCancellation?.Cancel();
   // Include superseded workers and dispatcher publications, not only the newest projection.
   searchDrain=Task.WhenAll(searches.ToArray());return searchDrain;
  }
 }
 private async Task RetireSearchAsync(Task task,CancellationTokenSource cancellation)
 {
  try{await task.ConfigureAwait(false);}catch{ /* ComputeAndPublish reports ordinary errors; draining observes any remaining fault. */ }
  finally
  {
   lock(searchGate)
   {
    searches.Remove(task);
    if(ReferenceEquals(searchCancellation,cancellation))searchCancellation=null;
    cancellation.Dispose();
   }
  }
 }
 private void QueueSearch()
 {
  lock(searchGate)
  {
  if(searchDrain is not null)return;
  searchCancellation?.Cancel();
  var captured=tree;var version=Interlocked.Increment(ref searchVersion);var text=query;var scope=CurrentIndex;var all=global;var only=isolate;var measure=metric;
  if(captured is null){SearchBusy=false;return;}
  SearchBusy=true;Changed(nameof(SearchBusy));
  var cancellation=new CancellationTokenSource();searchCancellation=cancellation;
  searchTask=ComputeAndPublish(cancellation.Token);searches.Add(searchTask);
  _=RetireSearchAsync(searchTask,cancellation);
  async Task ComputeAndPublish(CancellationToken ct)
  {
   try
   {
   await Task.Delay(100,ct);
   if(version!=Interlocked.Read(ref searchVersion))return;
   var projection=await Task.Run(()=>
   {
    var count=captured.Entries.Count;var found=new bool[count];var retained=new bool[count];var inherited=new bool[count];var values=new double[count];var rows=new List<MapSearchResult>();var total=0;var largestChildren=new PriorityQueue<int,(double,int)>();
    for(var i=0;i<count;i++)
    {
     ct.ThrowIfCancellationRequested();
     if(Interlocked.Read(ref searchVersion)!=version)return ((bool[],bool[],double[],MapSearchResult[],int)?)null;
     var e=captured.Entries[i];var parent=e.ParentIndex;
     var inScope=all||i==scope||(parent>=0&&inherited[parent]);inherited[i]=inScope;
     found[i]=text.Length>0&&inScope&&e.Name.Contains(text,StringComparison.OrdinalIgnoreCase);
     if(found[i]){total++;if(rows.Count<1000)rows.Add(new(i,captured.GetPath(i),e.LogicalBytes,e.IsDirectory));}
     if(text.Length==0&&e.ParentIndex==scope){total++;var weight=measure switch{DiskMapMetric.Logical=>(double)e.LogicalBytes,DiskMapMetric.Files=>e.FileCount,_=>e.AllocatedBytes??0};largestChildren.Enqueue(i,(weight,-i));if(largestChildren.Count>1000)largestChildren.Dequeue();}
     retained[i]=text.Length==0||!only||found[i];
    }
    if(text.Length==0)rows=largestChildren.UnorderedItems.OrderByDescending(x=>x.Priority.Item1).ThenBy(x=>x.Element).Select(x=>{ct.ThrowIfCancellationRequested();var e=captured.Entries[x.Element];return new MapSearchResult(x.Element,captured.GetPath(x.Element),e.LogicalBytes,e.IsDirectory);}).ToList();
    // Matching a folder keeps its full subtree; matching a leaf keeps every ancestor.
    if(only&&text.Length>0){
     var whole=new bool[count];for(var i=0;i<count;i++){ct.ThrowIfCancellationRequested();var p=captured.Entries[i].ParentIndex;whole[i]=(p>=0&&whole[p])||(found[i]&&captured.Entries[i].IsDirectory);if(whole[i])retained[i]=true;}
     for(var i=count-1;i>0;i--){ct.ThrowIfCancellationRequested();if(retained[i])retained[captured.Entries[i].ParentIndex]=true;}
    }
    for(var i=count-1;i>=0;i--){ct.ThrowIfCancellationRequested();var e=captured.Entries[i];if(!e.IsDirectory&&retained[i])values[i]=measure switch{DiskMapMetric.Logical=>e.LogicalBytes,DiskMapMetric.Files=>1,_=>e.AllocatedBytes??0};if(e.ParentIndex>=0)values[e.ParentIndex]+=values[i];}
    return ((bool[],bool[],double[],MapSearchResult[],int)?)(found,retained,values,rows.ToArray(),total);
   },ct);
   if(projection is not {} p)return;
   ct.ThrowIfCancellationRequested();
   await dispatcher.InvokeAsync(()=>{if(version!=Interlocked.Read(ref searchVersion)||!ReferenceEquals(tree,captured))return;matches=p.Item1;keep=p.Item2;filteredWeights=p.Item3;layoutCache=null;SearchResults=p.Item4;MatchCount=p.Item5;SearchBusy=false;Refresh();MapChanged?.Invoke();});
   }
   catch(OperationCanceledException) when(ct.IsCancellationRequested){ }
   catch(Exception error)
   {
    await dispatcher.InvokeAsync(()=>{if(version!=Interlocked.Read(ref searchVersion))return;SearchBusy=false;Error=error.Message;Refresh();MapChanged?.Invoke();});
   }
  }
  }
 }
 public IReadOnlyList<DisplayMapTile> Layout(double width,double height,int depth=2)
 {
  if(tree is null||filteredWeights.Length!=tree.Entries.Count||width<=0||height<=0)return [];
  if(layoutCache is not null&&layoutWidth==width&&layoutHeight==height&&layoutDepth==depth)return layoutCache;
  var result=new List<DisplayMapTile>();Fill(CurrentIndex,new(0,0,width,height),0);layoutWidth=width;layoutHeight=height;layoutDepth=depth;layoutCache=result;return result;
  void Fill(int parent,MapRect bounds,int level)
  {
   // Bound drawing and sorting cost, while retaining the entire tail's area.
   var top=new PriorityQueue<MapWeight,(double,int)>();double total=0;
   for(var j=offsets[parent];j<offsets[parent+1];j++)
   {var i=children[j];if(!keep[i]||filteredWeights[i]<=0)continue;var item=new MapWeight(i,filteredWeights[i]);total+=item.Weight;top.Enqueue(item,(item.Weight,-i));if(top.Count>511)top.Dequeue();}
   var weights=top.UnorderedItems.Select(x=>x.Element).ToList();var tail=total-weights.Sum(x=>x.Weight);
   if(tail>Math.Max(1,total)*1e-12)weights.Add(new(-1,tail));
   foreach(var tile in SquarifiedTreemap.Layout(weights,bounds))
   {
    result.Add(new(tile.Index,tile.Bounds,level,tile.Index>=0&&matches[tile.Index]));
    if(tile.Index>=0&&tree.Entries[tile.Index].IsDirectory&&level+1<depth&&tile.Bounds.Width>65&&tile.Bounds.Height>55)
     Fill(tile.Index,new(tile.Bounds.X+3,tile.Bounds.Y+22,Math.Max(0,tile.Bounds.Width-6),Math.Max(0,tile.Bounds.Height-25)),level+1);
   }
  }
 }
}
