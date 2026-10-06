using System.Diagnostics;
using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using System.Windows.Threading;
using DiskBurrow.App;
using DiskBurrow.App.Services;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;

namespace DiskBurrow.Tests;

public sealed class BackgroundSnapshotTests
{
    [Theory]
    [InlineData("Map")]
    [InlineData("Largest")]
    [InlineData("Overview")]
    public async Task RealScanButtonPublishesWorkerSnapshotToLoadedWpfViews(string page)
    {
        var start = new ProcessStartInfo("dotnet") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true };
        start.ArgumentList.Add(typeof(BackgroundSnapshotTests).Assembly.Location);
        start.ArgumentList.Add("--verify-background-snapshot");
        start.ArgumentList.Add(page);
        using var process = Process.Start(start)!;
        var output = process.StandardOutput.ReadToEndAsync();
        var error = process.StandardError.ReadToEndAsync();
        try { await process.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(40)); }
        catch { if (!process.HasExited) process.Kill(); throw; }
        Assert.True(process.ExitCode == 0, $"WPF scan failed on {page}: {await output} {await error}");
        Assert.Contains("Worker snapshot published", await output);
    }

    internal static async Task VerifyAsync(string page)
    {
        using var fixture = new TempTree();
        var root = fixture.DirectoryAt("scan");
        var payload = fixture.FileAt("scan/payload.bin", 43);
        File.WriteAllText(Path.Combine(root, ".owned-test"), "DiskBurrow background snapshot fixture");
        var data = fixture.DirectoryAt("data");
        await new SettingsStore(data).SaveAsync(new() { Language = "en", Paused = true }, default);
        var dispatcher = Dispatcher.CurrentDispatcher;
        await using var runtime = await AppRuntime.CreateAsync(data, dispatcher, new NoRegistry(), new LocalEnvironment(root));
        var model = runtime.Model;
        var window = new MainWindow(model) { Left = -10000, Top = -10000, ShowInTaskbar = false, ShowActivated = false };
        string? affinityStack = null;
        void Capture(object? sender, FirstChanceExceptionEventArgs args)
        {
            if (args.Exception is InvalidOperationException &&
                (args.Exception.Message.Contains("Вызывающий поток", StringComparison.Ordinal) ||
                 args.Exception.Message.Contains("calling thread", StringComparison.OrdinalIgnoreCase)))
                affinityStack ??= args.Exception + Environment.NewLine + Environment.StackTrace;
        }
        AppDomain.CurrentDomain.FirstChanceException += Capture;
        try
        {
            window.Show();
            var tabs = Descendants<TabControl>(window).Single();
            tabs.SelectedIndex = page switch { "Overview" => 0, "Largest" => 1, "Map" => 2, _ => throw new ArgumentException("Unknown fixture page.") };
            window.UpdateLayout();
            await dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
            var completed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            model.PropertyChanged += (_, _) =>
            {
                if (model.ErrorDetails is not null || !model.Busy && model.Overview.Snapshot is not null && model.LargestFiles.Any(row => row.Path == payload)) completed.TrySetResult();
            };
            var scan = Descendants<Button>(window).Single(button => button.Tag as string == "Scan");
            Assert.True(scan.IsEnabled);
            scan.RaiseEvent(new RoutedEventArgs(Button.ClickEvent, scan));
            await completed.Task.WaitAsync(TimeSpan.FromSeconds(15));
            Assert.True(model.ErrorDetails is null, $"{model.ErrorDetails}{Environment.NewLine}{affinityStack}");
            Assert.Null(affinityStack);
            Assert.NotNull(model.Overview.Snapshot);
            Assert.Contains(model.LargestFiles, row => row.Path == payload && row.Observation.LogicalBytes == 43);
            await model.Map.WaitForSearchAsync();
            Assert.NotNull(model.Map.Tree);
            Console.WriteLine($"Worker snapshot published through real {page} page scan button.");
        }
        finally
        {
            AppDomain.CurrentDomain.FirstChanceException -= Capture;
            window.AllowClose = true;
            window.Close();
        }
    }

    private static IEnumerable<T> Descendants<T>(DependencyObject node) where T : DependencyObject
    {
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(node); index++)
        {
            var child = VisualTreeHelper.GetChild(node, index);
            if (child is T match) yield return match;
            foreach (var nested in Descendants<T>(child)) yield return nested;
        }
    }

    private sealed class LocalEnvironment(string root) : IMonitoringEnvironment
    {
        public string SystemRoot => root;
        public bool IsOnBattery => false;
        public bool IsLocalRoot(string path) => string.Equals(Path.TrimEndingDirectorySeparator(path), root, StringComparison.OrdinalIgnoreCase);
        public long GetFreeBytes(string path) => 20_000_000_000;
        public VolumeSpace GetVolumeSpace(string path) => new(40_000_000_000, 20_000_000_000);
    }

    private sealed class NoRegistry : IRunRegistry
    {
        public string? Read(string name) => null;
        public void Write(string name, string value) => throw new InvalidOperationException("Fixture autostart is disabled.");
        public void Delete(string name) => throw new InvalidOperationException("Fixture autostart is disabled.");
    }
}
