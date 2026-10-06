using System.Threading.Channels;
using System.Windows.Input;
using DiskBurrow.App.Services;
using DiskBurrow.App.ViewModels;
using DiskBurrow.App.Views;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;
namespace DiskBurrow.Tests;
public sealed class MapReviewRegressionTests
{
 private static ScanSnapshot Snapshot(string root=@"C:\fixture")
 {var b=new ScanTreeBuilder(root);for(var i=0;i<3;i++)b.AddFile(0,$"f{i}.bin",10,10,default);return new(Guid.NewGuid(),root,default,default,true,[],[],[]){Tree=b.Build()};}
 [Fact] public async Task Map_stop_drains_superseded_dispatcher_publication_and_rejects_later_search()
 {
  var ui=new HeldDispatcher();var vm=new DiskMapViewModel(ui,(_,_)=>{},_=>false);vm.Show(Snapshot());var first=vm.WaitForSearchAsync();var held=await ui.Pending.Reader.ReadAsync().AsTask().WaitAsync(TimeSpan.FromSeconds(3));
  vm.Query="f1";var newest=vm.WaitForSearchAsync();var stopping=vm.StopAsync();
  try{await newest.WaitAsync(TimeSpan.FromSeconds(3));Assert.False(stopping.IsCompleted);Assert.False(first.IsCompleted);Assert.Empty(vm.SearchResults);}
  finally{held.Apply();await Task.WhenAll(first,newest,stopping).WaitAsync(TimeSpan.FromSeconds(3));}
  Assert.True(first.IsCompleted);Assert.Same(stopping,vm.StopAsync());vm.Query="f2";Assert.Same(newest,vm.WaitForSearchAsync());Assert.Empty(vm.SearchResults);
 }
 [Fact] public async Task Main_stop_also_waits_for_superseded_map_search_publication()
 {
  using var owned=new TempTree();var ui=new HeldDispatcher();var history=new History();var env=new EnvironmentFixture();var settings=new AppSettings();
  await using var coordinator=new MonitoringCoordinator(settings,new NeverScanner(),history,env,new SettingsStore(owned.Root));
  var model=new MainViewModel(coordinator,env,ui,new(),new(ui),new(history,ui),new(new Planner(),new Executor(),history,ui),new(settings,new SettingsStore(owned.Root),new AutostartRegistration(new Registry(),@"C:\fixture\DiskBurrow.exe"),_=>null,_=>{}),settings);
  model.Map.Show(Snapshot());var first=model.Map.WaitForSearchAsync();var held=await ui.Pending.Reader.ReadAsync().AsTask().WaitAsync(TimeSpan.FromSeconds(3));model.Map.Query="f1";var newest=model.Map.WaitForSearchAsync();var stopping=model.StopAsync();
  try{await newest.WaitAsync(TimeSpan.FromSeconds(3));Assert.False(stopping.IsCompleted);Assert.False(first.IsCompleted);}
  finally{held.Apply();await stopping.WaitAsync(TimeSpan.FromSeconds(3));}
  Assert.True(first.IsCompleted);Assert.Empty(model.Map.SearchResults);
 }
 [Theory] [InlineData(false)] [InlineData(true)]
 public async Task Volume_root_absorbs_children_in_both_orders_and_map_cover_matches_selection(bool parentFirst)
 {
  var manual=new ManualDeletionViewModel(new NeverDelete(),null,new InlineDispatcher());var vm=new DiskMapViewModel(new InlineDispatcher(),manual.Select,manual.IsSelected);vm.Show(Snapshot(@"C:\"));await vm.WaitForSearchAsync();
  if(parentFirst){vm.ToggleMark(0);vm.ToggleMark(1);}else{vm.ToggleMark(1);vm.ToggleMark(0);}
  Assert.Equal(new[]{@"C:\"},manual.SelectedPaths);Assert.True(vm.IsMarked(0));Assert.True(vm.IsCoveredByMark(1));Assert.False(vm.IsMarked(1));
  manual.InvalidateContext();manual.Select(@"C:\child",true);manual.Select(@"C:\childish\f.bin",true);Assert.Equal(2,manual.SelectedPaths.Count);
 }
 [Fact] public async Task Reverse_first_focus_uses_last_child_and_both_directions_wrap()
 {
  var vm=new DiskMapViewModel(new InlineDispatcher(),(_,_)=>{},_=>false);vm.Show(Snapshot());await vm.WaitForSearchAsync();vm.MoveFocus(-1);Assert.Equal(3,vm.FocusedIndex);vm.MoveFocus(1);Assert.Equal(1,vm.FocusedIndex);vm.MoveFocus(-1);Assert.Equal(3,vm.FocusedIndex);
  vm.Root();vm.MoveFocus(1);Assert.Equal(1,vm.FocusedIndex);await vm.StopAsync();
 }
 [Fact] public async Task Tab_variants_are_not_consumed_by_map_keyboard_handler()
 {
  await Sta(()=>{var vm=new DiskMapViewModel(new InlineDispatcher(),(_,_)=>{},_=>false);vm.Show(Snapshot());var canvas=new DiskMapControl{Model=vm};foreach(var modifiers in new[]{ModifierKeys.None,ModifierKeys.Shift,ModifierKeys.Control,ModifierKeys.Control|ModifierKeys.Shift}){Assert.False(canvas.HandleKey(Key.Tab,modifiers));Assert.Equal(0,vm.FocusedIndex);}vm.StopAsync().GetAwaiter().GetResult();});
 }
 private static Task Sta(Action action){var done=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);var thread=new Thread(()=>{try{action();done.SetResult();}catch(Exception error){done.SetException(error);}}){IsBackground=true};thread.SetApartmentState(ApartmentState.STA);thread.Start();return done.Task.WaitAsync(TimeSpan.FromSeconds(5));}
 private sealed class HeldDispatcher:IUiDispatcher {public Channel<Invocation> Pending=Channel.CreateUnbounded<Invocation>();public Task InvokeAsync(Action action){var invocation=new Invocation(action);Pending.Writer.TryWrite(invocation);return invocation.Done.Task;}}
 private sealed class Invocation(Action action){public TaskCompletionSource Done=new(TaskCreationOptions.RunContinuationsAsynchronously);public void Apply(){action();Done.TrySetResult();}}
 private sealed class NeverDelete:IManualDeletionService{public Task<ManualDeletePlan> PreviewAsync(IEnumerable<string> paths,CancellationToken ct)=>throw new InvalidOperationException();public Task<CleanupReport> ExecuteConfirmedAsync(Guid id,bool confirmed,CancellationToken ct)=>throw new InvalidOperationException();}
 private sealed class History:IHistoryStore{public Task<HistoryWriteResult> SaveAsync(ScanSnapshot s,CancellationToken ct)=>Task.FromResult(new HistoryWriteResult(true,null));public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>Task.FromResult<IReadOnlyList<ScanSnapshot>>([]);public Task AppendCleanupAsync(CleanupReport report,CancellationToken ct)=>Task.CompletedTask;}
 private sealed class EnvironmentFixture:IMonitoringEnvironment{public string SystemRoot=>@"C:\fixture";public bool IsOnBattery=>false;public bool IsLocalRoot(string root)=>true;public long GetFreeBytes(string root)=>10;public VolumeSpace GetVolumeSpace(string root)=>new(20,10);}
 private sealed class NeverScanner:IDiskScanner{public Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? p,CancellationToken ct)=>throw new InvalidOperationException();}
 private sealed class Planner:ICleanupPlanner{public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> p,CancellationToken ct)=>throw new InvalidOperationException();}
 private sealed class Executor:ICleanupExecutor{public Task<CleanupReport> ExecuteAsync(CleanupPlan p,IReadOnlySet<Guid> ids,CancellationToken ct)=>throw new InvalidOperationException();}
 private sealed class Registry:IRunRegistry{public string? Read(string n)=>null;public void Write(string n,string v)=>throw new InvalidOperationException();public void Delete(string n)=>throw new InvalidOperationException();}
}
