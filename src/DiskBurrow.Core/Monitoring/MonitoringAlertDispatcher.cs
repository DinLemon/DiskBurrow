using DiskBurrow.Core.Scanning;

namespace DiskBurrow.Core.Monitoring;

// Alert persistence has its own gate: a full scan/exclusive operation never blocks the volume check.
internal sealed class MonitoringAlertDispatcher(
    IMonitoringEnvironment environment, IMonitoringStateStore stateStore, TimeProvider time,
    Func<AppSettings> getSettings, Action<string> report, Action<IReadOnlyList<AlertEvent>> notify,
    Action<VolumeObservation> observed) : IDisposable
{
    private readonly object _sync = new();
    private readonly SemaphoreSlim _gate = new(1);
    private Task? _initialize;
    private AlertPolicy? _policy;
    private AppSettings? _policySettings;

    public Task InitializeAsync(CancellationToken ct)
    {
        lock (_sync) return _initialize ??= LoadAsync(ct);
    }

    private async Task LoadAsync(CancellationToken ct)
    {
        try
        {
            var state = await stateStore.LoadAlertStateAsync(ct);
            if (stateStore.LastUserMessage is { } message) report(message);
            _policySettings = getSettings();
            _policy = new(_policySettings, environment.SystemRoot, state);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            report(e.Message);
            _policySettings = getSettings();
            _policy = new(_policySettings, environment.SystemRoot);
        }
    }

    public async Task EvaluateAsync(ScanSnapshot? previous, ScanSnapshot? current, string root, CancellationToken ct)
    {
        await InitializeAsync(ct);
        await _gate.WaitAsync(ct);
        try
        {
            var settings = getSettings();
            if (!ReferenceEquals(settings, _policySettings))
            {
                _policy = new(settings, environment.SystemRoot, _policy!.State);
                _policySettings = settings;
            }
            var now=time.GetUtcNow();
            long free;
            try
            {
                var space=await Task.Run(()=>environment.GetVolumeSpace(root),ct);
                free=space.FreeBytes;observed(new(root,space,now));
            }
            catch(Exception error) when(error is not OperationCanceledException)
            {
                report(error.Message);
                free=await Task.Run(()=>environment.GetFreeBytes(root),ct);
            }
            var alerts = _policy!.Evaluate(previous, current, free, now);
            // On failure retain suppression in memory and report the persistence failure to the UI.
            try { await stateStore.SaveAlertStateAsync(_policy.State, ct); }
            catch (Exception e) when (e is not OperationCanceledException) { report(e.Message); }
            if (alerts.Count > 0) notify(alerts);
        }
        catch (Exception e) when (e is not OperationCanceledException) { report(e.Message); }
        finally { _gate.Release(); }
    }

    public void Dispose() => _gate.Dispose();
}
