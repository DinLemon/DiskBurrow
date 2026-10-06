using System.Text.Json;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Storage;

namespace DiskBurrow.Tests;

public sealed class ThemeSettingsTests
{
    [Fact]
    public async Task DarkChoiceSurvivesSaveAndReload()
    {
        using var tree = new TempTree();
        var settings = JsonSerializer.Deserialize<AppSettings>("{\"Theme\":\"dark\"}")!;
        var store = new SettingsStore(tree.Root);
        await store.SaveAsync(settings, default);
        var loaded = await store.LoadAsync(default);
        using var json = JsonDocument.Parse(JsonSerializer.Serialize(loaded));
        Assert.True(json.RootElement.TryGetProperty("Theme", out var theme), "Theme choice was discarded.");
        Assert.Equal("dark", theme.GetString());
    }

    [Fact]
    public void InvalidThemeIsRejectedBeforeSaving()
    {
        var settings = JsonSerializer.Deserialize<AppSettings>("{\"Theme\":\"invalid\"}")!;
        Assert.Throws<ArgumentException>(() => settings.Validate());
    }
}
