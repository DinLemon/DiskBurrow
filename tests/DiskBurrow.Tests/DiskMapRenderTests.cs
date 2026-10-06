using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using DiskBurrow.App.Services;
using DiskBurrow.App.Views;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Windows.Files;
using DiskBurrow.Windows.System;
namespace DiskBurrow.Tests;
public sealed class DiskMapRenderTests
{
 [Fact] public async Task Real_wpf_map_renders_both_languages_and_keys_mark_same_manual_selection()
 {
  var start=new ProcessStartInfo("dotnet"){RedirectStandardOutput=true,RedirectStandardError=true,UseShellExecute=false,CreateNoWindow=true};start.ArgumentList.Add(typeof(DiskMapRenderTests).Assembly.Location);start.ArgumentList.Add("--verify-disk-map");
  using var child=Process.Start(start)!;var output=child.StandardOutput.ReadToEndAsync();var error=child.StandardError.ReadToEndAsync();
  try{await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(50));}catch{if(!child.HasExited)child.Kill();throw;}
  Assert.True(child.ExitCode==0,$"Map WPF child failed: {await output} {await error}");Assert.Contains("Disk map: RU/EN rendered",await output);
 }
}
internal static class DiskMapRenderProbe
{
 public static void Run()
 {
  var app=new Application{ShutdownMode=ShutdownMode.OnExplicitShutdown};app.Resources.MergedDictionaries.Add(new(){Source=new Uri("/DiskBurrow;component/Resources/Styles.xaml",UriKind.Relative)});
  Exception? failure=null;
  app.Dispatcher.BeginInvoke(async()=>{try{await Render(app);}catch(Exception error){failure=error;}finally{app.Shutdown();}});
  app.Run();if(failure is not null)throw failure;
 }
 private static async Task Render(Application app)
 {
  using var fixture=new TempTree();fixture.FileAt("media/movie.mp4",1024);fixture.FileAt("images/photo.png",512);fixture.FileAt("archives/source.zip",256);fixture.FileAt("documents/notes.txt",128);fixture.FileAt("disk/system.vhdx",800);fixture.FileAt("code/library.dll",200);fixture.FileAt("other.bin",80);
  var snapshot=await new WindowsDiskScanner().ScanAsync(fixture.Root,null,default);
  await using var runtime=await AppRuntime.CreateAsync(Path.Combine(fixture.Root,"app-data"),app.Dispatcher,new Registry(),new LocalEnvironment(fixture.Root),new NeverScanner());
  await runtime.History.SaveAsync(snapshot,default);var main=runtime.Model;await main.ShowSnapshotAsync(snapshot);await main.Map.WaitForSearchAsync();
  var window=new DiskBurrow.App.MainWindow(main){Left=-10000,Top=-10000,ShowInTaskbar=false,ShowActivated=false};window.Show();var tabs=Find<TabControl>(window).Single(t=>t.Name=="Pages");tabs.SelectedIndex=2;await Pump(window);
  var canvas=Find<DiskMapControl>(window).Single();Require(canvas.ActualWidth>300&&canvas.ActualHeight>120,"Map canvas has no usable area");
  foreach(var modifiers in new[]{ModifierKeys.None,ModifierKeys.Shift,ModifierKeys.Control,ModifierKeys.Control|ModifierKeys.Shift})Require(!canvas.HandleKey(Key.Tab,modifiers),"Map consumed WPF Tab traversal");
  Require(canvas.Focus(),"Map canvas could not receive keyboard focus");
  Require(canvas.MoveFocus(new TraversalRequest(FocusNavigationDirection.Next)),"WPF could not traverse forward out of map");
  Require(!ReferenceEquals(Keyboard.FocusedElement,canvas),"Forward traversal remained on map canvas");
  canvas.Focus();Require(canvas.MoveFocus(new TraversalRequest(FocusNavigationDirection.Previous)),"WPF could not traverse backward out of map");
  Require(!ReferenceEquals(Keyboard.FocusedElement,canvas),"Backward traversal remained on map canvas");
  var directory=Enumerable.Range(1,snapshot.Tree!.Entries.Count-1).First(i=>snapshot.Tree.Entries[i].IsDirectory);
  main.Map.Focus(directory);Require(canvas.HandleKey(Key.Space),"Space was not handled");Require(main.Manual.SelectedPaths.Contains(snapshot.Tree.GetPath(directory)),"Keyboard map mark did not reach Manual selection");
  await main.AnalyzeManualAsync();Require(main.Manual.Plan?.Roots.SequenceEqual(new[]{snapshot.Tree.GetPath(directory)})==true,"Map marks were not the exact preview roots");Require(main.CanDeleteManual,"Owned map-selected folder did not produce an executable review");
  main.Map.ToggleMark(directory);Require(main.Manual.Plan is null,"Changing a map mark did not invalidate the reviewed deletion plan");main.Map.ToggleMark(directory);
  Require(canvas.HandleKey(Key.Enter),"Enter was not handled");Require(main.Map.CurrentIndex==directory,"Enter did not navigate");canvas.HandleKey(Key.Back);Require(main.Map.CurrentIndex==0,"Backspace did not navigate up");canvas.HandleKey(Key.OemPlus);Require(main.Map.Zoom>1,"+ did not zoom");canvas.HandleKey(Key.D0);Require(main.Map.Zoom==1,"0 did not reset zoom");
  main.Map.Query="movie";await main.Map.WaitForSearchAsync();Require(main.Map.MatchCount==1,"Full live search did not find file");main.Map.Isolate=true;await main.Map.WaitForSearchAsync();await Pump(window);Capture(window,"DiskMap-ru-isolate");main.Map.Isolate=false;main.Map.Query="";await main.Map.WaitForSearchAsync();
  foreach(var lang in new[]{"ru","en"})
  {
   main.Language=lang;await Pump(window);Require(Find<TextBlock>(window).Any(t=>t.Text==LocalizationService.ReadText(lang,"Nav.Map")),"Map title was not localized");
   Require(canvas.LanguageCode==lang,"Map tooltip language did not switch");Capture(window,$"DiskMap-{lang}");
  }
  window.Width=880;window.Height=600;await Pump(window);Capture(window,"DiskMap-en-minimum");Require(canvas.ActualHeight>=100,"Minimum window loses map entirely");var canvasBounds=canvas.TransformToAncestor(window).TransformBounds(new Rect(canvas.RenderSize));Require(canvasBounds.Top<window.ActualHeight-100,"Minimum window places map outside viewport");
  main.Manual.InvalidateContext();var fileIndex=Enumerable.Range(1,snapshot.Tree.Entries.Count-1).First(i=>!snapshot.Tree.Entries[i].IsDirectory);main.Map.ToggleMark(fileIndex);Require(main.Manual.SelectedPaths.Count==1,"Map file mark was not retained");
  await main.LoadSelectedRootAsync();Require(main.Manual.SelectedPaths.Count==0,"Root reload did not invalidate map marks");Require(main.Map.IsHistorical,"Persisted bounded snapshot is not explicitly historical");
  window.AllowClose=true;window.Close();Console.WriteLine("Disk map: RU/EN rendered; keyboard navigation/zoom/shared marks; full search; historical invalidation");
 }
 private static async Task Pump(Window window){window.UpdateLayout();await window.Dispatcher.InvokeAsync(()=>{},DispatcherPriority.ApplicationIdle);window.UpdateLayout();}
 private static IEnumerable<T> Find<T>(DependencyObject node)where T:DependencyObject{if(node is T own)yield return own;for(var i=0;i<VisualTreeHelper.GetChildrenCount(node);i++)foreach(var item in Find<T>(VisualTreeHelper.GetChild(node,i)))yield return item;}
 private static void Require(bool condition,string message){if(!condition)throw new InvalidOperationException(message);}
 private static void Capture(Window window,string name)
 {
  var repo=Locate();var directory=Path.Combine(repo,"work","map-evidence");Directory.CreateDirectory(directory);var bitmap=new RenderTargetBitmap((int)window.ActualWidth,(int)window.ActualHeight,96,96,PixelFormats.Pbgra32);bitmap.Render(window);var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var stream=File.Create(Path.Combine(directory,name+".png"));encoder.Save(stream);
 }
 private static string Locate(){for(var dir=new DirectoryInfo(AppContext.BaseDirectory);dir is not null;dir=dir.Parent)if(File.Exists(Path.Combine(dir.FullName,"DiskBurrow.slnx")))return dir.FullName;throw new InvalidOperationException("Owned repository fixture root missing.");}
 private sealed class LocalEnvironment(string root):IMonitoringEnvironment{public string SystemRoot=>root;public bool IsOnBattery=>false;public bool IsLocalRoot(string path)=>true;public long GetFreeBytes(string path)=>50;public VolumeSpace GetVolumeSpace(string path)=>new(100,50);}
 private sealed class Registry:IRunRegistry{public string? Read(string name)=>null;public void Write(string name,string value)=>throw new InvalidOperationException();public void Delete(string name)=>throw new InvalidOperationException();}
 private sealed class NeverScanner:IDiskScanner{public Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? progress,CancellationToken ct)=>throw new InvalidOperationException("No implicit scan allowed.");}
}
