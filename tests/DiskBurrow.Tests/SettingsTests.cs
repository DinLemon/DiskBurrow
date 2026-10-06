using DiskBurrow.Core.Monitoring;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;

namespace DiskBurrow.Tests;

public sealed class SettingsTests
{
    [Fact]
    public void DefaultsMatchSpec()
    {
        var settings = new AppSettings();
        Assert.Equal(6, settings.IntervalHours);
        Assert.Equal(TimeSpan.FromMinutes(5), settings.InitialDelay);
        Assert.Equal(TimeSpan.FromMinutes(5), settings.FreeSpaceInterval);
        Assert.Equal(15L * 1024 * 1024 * 1024, settings.LowSpaceBytes);
        Assert.Equal(5L * 1024 * 1024 * 1024, settings.GrowthBytes);
        Assert.Equal("ru", settings.Language);
        Assert.False(settings.AllowOnBattery);
        Assert.False(settings.Autostart);
        Assert.Null(settings.ApprovedCustomTempPath);
    }

    [Theory]
    [InlineData(2, 10, 10)]
    [InlineData(6, -1, 10)]
    [InlineData(6, 10, -1)]
    public async Task InvalidIntervalAndNegativeThresholdsAreRejected(int interval, long low, long growth)
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        await Assert.ThrowsAsync<ArgumentException>(() => store.SaveAsync(new AppSettings
            { IntervalHours = interval, LowSpaceBytes = low, GrowthBytes = growth }, default));
        Assert.False(File.Exists(Path.Combine(tree.Root, "settings.json")));
    }

    [Fact]
    public async Task SettingsSaveIsAtomicAndPreservesChoices()
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        var settings = new AppSettings { IntervalHours = 12, Language = "en", Paused = true,
            ExcludedPaths = [@"C:\keep"], ApprovedCustomTempPath = @"C:\narrow-temp" };
        await store.SaveAsync(settings, default);
        using var cancelled = new CancellationTokenSource();
        cancelled.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => store.SaveAsync(new() { IntervalHours = 1 }, cancelled.Token));
        var loaded = await new SettingsStore(tree.Root).LoadAsync(default);
        Assert.Equal(12, loaded.IntervalHours);
        Assert.Equal("en", loaded.Language);
        Assert.True(loaded.Paused);
        Assert.Equal(@"C:\keep", Assert.Single(loaded.ExcludedPaths));
        Assert.Equal(@"C:\narrow-temp", loaded.ApprovedCustomTempPath);
        Assert.Single(Directory.GetFiles(tree.Root));
    }

    [Theory]
    [InlineData("broken")]
    [InlineData("{\"IntervalHours\":2}")]
    [InlineData("{\"Language\":\"fr\"}")]
    [InlineData("{\"ExcludedPaths\":null}")]
    public async Task InvalidSettingsRecoverToVisibleDefaults(string text)
    {
        using var tree = new TempTree();
        File.WriteAllText(Path.Combine(tree.Root, "settings.json"), text);
        var store = new SettingsStore(tree.Root);
        Assert.Equal(6, (await store.LoadAsync(default)).IntervalHours);
        Assert.NotNull(store.LastUserMessage);
        Assert.Equal(text, File.ReadAllText(Path.Combine(tree.Root, "settings.json")));
    }

    [Fact]
    public async Task AlertStateSurvivesRestartAndInvalidStateIsVisible()
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        var state = new AlertSuppressionState();
        state.LowSpaceRoots.Add(@"C:\");
        state.LastGrowthNotifiedUtc[@"C:\cache"] = DateTimeOffset.UnixEpoch;
        await store.SaveAlertStateAsync(state, default);
        var restored = await new SettingsStore(tree.Root).LoadAlertStateAsync(default);
        Assert.Contains(@"C:\", restored.LowSpaceRoots);
        Assert.Equal(DateTimeOffset.UnixEpoch, restored.LastGrowthNotifiedUtc[@"C:\cache"]);
        File.WriteAllText(Path.Combine(tree.Root, "alerts.json"), "broken");
        Assert.Empty((await store.LoadAlertStateAsync(default)).LowSpaceRoots);
        Assert.NotNull(store.LastUserMessage);
    }

    [Fact]
    public async Task SuccessfulSettingsRecoveryClearsCurrentDiagnostic()
    {
        using var tree = new TempTree();
        File.WriteAllText(Path.Combine(tree.Root, "settings.json"), "broken");
        var store = new SettingsStore(tree.Root);
        await store.LoadAsync(default);
        Assert.NotNull(store.LastUserMessage);
        await store.SaveAsync(new(), default);
        Assert.Null(store.LastUserMessage);
        File.WriteAllText(Path.Combine(tree.Root, "settings.json"), "broken");
        await store.LoadAsync(default);
        File.WriteAllText(Path.Combine(tree.Root, "settings.json"), "{}");
        await store.LoadAsync(default);
        Assert.Null(store.LastUserMessage);
    }

    [Fact]
    public async Task SuppressedGrowthEventIsPersistedAndCannotReplayAfterRestart()
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        var now = DateTimeOffset.UtcNow;
        var policy = new AlertPolicy(new(), @"C:\");
        Assert.Single(policy.Evaluate(MonitoringTests.Snapshot(0), MonitoringTests.Snapshot(6L << 30), 20L << 30, now));
        var previous = MonitoringTests.Snapshot(6L << 30);
        var current = MonitoringTests.Snapshot(12L << 30);
        Assert.Empty(policy.Evaluate(previous, current, 20L << 30, now.AddHours(23)));
        await store.SaveAlertStateAsync(policy.State, default);
        var restored = new AlertPolicy(new(), @"C:\", await store.LoadAlertStateAsync(default));
        Assert.Empty(restored.Evaluate(previous, current, 20L << 30, now.AddDays(1)));
    }

    [Fact]
    public async Task UnreadableSettingsAreVisibleAndExistingDocumentSurvivesFailedSave()
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        await store.SaveAsync(new(), default);
        var path = Path.Combine(tree.Root, "settings.json");
        using (File.Open(path, FileMode.Open, FileAccess.ReadWrite, FileShare.None))
        {
            Assert.Equal(6, (await store.LoadAsync(default)).IntervalHours);
            Assert.NotNull(store.LastUserMessage);
            var error = await Record.ExceptionAsync(() => store.SaveAsync(new() { IntervalHours = 12 }, default));
            Assert.True(error is IOException or UnauthorizedAccessException);
        }
        Assert.Equal(6, (await store.LoadAsync(default)).IntervalHours);
        Assert.Single(Directory.GetFiles(tree.Root));
    }

    [Fact]
    public void AutostartWritesOnlyDiskBurrowValueAndQuotesExe()
    {
        var registry = new MemoryRunRegistry();
        registry.Values["Other"] = "keep";
        var registration = new AutostartRegistration(registry, @"C:\Program Files\DiskBurrow\DiskBurrow.exe");
        registration.Apply(true);
        Assert.Equal("\"C:\\Program Files\\DiskBurrow\\DiskBurrow.exe\"", registry.Values["DiskBurrow"]);
        registration.Apply(false);
        Assert.Equal("keep", registry.Values["Other"]);
        Assert.False(registry.Values.ContainsKey("DiskBurrow"));
    }

    [Fact]
    public void MovedExeIsDetectedAndForeignValueIsNotDeleted()
    {
        var registry = new MemoryRunRegistry();
        registry.Values["DiskBurrow"] = "\"C:\\old\\DiskBurrow.exe\"";
        var registration = new AutostartRegistration(registry, @"C:\new\DiskBurrow.exe");
        Assert.True(registration.IsPathChanged);
        registration.Apply(false);
        Assert.Equal("\"C:\\old\\DiskBurrow.exe\"", registry.Values["DiskBurrow"]);
        Assert.NotNull(registration.LastUserMessage);
    }

    [Fact]
    public void AutostartOffDoesNotTouchRegistry()
    {
        var registry = new MemoryRunRegistry();
        var registration = new AutostartRegistration(registry, @"C:\app\DiskBurrow.exe");
        registration.Apply(false);
        Assert.Equal(0, registry.Writes);
    }

    [Fact]
    public void WindowsEnvironmentReportsRealVolumeAndRejectsRemoteOrUnknownRoots()
    {
        var environment = new WindowsEnvironment();
        Assert.Equal(Path.GetPathRoot(Environment.GetFolderPath(Environment.SpecialFolder.Windows)), environment.SystemRoot);
        var volume = environment.GetVolumeSpace(environment.SystemRoot);
        Assert.True(volume.TotalBytes > 0);
        Assert.InRange(volume.FreeBytes, 0, volume.TotalBytes);
        Assert.Equal(volume.TotalBytes - volume.FreeBytes, volume.UsedBytes);
        Assert.True(environment.IsLocalRoot(environment.SystemRoot));
        Assert.False(environment.IsLocalRoot(@"\\server\share"));
        Assert.False(environment.IsLocalRoot("relative"));
        Assert.False(environment.IsLocalRoot(@"Z:\does-not-exist\"));
    }

    private sealed class MemoryRunRegistry : IRunRegistry
    {
        public Dictionary<string, string> Values { get; } = [];
        public int Writes { get; private set; }
        public string? Read(string name) => Values.GetValueOrDefault(name);
        public void Write(string name, string value) { Writes++; Values[name] = value; }
        public void Delete(string name) { Writes++; Values.Remove(name); }
    }
}
