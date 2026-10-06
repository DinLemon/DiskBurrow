using DiskBurrow.Core.History;
using DiskBurrow.Core.Scanning;

namespace DiskBurrow.Core.Monitoring;

public sealed class MonitoringCoordinator : IAsyncDisposable
{
    private AppSettings _settings;
    private readonly IDiskScanner _scanner;
    private readonly IHistoryStore _history;
    private readonly IMonitoringEnvironment _environment;
    private readonly MonitoringAlertDispatcher _alerts;
    private readonly TimeProvider _time;
    private readonly SchedulePolicy _schedule;
    private readonly object _sync = new();
    private readonly SemaphoreSlim _operationGate = new(1);
    private readonly CancellationTokenSource _lifetime = new();
    private readonly Dictionary<string, Task<ScanSnapshot>> _scans = new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<string, IDiskScanner> _scanEngines = new(StringComparer.OrdinalIgnoreCase);
    private readonly Dictionary<string, ScanSnapshot> _previous = new(StringComparer.OrdinalIgnoreCase);
    private readonly HashSet<Task> _exclusiveTasks = [];
    private CancellationTokenSource? _activeScanCancellation;
    private Task? _loop;
    private bool _operationRunning;
    private bool _disposed;

    public MonitoringCoordinator(AppSettings settings, IDiskScanner scanner, IHistoryStore history,
        IMonitoringEnvironment environment, IMonitoringStateStore stateStore, TimeProvider? timeProvider = null)
    {
        settings.Validate();
        _settings = CaptureSettings(settings);
        _scanner = scanner;
        _history = history;
        _environment = environment;
        _time = timeProvider ?? TimeProvider.System;
        _schedule = new(_settings, _time.GetUtcNow());
        _alerts = new(environment, stateStore, _time, () => { lock (_sync) return _settings; },
            Report, alerts => AlertsRaised?.Invoke(alerts), observation=>VolumeObserved?.Invoke(observation));
    }

    public ScanSnapshot? LiveSnapshot { get; private set; }
    public string? LastUserMessage { get; private set; }
    public bool OperationRunning { get { lock (_sync) return _operationRunning; } }
    public bool ScanRunning { get { lock (_sync) return _activeScanCancellation is not null; } }
    public event Action<IReadOnlyList<AlertEvent>>? AlertsRaised;
    public event Action<ScanSnapshot>? SnapshotChanged;
    public event Action<ScanProgress>? ProgressChanged;
    public event Action<VolumeObservation>? VolumeObserved;
    public event Action? StatusChanged;

    // Counter-only observation: does not touch the scan schedule or evaluate historical growth.
    public Task<VolumeObservation> ReadVolumeAsync(string root,CancellationToken ct=default)=>Task.Run(()=>
    {
        ct.ThrowIfCancellationRequested();
        if(!_environment.IsLocalRoot(root))throw new ArgumentException("A confirmed local directory is required.",nameof(root));
        return new VolumeObservation(root,_environment.GetVolumeSpace(root),_time.GetUtcNow());
    },ct);

    public Task<ScanSnapshot> RequestScanAsync(CancellationToken ct) => RequestScanAsync(_environment.SystemRoot, ct);
    public Task<ScanSnapshot> RequestScanAsync(string root, CancellationToken ct) => RequestScanAsync(root, _scanner, ct);
    // Interactive-only alternative. The scheduled loop always calls the normal-scanner overload.
    public Task<ScanSnapshot> RequestScanAsync(string root, IDiskScanner scanner, CancellationToken ct)
    {
        ArgumentNullException.ThrowIfNull(scanner);
        if (ct.IsCancellationRequested) return Task.FromCanceled<ScanSnapshot>(ct);
        try
        {
            if (!_environment.IsLocalRoot(root)) throw new ArgumentException("A confirmed local directory is required.", nameof(root));
            root = SnapshotComparer.NormalizePath(root);
        }
        catch (Exception e) { return Task.FromException<ScanSnapshot>(e); }
        lock (_sync)
        {
            if (_lifetime.IsCancellationRequested) return Task.FromCanceled<ScanSnapshot>(_lifetime.Token);
            if (!_scans.TryGetValue(root, out var task))
            {
                task = ScanAsync(root, scanner);
                _scanEngines.Add(root, scanner);
                _scans.Add(root, task);
            }
            else if (!ReferenceEquals(_scanEngines[root], scanner))
                return Task.FromException<ScanSnapshot>(new InvalidOperationException("A different scan engine already has a request for this root."));
            // Cancelling one caller's wait does not cancel another caller's shared scan.
            return ct.CanBeCanceled ? task.WaitAsync(ct) : task;
        }
    }

    private async Task<ScanSnapshot> ScanAsync(string root, IDiskScanner scanner)
    {
        await Task.Yield(); // Register task before any synchronous fake/native completion.
        using var cancellation = CancellationTokenSource.CreateLinkedTokenSource(_lifetime.Token);
        var ct = cancellation.Token;
        var entered = false;
        try
        {
            await _alerts.InitializeAsync(_lifetime.Token);
            await _operationGate.WaitAsync(ct);
            entered = true;
            lock (_sync) { _operationRunning = true; _activeScanCancellation = cancellation; }
            StatusChanged?.Invoke();
            // Revalidate after queuing, in case the selected root disappeared meanwhile.
            if (!_environment.IsLocalRoot(root)) throw new ArgumentException("The local scan root is no longer available.", nameof(root));
            ScanSnapshot? previous = _previous.GetValueOrDefault(root);
            if (previous is null)
            {
                try { previous = (await _history.LoadRecentAsync(root, 1, ct)).FirstOrDefault(); }
                catch (Exception e) when (e is not OperationCanceledException) { Report(e.Message); }
            }
            var snapshot = await scanner.ScanAsync(root, new ScanProgressRelay(p => ProgressChanged?.Invoke(p)), ct);
            ct.ThrowIfCancellationRequested();
            if (!snapshot.TraversalCompleted) return snapshot;
            if (!StringComparer.OrdinalIgnoreCase.Equals(SnapshotComparer.NormalizePath(snapshot.Root), root))
                throw new InvalidOperationException("Scanner returned a snapshot for a different root.");
            LiveSnapshot = snapshot;
            _previous[root] = snapshot;
            if (StringComparer.OrdinalIgnoreCase.Equals(root, SnapshotComparer.NormalizePath(_environment.SystemRoot)))
                lock (_sync) _schedule.RecordFullScan(_time.GetUtcNow());
            SnapshotChanged?.Invoke(snapshot);
            try
            {
                var result = await _history.SaveAsync(snapshot, ct);
                if (!result.Saved) Report(result.UserMessage ?? "History write was not saved.");
            }
            catch (Exception e) when (e is not OperationCanceledException) { Report(e.Message); }
            await _alerts.EvaluateAsync(previous, snapshot, root, ct);
            return snapshot;
        }
        catch (Exception e) when (e is not OperationCanceledException) { Report(e.Message); throw; }
        finally
        {
            lock (_sync)
            {
                _scans.Remove(root);
                _scanEngines.Remove(root);
                if (entered) { _operationRunning = false; _activeScanCancellation = null; }
            }
            if (entered) _operationGate.Release();
            StatusChanged?.Invoke();
        }
    }

    public Task<T> RunExclusiveAsync<T>(Func<CancellationToken, Task<T>> operation, CancellationToken ct)
    {
        lock (_sync)
        {
            if (_lifetime.IsCancellationRequested) return Task.FromCanceled<T>(_lifetime.Token);
            var task = ExecuteExclusiveAsync(operation, ct);
            _exclusiveTasks.Add(task);
            _ = ForgetExclusiveAsync(task);
            return task;
        }
    }

    private async Task<T> ExecuteExclusiveAsync<T>(Func<CancellationToken, Task<T>> operation, CancellationToken ct)
    {
        await Task.Yield();
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(ct, _lifetime.Token);
        await _operationGate.WaitAsync(linked.Token);
        try
        {
            lock (_sync) _operationRunning = true;
            StatusChanged?.Invoke();
            return await operation(linked.Token);
        }
        finally
        {
            lock (_sync) _operationRunning = false;
            _operationGate.Release();
            StatusChanged?.Invoke();
        }
    }

    private async Task ForgetExclusiveAsync(Task task)
    {
        try { await task; }
        catch (OperationCanceledException) { }
        catch (Exception e) { Report(e.Message); }
        finally { lock (_sync) _exclusiveTasks.Remove(task); }
    }

    public Task RunAsync(CancellationToken ct)
    {
        lock (_sync) return _loop ??= RunLoopAsync(ct);
    }

    private async Task RunLoopAsync(CancellationToken ct)
    {
        await Task.Yield();
        using var registration = ct.Register(() => _lifetime.Cancel());
        try
        {
            await _alerts.InitializeAsync(_lifetime.Token);
            while (true)
            {
                // TimeProvider timers use a monotonic clock; wall-clock changes cannot spin this loop.
                await Task.Delay(_settings.FreeSpaceInterval, _time, _lifetime.Token);
                await _alerts.EvaluateAsync(null, null, _environment.SystemRoot, _lifetime.Token);
                try
                {
                    var onBattery = _environment.IsOnBattery;
                    bool due;
                    lock (_sync) due = _schedule.IsFullScanDue(_time.GetUtcNow(), onBattery, _operationRunning);
                    if (due)
                        _ = ObserveBackgroundAsync(RequestScanAsync(_environment.SystemRoot, default));
                }
                catch (Exception e) when (e is not OperationCanceledException) { Report(e.Message); }
            }
        }
        catch (OperationCanceledException) when (_lifetime.IsCancellationRequested) { }
    }

    private async Task ObserveBackgroundAsync(Task scan)
    {
        try { await scan; }
        catch (OperationCanceledException) { }
        catch (Exception e) { Report(e.Message); }
    }

    public void CancelScan() { lock (_sync) _activeScanCancellation?.Cancel(); }
    public void Pause(bool paused)
    {
        AppSettings settings;
        lock (_sync) settings = _settings with { Paused = paused };
        ApplySettings(settings);
    }
    public void ApplySettings(AppSettings settings)
    {
        var captured = CaptureSettings(settings);
        lock (_sync) { _settings = captured; _schedule.ApplySettings(captured); }
        StatusChanged?.Invoke();
    }
    private static AppSettings CaptureSettings(AppSettings settings)
    {
        settings.Validate();
        return settings with { ExcludedPaths = settings.ExcludedPaths.ToArray() };
    }
    private void Report(string message) { LastUserMessage = message; StatusChanged?.Invoke(); }

    public async Task StopAsync()
    {
        Task[] tasks;
        lock (_sync)
        {
            _lifetime.Cancel();
            tasks = _scans.Values.Cast<Task>().Concat(_exclusiveTasks).Concat(_loop is null ? [] : new[] { _loop }).ToArray();
        }
        try { await Task.WhenAll(tasks); }
        catch (OperationCanceledException) { }
        catch (Exception e) { Report(e.Message); }
    }

    public async ValueTask DisposeAsync()
    {
        if (_disposed) return;
        await StopAsync();
        _disposed = true;
        _lifetime.Dispose();
        _operationGate.Dispose();
        _alerts.Dispose();
    }

    private sealed class ScanProgressRelay(Action<ScanProgress> action) : IProgress<ScanProgress>
    {
        public void Report(ScanProgress value) => action(value);
    }
}
