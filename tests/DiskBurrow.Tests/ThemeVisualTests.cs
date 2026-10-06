using System.Diagnostics;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using DiskBurrow.App.Services;
using DiskBurrow.App.Views;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Windows.System;

namespace DiskBurrow.Tests;

public sealed class ThemeVisualTests
{
    [Fact]
    public async Task ThemeChangesDerivedWindowsControlsAndPopupWithoutRecreatingThem()
    {
        var start = new ProcessStartInfo("dotnet") { RedirectStandardOutput = true, RedirectStandardError = true, UseShellExecute = false, CreateNoWindow = true };
        start.ArgumentList.Add(typeof(ThemeVisualTests).Assembly.Location);
        start.ArgumentList.Add("--verify-theme-views");
        using var child = Process.Start(start)!;
        var output = child.StandardOutput.ReadToEndAsync();
        var error = child.StandardError.ReadToEndAsync();
        try { await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(50)); }
        catch { if (!child.HasExited) child.Kill(); throw; }
        Assert.True(child.ExitCode == 0, $"Theme WPF failed: {await output} {await error}");
        Assert.Contains("48 theme/language/size views", await output);
    }
}

internal static class ThemeVisualProbe
{
    internal static void Run()
    {
        var app = new Application { ShutdownMode = ShutdownMode.OnExplicitShutdown };
        app.Resources.MergedDictionaries.Add(new() { Source = new Uri("/DiskBurrow;component/Resources/Styles.xaml", UriKind.Relative) });
        SynchronizationContext.SetSynchronizationContext(new DispatcherSynchronizationContext(app.Dispatcher));
        var task = VerifyAsync(app);
        var frame = new DispatcherFrame();
        _ = task.ContinueWith(_ => app.Dispatcher.BeginInvoke(() => frame.Continue = false), TaskScheduler.Default);
        Dispatcher.PushFrame(frame);
        task.GetAwaiter().GetResult();
        app.Shutdown();
    }

    private static async Task VerifyAsync(Application app)
    {
        using var tree = new TempTree();
        await using var runtime = await AppRuntime.CreateAsync(Path.Combine(tree.Root, "data"), app.Dispatcher, new NoRegistry());
        var window = new DiskBurrow.App.MainWindow(runtime.Model) { Left = -10000, Top = -10000, ShowActivated = false, ShowInTaskbar = false };
        var evidence = Environment.GetEnvironmentVariable("DISKBURROW_UI_EVIDENCE");
        try
        {
            window.Show();
            var tabs = Find<TabControl>(window).Single(t => t.Name == "Pages");
            foreach (var theme in new[] { "light", "dark" })
            foreach (var language in new[] { "ru", "en" })
            foreach (var size in new[] { (880d, 600d), (1280d, 800d) })
            {
                runtime.Model.Settings.Theme = theme;
                runtime.Model.Language = language;
                window.Width = size.Item1; window.Height = size.Item2;
                for (var tab = 0; tab < 6; tab++)
                {
                    tabs.SelectedIndex = tab; await Pump(window);
                    Require(ReferenceEquals(window.Background, window.FindResource("WindowBackgroundBrush")), "Derived main window ignores selected theme.");
                    Require(ReferenceEquals(window.Foreground, window.FindResource("TextPrimaryBrush")), "Main window text ignores selected theme.");
                    foreach (var box in Find<TextBox>(tabs))
                    {
                        if (box.Background is SolidColorBrush { Color.A: > 0 } bg && box.Foreground is SolidColorBrush fg)
                            Require(Contrast(fg.Color, bg.Color) >= 4.5, "TextBox theme contrast below 4.5.");
                    }
                    foreach (var grid in Find<DataGrid>(tabs))
                        Require(ReferenceEquals(grid.Background, window.FindResource("SurfaceBackgroundBrush")), "DataGrid has stale background.");
                    if (tab == 2)
                    {
                        var view = Find<DiskMapView>(tabs).Single();
                        var canvas = Find<DiskMapControl>(view).Single();
                        var visible = canvas.TransformToAncestor(view).TransformBounds(new Rect(canvas.RenderSize));
                        visible.Intersect(new Rect(view.RenderSize));
                        Require(visible.Height >= 180 && Math.Abs(visible.Height-canvas.ActualHeight) < 1, "Map is clipped at minimum size.");
                    }
                    if (tab == 5)
                    {
                        var selector = Find<ComboBox>(tabs).Single(b => b.ItemsSource == runtime.Model.Settings.Themes);
                        Require(selector.SelectedItem?.ToString() == theme, "Theme selector value is stale.");
                        selector.IsDropDownOpen = true; await Pump(window);
                        var item = (ComboBoxItem)selector.ItemContainerGenerator.ContainerFromIndex(0);
                        Require(item is not null && ReferenceEquals(item.Background, window.FindResource("ControlBackgroundBrush")), "Dropdown popup ignores theme.");
                        selector.IsDropDownOpen = false;
                    }
                    if (evidence is not null)
                    {
                        Directory.CreateDirectory(evidence);
                        var bitmap = new RenderTargetBitmap((int)window.ActualWidth, (int)window.ActualHeight, 96, 96, PixelFormats.Pbgra32); bitmap.Render(window);
                        var png = new PngBitmapEncoder(); png.Frames.Add(BitmapFrame.Create(bitmap));
                        using var file = File.Create(Path.Combine(evidence, $"theme-{theme}-{language}-{size.Item1}-{size.Item2}-tab{tab}.png")); png.Save(file);
                    }
                }
                var review = new ManualDeletionReviewWindow(null, null, runtime.Locale, runtime.Model.Bytes) { ShowInTaskbar = false, ShowActivated = false, Left = -10000, Top = -10000 };
                review.Show(); await Pump(review);
                Require(ReferenceEquals(review.Background, window.FindResource("WindowBackgroundBrush")), "Derived review window ignores theme."); review.Close();
            }
            Console.WriteLine("48 theme/language/size views; derived windows, popup, field contrast and grids passed");
        }
        finally { window.AllowClose = true; window.Close(); }
    }
    private static double Contrast(Color a, Color b) { static double L(Color c) { static double V(byte n) { var x = n / 255d; return x <= .04045 ? x / 12.92 : Math.Pow((x + .055) / 1.055, 2.4); } return .2126 * V(c.R) + .7152 * V(c.G) + .0722 * V(c.B); } var x = L(a); var y = L(b); return (Math.Max(x,y)+.05)/(Math.Min(x,y)+.05); }
    private static void Require(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private static async Task Pump(Window window) { window.UpdateLayout(); await window.Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle); window.UpdateLayout(); }
    private static IEnumerable<T> Find<T>(DependencyObject node) where T : DependencyObject { if (node is T t) yield return t; for (var i = 0; i < VisualTreeHelper.GetChildrenCount(node); i++) foreach (var c in Find<T>(VisualTreeHelper.GetChild(node,i))) yield return c; }
    private sealed class NoRegistry : IRunRegistry { public string? Read(string name) => null; public void Write(string name,string value) => throw new InvalidOperationException("Registry writes forbidden in theme probe."); public void Delete(string name) => throw new InvalidOperationException("Registry writes forbidden in theme probe."); }
}
