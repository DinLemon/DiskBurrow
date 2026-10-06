namespace DiskBurrow.Core.Monitoring;

public sealed record AppSettings
{
    public int IntervalHours { get; set; } = 6;
    public TimeSpan InitialDelay { get; set; } = TimeSpan.FromMinutes(5);
    public TimeSpan FreeSpaceInterval { get; set; } = TimeSpan.FromMinutes(5);
    public long LowSpaceBytes { get; set; } = 15L * 1024 * 1024 * 1024;
    public long GrowthBytes { get; set; } = 5L * 1024 * 1024 * 1024;
    public string Language { get; set; } = "ru";
    public string[] ExcludedPaths { get; set; } = [];
    public bool Paused { get; set; }
    public bool AllowOnBattery { get; set; }
    public bool Autostart { get; set; }
    public string? ApprovedCustomTempPath { get; set; }
    public void Validate()
    {
        if (IntervalHours is not (1 or 6 or 12 or 24) || LowSpaceBytes < 0 || GrowthBytes <= 0 ||
            InitialDelay != TimeSpan.FromMinutes(5) || FreeSpaceInterval != TimeSpan.FromMinutes(5) ||
            Language is not ("ru" or "en") || ExcludedPaths is null ||
            ExcludedPaths.Any(p => string.IsNullOrWhiteSpace(p) || !Path.IsPathFullyQualified(p)) ||
            (ApprovedCustomTempPath is not null && (string.IsNullOrWhiteSpace(ApprovedCustomTempPath) ||
                !Path.IsPathFullyQualified(ApprovedCustomTempPath))))
            throw new ArgumentException("Invalid DiskBurrow settings.");
    }
}
