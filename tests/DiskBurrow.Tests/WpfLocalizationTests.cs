using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using DiskBurrow.App.Services;
using DiskBurrow.App.Views;

namespace DiskBurrow.Tests;

public class WpfLocalizationTests
{
    [Fact]
    public async Task RenderedTableHeadersChangeLanguageWithoutLosingRowsOrSelection()
    {
        var assembly=typeof(WpfLocalizationTests).Assembly.Location;
        var start=new ProcessStartInfo("dotnet"){RedirectStandardOutput=true,RedirectStandardError=true,UseShellExecute=false,CreateNoWindow=true};
        start.ArgumentList.Add(assembly);start.ArgumentList.Add("--verify-localized-views");
        using var child=Process.Start(start)!;
        var output=child.StandardOutput.ReadToEndAsync();var error=child.StandardError.ReadToEndAsync();
        try{await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(40));}
        catch{if(!child.HasExited)child.Kill();throw;}
        Assert.True(child.ExitCode==0,$"WPF child failed ({child.ExitCode}): {await output} {await error}");
        Assert.Contains("RU->EN->RU",await output);
    }
}

internal static class WpfProbeProgram
{
    [STAThread]
    private static int Main(string[] args)
    {
        if(args is ["--verify-background-snapshot",var page])
        {
            try
            {
                var app=new Application{ShutdownMode=ShutdownMode.OnExplicitShutdown};
                app.Resources.MergedDictionaries.Add(new(){Source=new Uri("/DiskBurrow;component/Resources/Styles.xaml",UriKind.Relative)});
                SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext());
                var task=BackgroundSnapshotTests.VerifyAsync(page);var frame=new DispatcherFrame();
                _=task.ContinueWith(_=>app.Dispatcher.BeginInvoke(()=>frame.Continue=false),TaskScheduler.Default);
                Dispatcher.PushFrame(frame);task.GetAwaiter().GetResult();app.Shutdown();return 0;
            }
            catch(Exception error){Console.Error.WriteLine(error);return 1;}
        }
        if(args is ["--verify-theme-views"]){try{ThemeVisualProbe.Run();return 0;}catch(Exception error){Console.Error.WriteLine(error);return 1;}}
        if(args is ["--verify-live-manual-report"])
        {
            try
            {
                var app=new Application{ShutdownMode=ShutdownMode.OnExplicitShutdown};
                app.Resources.MergedDictionaries.Add(new(){Source=new Uri("/DiskBurrow;component/Resources/Styles.xaml",UriKind.Relative)});
                app.Resources.MergedDictionaries.Add(new(){Source=new Uri("/DiskBurrow;component/Resources/Strings.en.xaml",UriKind.Relative)});
                SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext());
                var task=ManualDeletionCoordinationTests.VerifyLiveReportAsync();var frame=new DispatcherFrame();
                _=task.ContinueWith(_=>app.Dispatcher.BeginInvoke(()=>frame.Continue=false),TaskScheduler.Default);
                Dispatcher.PushFrame(frame);task.GetAwaiter().GetResult();app.Shutdown();return 0;
            }
            catch(Exception error){Console.Error.WriteLine(error);return 1;}
        }
        if(args is ["--verify-disk-map"]){try{DiskMapRenderProbe.Run();return 0;}catch(Exception error){Console.Error.WriteLine(error);return 1;}}
        if(args is not ["--verify-localized-views"])return 2;
        try{Run();return 0;}catch(Exception error){Console.Error.WriteLine(error);return 1;}
    }
    private static void Run()
    {

        var app=new Application{ShutdownMode=ShutdownMode.OnExplicitShutdown};
        app.Resources.MergedDictionaries.Add(new(){Source=new Uri("/DiskBurrow;component/Resources/Styles.xaml",UriKind.Relative)});
        // Swap the same merged locale dictionaries as LocalizationService.SetLanguage.
        // Component URIs allow the isolated worker to load the app's resources from a test entry assembly.
        ResourceDictionary? active=null;var currentLanguage="ru";
        void Switch(string language){currentLanguage=language;if(active is not null)app.Resources.MergedDictionaries.Remove(active);active=new(){Source=new Uri($"/DiskBurrow;component/Resources/Strings.{language}.xaml",UriKind.Relative)};app.Resources.MergedDictionaries.Add(active);}
        Switch("ru");
        var cases=new (UserControl View,string[][] Keys)[]{
            (new CleanupView(),[["Select","Path","Category","Logical","Modified","Reason"],["Path","Reason"],["Path","Outcome","Reason"]]),
            (new LargestView(),[["Select","Path","Logical","Allocated","Coverage"],["Select","Path","Logical","Allocated"]]),
            (new HistoryView(),[["Scan.Time","Root"],["Path","Delta","Kind"]]),
            (new OverviewView(),[["Reason","Issue.Count"],["Category","Path","Logical",""]])};
        foreach(var (view,keys) in cases)
        {
            var window=new Window{Content=view,DataContext=new ProbeModel(),Width=1160,Height=1000,Left=-10000,Top=-10000,ShowInTaskbar=false,ShowActivated=false};
            try
            {
                window.Show();foreach(var expander in LogicalDescendants<Expander>(view))expander.IsExpanded=true;Pump(window);
                var grids=Descendants<DataGrid>(view).ToArray();
                Require(grids.Length==keys.Length,$"Unexpected grid count in {view.GetType().Name}");
                var row=new ProbeRow();
                foreach(var grid in grids){grid.EnableColumnVirtualization=false;grid.ItemsSource=new[]{row};grid.SelectedItem=row;}
                foreach(var language in new[]{"ru","en","ru"})
                {
                    Switch(language);((ProbeModel)window.DataContext).Language=language;Pump(window);
                    for(var i=0;i<grids.Length;i++)
                    {
                        var grid=grids[i];var headers=Descendants<DataGridColumnHeader>(grid).Where(h=>h.Column is not null).ToDictionary(h=>h.Column!.DisplayIndex);
                        for(var c=0;c<keys[i].Length;c++)
                        {
                            if(keys[i][c]=="")continue;
                            Require(headers.TryGetValue(c,out var header),$"Missing rendered header {i}/{c}");
                            var text=header!.Content is TextBlock block?block.Text:header.Content?.ToString();
                            var expected=LocalizationService.ReadText(language,keys[i][c]);
                            Require(text==expected,$"{view.GetType().Name} grid {i} header {c}: {language} expected '{expected}', actual '{text}'");
                            Require(header.ActualHeight>0 && double.IsFinite(header.FontSize),"Header is not measured");
                        }
                        Require(ReferenceEquals(row,grid.SelectedItem)&&ReferenceEquals(row,grid.Items[0]),"Language change discarded row/selection");
                        if(view is LargestView)
                        {
                            var mark=Descendants<CheckBox>(grid).FirstOrDefault();
                            Require(mark is not null,"Largest row is missing its selection checkbox");
                            mark!.IsEnabled=true;mark.IsChecked=false;Require(!row.Selected,"Unmarking the visible checkbox did not update the selected path");
                            mark.IsChecked=true;Require(row.Selected,"Marking the visible checkbox did not update the selected path");
                        }
                    }
                    Capture(view,$"{view.GetType().Name}-{language}");
                }
                Console.WriteLine($"{view.GetType().Name}: RU->EN->RU rendered headers; rows/selection preserved");
            }
            finally{window.Close();}
        }
        var settings=new SettingsView();
        var host=new Window{Content=settings,DataContext=new ProbeModel(),Width=1160,Height=900,Left=-10000,Top=-10000,ShowInTaskbar=false,ShowActivated=false};
        try
        {
            host.Show();
            foreach(var language in new[]{"ru","en"})
            {
                Switch(language);((ProbeModel)host.DataContext).Language=language;Pump(host);
                var languageLabel=Descendants<TextBlock>(settings).Single(t=>t.Text==LocalizationService.ReadText(currentLanguage,"Settings.Language"));
                var languageInput=Descendants<ComboBox>(settings).Last();
                var label=languageLabel.TransformToAncestor(settings).TransformBounds(new Rect(languageLabel.RenderSize));
                var input=languageInput.TransformToAncestor(settings).TransformBounds(new Rect(languageInput.RenderSize));
                Require(input.Width>=150,"Settings input became too narrow");
                Require(Math.Abs(input.Left-label.Left)<1 && Math.Abs(input.Top-label.Bottom-8)<1,"Settings field is not aligned directly under its label with an 8px gap");
                Console.WriteLine($"Settings {language} measured labelWidth={label.Width:F1}, inputWidth={input.Width:F1}, horizontalOffset={input.Left-label.Left:F1}, verticalGap={input.Top-label.Bottom:F1}");
                Capture(settings,$"SettingsView-{language}");
                host.Width=640;host.Height=520;Pump(host);
                Require(settings.ActualWidth>=500 && languageInput.ActualWidth==200,"Compact settings do not fit the narrow content viewport");
                Capture(settings,$"SettingsView-narrow-{language}");
                host.Width=1160;host.Height=900;
            }
        }
        finally{host.Close();app.Shutdown();}
    }
    private static void Pump(Window window){window.UpdateLayout();window.Dispatcher.Invoke(()=>{},DispatcherPriority.ApplicationIdle);window.UpdateLayout();}
    private static void Capture(FrameworkElement view,string name)
    {
        var directory=Environment.GetEnvironmentVariable("DISKBURROW_UI_EVIDENCE");if(string.IsNullOrEmpty(directory))return;
        Directory.CreateDirectory(directory);
        var width=(int)Math.Ceiling(view.ActualWidth);var height=(int)Math.Ceiling(view.ActualHeight);
        var content=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);content.Render(view);
        var drawing=new DrawingVisual();using(var context=drawing.RenderOpen()){context.DrawRectangle(Window.GetWindow(view)?.Background??Brushes.White,null,new Rect(0,0,width,height));context.DrawImage(content,new Rect(0,0,width,height));}
        var bitmap=new RenderTargetBitmap(width,height,96,96,PixelFormats.Pbgra32);bitmap.Render(drawing);
        var encoder=new PngBitmapEncoder();encoder.Frames.Add(BitmapFrame.Create(bitmap));using var file=File.Create(Path.Combine(directory,name+".png"));encoder.Save(file);
    }
    private static IEnumerable<T> Descendants<T>(DependencyObject parent) where T:DependencyObject
    {for(var i=0;i<VisualTreeHelper.GetChildrenCount(parent);i++){var child=VisualTreeHelper.GetChild(parent,i);if(child is T value)yield return value;foreach(var descendant in Descendants<T>(child))yield return descendant;}}
    private static IEnumerable<T> LogicalDescendants<T>(DependencyObject parent) where T:DependencyObject
    {foreach(var item in LogicalTreeHelper.GetChildren(parent)){if(item is not DependencyObject child)continue;if(child is T value)yield return value;foreach(var descendant in LogicalDescendants<T>(child))yield return descendant;}}
    private sealed class ProbeModel:System.ComponentModel.INotifyPropertyChanged
    {
        private string language="ru";public bool Idle=>true;public string Language {get=>language;set{language=value;PropertyChanged?.Invoke(this,new(""));}}
        public event System.ComponentModel.PropertyChangedEventHandler? PropertyChanged;
        public ProbeSettings Settings {get;}=new();public string Filter {get;set;}="";public string Category {get;set;}="All";
        public object Cleanup {get;}=new {Categories=new[]{"All","UserTemp","CrashDumps","ChromeCache","EdgeCache"},CanExecuteCleanup=false,UnattemptedCount=0};
        public object[] CleanupCategorySummaries=>new[]{new {Category=LocalizationService.ReadText(Language,"UserTemp"),Size="1 · 0.01 GB",Status=LocalizationService.ReadText(Language,"Cleanup.Eligible"),Warnings=""},new {Category=LocalizationService.ReadText(Language,"CrashDumps"),Size="0 · 0.00 GB",Status=LocalizationService.ReadText(Language,"Cleanup.NoEligible"),Warnings=""},new {Category=LocalizationService.ReadText(Language,"ChromeCache"),Size="0 · 0.00 GB",Status=LocalizationService.ReadText(Language,"Cleanup.HasWarnings"),Warnings=LocalizationService.ReadText(Language,"Cleanup.OwnerRunning")},new {Category=LocalizationService.ReadText(Language,"EdgeCache"),Size="1 · 0.01 GB",Status=LocalizationService.ReadText(Language,"Cleanup.Eligible"),Warnings=""}};
    }
    private sealed class ProbeSettings {public int[] Intervals=>[1,6,12,24];public int IntervalHours {get;set;}=6;public int LowGB {get;set;}=15;public int GrowthGB {get;set;}=5;public string[] Languages=>["ru","en"];public bool AllowOnBattery {get;set;}public bool Autostart {get;set;}public string Exclusions {get;set;}="";public string CustomTemp {get;set;}="";}
    private sealed class ProbeRow {public string Path {get;}=@"E:\owned-fixture\row";public long Bytes=>12;public long LogicalBytes=>12;public long AllocatedBytes=>12;public bool CoverageComplete=>true;public bool Selected {get;set;}=true;public int Count=>1;public string Category=>"fixture";public string Reason=>"fixture";public DateTimeOffset Modified=>DateTimeOffset.UnixEpoch;public string Outcome=>"fixture";public string Root=>@"E:\owned-fixture";public DateTimeOffset CompletedUtc=>DateTimeOffset.UnixEpoch;public string Kind=>"AccessDenied";public string Program=>"fixture";}
    private static void Require(bool condition,string message){if(!condition)throw new InvalidOperationException(message);}
}
