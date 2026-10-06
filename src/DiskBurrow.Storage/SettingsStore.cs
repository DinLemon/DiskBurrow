using DiskBurrow.Core.Monitoring;
using System.Text.Json;

namespace DiskBurrow.Storage;

public sealed class SettingsStore(string directory) : IMonitoringStateStore
{
    private readonly string _directory = Path.GetFullPath(directory);
    private readonly SemaphoreSlim _writeGate = new(1);
    public string? LastUserMessage { get; private set; }

    public Task<AppSettings> LoadAsync(CancellationToken ct) => LoadAsync("settings.json", new AppSettings(), s => s.Validate(), ct,
        (settings, document) =>
        {
            // Existing documents used binary defaults for omitted thresholds. Explicit
            // fields already contain bytes and must never be rescaled. Save materializes
            // both fields, so subsequent loads retain the exact effective thresholds.
            if (!document.TryGetProperty(nameof(AppSettings.LowSpaceBytes), out _)) settings.LowSpaceBytes = 15L << 30;
            if (!document.TryGetProperty(nameof(AppSettings.GrowthBytes), out _)) settings.GrowthBytes = 5L << 30;
        });
    public Task<AlertSuppressionState> LoadAlertStateAsync(CancellationToken ct) => LoadAsync("alerts.json", new AlertSuppressionState(), state =>
    {
        if (state.LowSpaceRoots is null || state.LastGrowthNotifiedUtc is null || state.LastGrowthSnapshot is null ||
            state.EvaluatedGrowthSnapshots is null || state.EvaluatedGrowthSnapshots.Values.Any(v => v is null || v.Count > 30))
            throw new ArgumentException("Invalid alert suppression state.");
        foreach (var path in state.LowSpaceRoots.Concat(state.LastGrowthNotifiedUtc.Keys).Concat(state.LastGrowthSnapshot.Keys)
            .Concat(state.EvaluatedGrowthSnapshots.Keys))
            if (!Path.IsPathFullyQualified(path)) throw new ArgumentException("Invalid alert suppression path.");
    }, ct);

    public Task SaveAsync(AppSettings settings, CancellationToken ct)
    {
        settings.Validate();
        return SaveAsync("settings.json", settings, ct);
    }
    public Task SaveAlertStateAsync(AlertSuppressionState state, CancellationToken ct) => SaveAsync("alerts.json", state, ct);

    private async Task<T> LoadAsync<T>(string name, T fallback, Action<T> validate, CancellationToken ct,
        Action<T, JsonElement>? normalize = null)
    {
        ct.ThrowIfCancellationRequested();
        try
        {
            var path = Path.Combine(_directory, name);
            await using var stream = File.OpenRead(path);
            using var document = await JsonDocument.ParseAsync(stream, cancellationToken: ct);
            var value = document.RootElement.Deserialize<T>()
                ?? throw new JsonException("The settings document is null.");
            normalize?.Invoke(value, document.RootElement);
            validate(value);
            LastUserMessage = null;
            return value;
        }
        catch (Exception e) when (e is FileNotFoundException or DirectoryNotFoundException)
        {
            LastUserMessage = null;
            return fallback;
        }
        catch (Exception e) when (e is JsonException or ArgumentException or IOException or UnauthorizedAccessException)
        {
            LastUserMessage = $"{name}: {e.Message}";
            return fallback;
        }
    }

    private async Task SaveAsync<T>(string name, T value, CancellationToken ct)
    {
        await _writeGate.WaitAsync(ct);
        string? temporary = null;
        try
        {
            Directory.CreateDirectory(_directory);
            temporary = Path.Combine(_directory, $".{name}.{Guid.NewGuid():N}.tmp");
            await using (var stream = new FileStream(temporary, FileMode.CreateNew, FileAccess.Write, FileShare.None,
                4096, FileOptions.Asynchronous | FileOptions.WriteThrough))
            {
                await JsonSerializer.SerializeAsync(stream, value, cancellationToken: ct);
                await stream.FlushAsync(ct);
                stream.Flush(flushToDisk: true);
            }
            ct.ThrowIfCancellationRequested();
            File.Move(temporary, Path.Combine(_directory, name), overwrite: true);
            LastUserMessage = null;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            LastUserMessage = $"{name}: {e.Message}";
            throw;
        }
        finally
        {
            if (temporary is not null)
            {
                try { File.Delete(temporary); }
                catch (Exception e) when (e is IOException or UnauthorizedAccessException) { LastUserMessage = e.Message; }
            }
            _writeGate.Release();
        }
    }
}
