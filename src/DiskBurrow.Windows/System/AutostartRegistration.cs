namespace DiskBurrow.Windows.System;

public interface IRunRegistry
{
    string? Read(string name);
    void Write(string name, string value);
    void Delete(string name);
}

public sealed class AutostartRegistration
{
    public const string ValueName = "DiskBurrow";
    private readonly IRunRegistry _registry;
    private readonly string _payload;
    public string? LastUserMessage { get; private set; }
    public AutostartRegistration(IRunRegistry registry, string executablePath)
    {
        if (!Path.IsPathFullyQualified(executablePath) || executablePath.Contains('"'))
            throw new ArgumentException("An absolute executable path is required.", nameof(executablePath));
        _registry = registry;
        _payload = $"\"{Path.GetFullPath(executablePath)}\"";
    }
    public bool IsPathChanged => _registry.Read(ValueName) is { } existing &&
        !StringComparer.OrdinalIgnoreCase.Equals(existing, _payload);

    // Only the explicit Save Settings action calls Apply. Construction and inspection are read-only.
    public void Apply(bool enabled)
    {
        if (enabled) _registry.Write(ValueName, _payload);
        else if (_registry.Read(ValueName) is { } existing)
        {
            if (StringComparer.OrdinalIgnoreCase.Equals(existing, _payload)) _registry.Delete(ValueName);
            else LastUserMessage = "The existing DiskBurrow autostart entry points to another executable; it was preserved.";
        }
    }
}

public sealed class WindowsRunRegistry : IRunRegistry
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    public string? Read(string name)
    {
        using var key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(RunKey, writable: false);
        return key?.GetValue(name) as string;
    }
    public void Write(string name, string value)
    {
        using var key = Microsoft.Win32.Registry.CurrentUser.CreateSubKey(RunKey, writable: true);
        key.SetValue(name, value, Microsoft.Win32.RegistryValueKind.String);
    }
    public void Delete(string name)
    {
        using var key = Microsoft.Win32.Registry.CurrentUser.OpenSubKey(RunKey, writable: true);
        key?.DeleteValue(name, throwOnMissingValue: false);
    }
}
