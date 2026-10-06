namespace DiskBurrow.Core.Monitoring;

public sealed class SchedulePolicy
{
    private AppSettings _settings;
    private readonly DateTimeOffset _startedUtc;
    private DateTimeOffset? _lastFullScan;

    public SchedulePolicy(AppSettings settings, DateTimeOffset startedUtc)
    {
        settings.Validate();
        _settings = settings;
        _startedUtc = startedUtc;
    }
    public bool IsFullScanDue(DateTimeOffset nowUtc, bool onBattery, bool operationRunning) =>
        !_settings.Paused && (!onBattery || _settings.AllowOnBattery) && !operationRunning &&
        nowUtc >= (_lastFullScan is { } last ? last.AddHours(_settings.IntervalHours) : _startedUtc + _settings.InitialDelay);

    public void RecordFullScan(DateTimeOffset nowUtc) => _lastFullScan =
        _lastFullScan is { } last && last > nowUtc ? last : nowUtc;
    internal void ApplySettings(AppSettings settings) => _settings = settings;
}
