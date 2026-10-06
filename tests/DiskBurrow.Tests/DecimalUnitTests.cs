using System.Globalization;
using System.Text.Json;
using DiskBurrow.App.Services;
using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;

namespace DiskBurrow.Tests;

public sealed class DecimalUnitTests
{
    [Theory]
    [InlineData("en-US", 1_000_000_000L, "1.00 GB")]
    [InlineData("ru-RU", 1_000_000_000L, "1,00 ГБ")]
    [InlineData("en-US", -5_500_000_000L, "-5.50 GB")]
    [InlineData("en-US", 1_073_741_824L, "1.07 GB")]
    public void DisplayConvertsActualBytesToDecimalGigabytes(string language, long bytes, string expected)
    {
        Assert.Equal(expected, new ByteConverter().Convert(bytes, typeof(string), null!, CultureInfo.GetCultureInfo(language)));
    }

    [Theory]
    [InlineData(16_106_127_360L, 5_368_709_120L)]
    [InlineData(long.MaxValue, long.MaxValue - 1)]
    public async Task ExistingByteThresholdsDisplayInGBAndSaveWithoutByteDrift(long low, long growth)
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        var original = new AppSettings { LowSpaceBytes = low, GrowthBytes = growth };
        await store.SaveAsync(original, default);
        var vm = Model(await store.LoadAsync(default), store);
        if (low == 16_106_127_360L)
        {
            Assert.Equal(16.10612736m, Convert.ToDecimal(vm.LowGB));
            Assert.Equal(5.36870912m, Convert.ToDecimal(vm.GrowthGB));
        }
        await vm.SaveAsync(default);
        var loaded = await new SettingsStore(tree.Root).LoadAsync(default);
        Assert.Equal(low, loaded.LowSpaceBytes);
        Assert.Equal(growth, loaded.GrowthBytes);
    }

    [Fact]
    public async Task NewDecimalThresholdInputSavesExactBytes()
    {
        using var tree = new TempTree();
        var store = new SettingsStore(tree.Root);
        var vm = Model(new(), store);
        vm.LowGB = 15.25m;
        vm.GrowthGB = 0.000000001m;
        await vm.SaveAsync(default);
        var loaded = await store.LoadAsync(default);
        Assert.Equal(15_250_000_000L, loaded.LowSpaceBytes);
        Assert.Equal(1L, loaded.GrowthBytes);
    }

    [Theory]
    [InlineData("{}", 16_106_127_360L, 5_368_709_120L)]
    [InlineData("{\"LowSpaceBytes\":12345678901}", 12_345_678_901L, 5_368_709_120L)]
    [InlineData("{\"GrowthBytes\":17}", 16_106_127_360L, 17L)]
    public async Task LegacySettingsKeepEffectiveByteThresholdsAcrossSaveAndRestart(string json, long low, long growth)
    {
        using var tree = new TempTree();
        var path = Path.Combine(tree.Root, "settings.json");
        await File.WriteAllTextAsync(path, json);
        var store = new SettingsStore(tree.Root);
        var loaded = await store.LoadAsync(default);
        Assert.Equal(low, loaded.LowSpaceBytes);
        Assert.Equal(growth, loaded.GrowthBytes);
        Assert.Equal(json, await File.ReadAllTextAsync(path));
        await Model(loaded, store).SaveAsync(default);
        var restarted = await new SettingsStore(tree.Root).LoadAsync(default);
        Assert.Equal(low, restarted.LowSpaceBytes);
        Assert.Equal(growth, restarted.GrowthBytes);
    }

    [Fact]
    public async Task NewInstallationUsesDecimalWarningBoundaries()
    {
        using var tree = new TempTree();
        var loaded = await new SettingsStore(tree.Root).LoadAsync(default);
        var now = DateTimeOffset.UnixEpoch;
        var policy = new AlertPolicy(loaded, @"C:\");
        Assert.Empty(policy.Evaluate(null, null, 15_000_000_000L, now));
        Assert.Single(policy.Evaluate(null, null, 14_999_999_999L, now));
        var growth = new AlertPolicy(loaded, @"C:\");
        Assert.Single(growth.Evaluate(MonitoringTests.Snapshot(0), MonitoringTests.Snapshot(5_000_000_000L), 20_000_000_000L, now));
        Assert.Empty(new AlertPolicy(loaded, @"C:\").Evaluate(MonitoringTests.Snapshot(0), MonitoringTests.Snapshot(4_999_999_999L), 20_000_000_000L, now));
    }

    [Fact]
    public async Task ExportKeepsByteCountsIndependentOfPresentationUnits()
    {
        using var tree = new TempTree();
        var path = Path.Combine(tree.Root, "report.json");
        await new ReportExporter().ExportAsync(MonitoringTests.Snapshot(1_073_741_824L), path, true, default);
        using var document = JsonDocument.Parse(await File.ReadAllTextAsync(path));
        Assert.Equal(1_073_741_824L, document.RootElement.GetProperty("Directories")[0].GetProperty("LogicalBytes").GetInt64());
    }

    private static SettingsViewModel Model(AppSettings settings, SettingsStore store) =>
        new(settings, store, new AutostartRegistration(new Registry(), @"C:\fixture\DiskBurrow.exe"), _ => null, _ => { });

    private sealed class Registry : IRunRegistry
    {
        public string? Read(string name) => null;
        public void Write(string name, string value) => throw new InvalidOperationException("Autostart is disabled in this fixture.");
        public void Delete(string name) => throw new InvalidOperationException("Autostart is disabled in this fixture.");
    }
}
