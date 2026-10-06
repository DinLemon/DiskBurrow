using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Scanning;
namespace DiskBurrow.Tests;
public sealed class DiskMapTests
{
 private static ScanSnapshot Snapshot()
 {var b=new ScanTreeBuilder(@"C:\fixture");var d=b.AddDirectory(0,"cache");b.AddFile(d,"unique.bin",10,16,default);b.AddFile(d,"other.bin",20,32,default);b.AddFile(0,"root.txt",5,8,default);return new(Guid.NewGuid(),@"C:\fixture",default,default,true,[],[],[]){Tree=b.Build()};}
 [Fact] public async Task Full_global_search_isolates_leaf_with_ancestors_and_folder_with_all_descendants()
 {
  var vm=new DiskMapViewModel(new InlineDispatcher(),(_,_)=>{},_=>false);vm.Show(Snapshot());await vm.WaitForSearchAsync();
  vm.Query="unique";vm.Isolate=true;await vm.WaitForSearchAsync();Assert.Single(vm.SearchResults);Assert.Single(vm.Layout(500,300),x=>x.Depth==0);Assert.Equal(1,vm.Layout(500,300).First().Index);
  vm.Query="cache";await vm.WaitForSearchAsync();Assert.Equal(2,vm.Layout(500,300).Count(x=>x.Depth==1));
  vm.Navigate(1);vm.GlobalSearch=false;vm.Query="root";await vm.WaitForSearchAsync();Assert.Empty(vm.SearchResults);
  vm.GlobalSearch=true;await vm.WaitForSearchAsync();Assert.Single(vm.SearchResults);vm.Navigate(vm.SearchResults[0].Index);Assert.Equal(4,vm.FocusedIndex);
 }
 [Fact] public async Task Navigation_history_zoom_and_marking_preserve_shared_selection()
 {
  var selected=new HashSet<string>(StringComparer.OrdinalIgnoreCase);var vm=new DiskMapViewModel(new InlineDispatcher(),(p,s)=>{if(s)selected.Add(p);else selected.Remove(p);},selected.Contains);
  vm.Show(Snapshot());await vm.WaitForSearchAsync();vm.Navigate(1);vm.Up();vm.Back();Assert.Equal(1,vm.CurrentIndex);vm.Forward();Assert.Equal(0,vm.CurrentIndex);
  vm.ChangeZoom(100);Assert.Equal(16,vm.Zoom);vm.ResetZoom();Assert.Equal(1,vm.Zoom);vm.ToggleMark(2);Assert.Contains(@"C:\fixture\cache\unique.bin",selected);vm.ToggleMark(2);Assert.Empty(selected);
 }
 [Fact] public async Task Historical_snapshot_explicitly_has_no_full_map_and_stale_search_cannot_publish()
 {
  var vm=new DiskMapViewModel(new InlineDispatcher(),(_,_)=>{},_=>false);vm.Show(Snapshot());vm.Query="unique";
  vm.Show(new(Guid.NewGuid(),@"C:\fixture",default,default,true,[],[],[]));await vm.WaitForSearchAsync();Assert.True(vm.IsHistorical);Assert.False(vm.HasTree);Assert.Empty(vm.SearchResults);Assert.Empty(vm.Layout(500,300));
 }
 [Fact] public async Task Empty_entries_remain_in_folder_list_and_unknown_allocation_is_explicit()
 {
  var b=new ScanTreeBuilder(@"C:\fixture");b.AddFile(0,"empty.bin",0,0,default);b.AddFile(0,"unknown.bin",500,null,default);b.MarkIncomplete(0);
  var vm=new DiskMapViewModel(new InlineDispatcher(),(_,_)=>{},_=>false);vm.Show(new(Guid.NewGuid(),@"C:\fixture",default,default,true,[],[],[]){Tree=b.Build()});await vm.WaitForSearchAsync();
  Assert.Equal(2,vm.SearchResults.Count);Assert.True(vm.HasNoVisibleData);Assert.Empty(vm.Layout(300,200));
  vm.Metric=DiskMapMetric.Logical;await vm.WaitForSearchAsync();Assert.False(vm.HasNoVisibleData);Assert.Single(vm.Layout(300,200));
 }
 [Fact] public async Task Parent_marks_absorb_children_and_busy_gate_rejects_map_mark_mutations()
 {
  var manual=new ManualDeletionViewModel(new NeverDelete(),null,new InlineDispatcher());bool busy=false;
  var vm=new DiskMapViewModel(new InlineDispatcher(),manual.Select,manual.IsSelected,()=>!busy);vm.Show(Snapshot());await vm.WaitForSearchAsync();
  vm.ToggleMark(2);vm.ToggleMark(1);Assert.Equal(new[]{@"C:\fixture\cache"},manual.SelectedPaths);Assert.True(vm.IsCoveredByMark(2));
  vm.ToggleMark(3);Assert.Single(manual.SelectedPaths);busy=true;vm.ToggleMark(1);Assert.Single(manual.SelectedPaths);busy=false;vm.ToggleMark(1);Assert.Empty(manual.SelectedPaths);
 }
 private sealed class NeverDelete:DiskBurrow.Core.Cleanup.IManualDeletionService
 {
  public Task<DiskBurrow.Core.Cleanup.ManualDeletePlan> PreviewAsync(IEnumerable<string> paths,CancellationToken ct)=>throw new InvalidOperationException();
  public Task<DiskBurrow.Core.Cleanup.CleanupReport> ExecuteConfirmedAsync(Guid id,bool confirmed,CancellationToken ct)=>throw new InvalidOperationException();
 }
}
