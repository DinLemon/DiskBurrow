using DiskBurrow.Core.Cleanup;
using DiskBurrow.Windows.Files;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;

namespace DiskBurrow.Windows.Cleanup;

public enum OwnerProcessState { Closed, Running, Unavailable }
public sealed record CleanupKnownDirectories(string UserProfile, string LocalAppData, string Windows,
    string ProgramFiles, string ProgramFilesX86, string ProgramData, string TempHint)
{
    public IReadOnlyList<string> UserLibraryRoots { get; init; } = [];
    public bool UserLibraryRootsVerified { get; init; } = true;
}
public interface IRuleEnvironment
{
    CleanupKnownDirectories KnownDirectories { get; }
    OwnerProcessState CheckOwnerProcess(string processName);
}
public sealed record RuleDiscoveryResult(IReadOnlyList<RuleRoot> Roots, IReadOnlyList<CleanupWarning> Warnings);
public sealed class ApprovedTempRoot
{
    public string Path { get; }
    internal ApprovedTempRoot(string path) => Path = path;
}
public sealed class RuleDiscovery
{
    private readonly IRuleEnvironment environment;
    private readonly NativeFileApi files;
    private readonly IReadOnlyList<ApprovedTempRoot> approvedTempRoots;
    private readonly Func<string, DriveType> driveType;

    public RuleDiscovery(IRuleEnvironment? environment = null, NativeFileApi? files = null,
        IReadOnlyList<ApprovedTempRoot>? approvedTempRoots = null, Func<string, DriveType>? driveType = null)
    {
        this.environment = environment ?? new WindowsRuleEnvironment();
        this.files = files ?? new NativeFileApi();
        this.approvedTempRoots = approvedTempRoots?.ToArray() ?? [];
        this.driveType = driveType ?? ReadDriveType;
    }

    public RuleDiscoveryResult Discover()
    {
        var roots = new List<RuleRoot>();
        var warnings = new List<CleanupWarning>();
        var known = environment.KnownDirectories;
        if (!CleanupPaths.TryNormalize(known.LocalAppData, out var local))
            return new([], [new("UserTemp", null, "Cleanup.UnsafeRoot")]);

        void Add(string ruleId, string path, TimeSpan? age, string? owner)
        {
            if (!CleanupPaths.TryNormalize(path, out var normalized))
            {
                warnings.Add(new(ruleId, path, "Cleanup.UnsafeRoot"));
                return;
            }
            if (!IsLocalPath(normalized))
            {
                warnings.Add(new(ruleId, path, "Cleanup.NonLocalVolume"));
                return;
            }
            if (!CleanupPaths.IsSafeDirectory(normalized, files))
            {
                warnings.Add(new(ruleId, path, "Cleanup.UnsafeRoot"));
                return;
            }
            if (!roots.Any(root => CleanupPaths.EqualsPath(root.Path, normalized))) roots.Add(new(ruleId, normalized, age, owner));
        }

        var standardTemp = Path.Combine(local, "Temp");
        Add("UserTemp", standardTemp, TimeSpan.FromDays(7), null);
        if (!CleanupPaths.TryNormalize(known.TempHint, out var hint) || !CleanupPaths.EqualsPath(standardTemp, hint))
        {
            var approved = approvedTempRoots.FirstOrDefault(root => CleanupPaths.EqualsPath(root.Path, hint));
            if (approved is not null && IsApprovedCustomTemp(hint)) Add("UserTemp", hint, TimeSpan.FromDays(7), null);
            else warnings.Add(new("UserTemp", known.TempHint,
                !known.UserLibraryRootsVerified ? "Cleanup.KnownFoldersUnavailable" :
                    !IsLocalPath(hint) ? "Cleanup.NonLocalVolume" : "Cleanup.TempApprovalRequired"));
        }
        Add("CrashDumps", Path.Combine(local, "CrashDumps"), TimeSpan.FromDays(7), null);
        AddBrowser("ChromeCache", "chrome", Path.Combine(local, "Google", "Chrome", "User Data"));
        AddBrowser("EdgeCache", "msedge", Path.Combine(local, "Microsoft", "Edge", "User Data"));
        return new(roots.ToArray(), warnings.ToArray());

        void AddBrowser(string ruleId, string process, string userData)
        {
            if (!IsLocalPath(userData))
            {
                warnings.Add(new(ruleId, userData, "Cleanup.NonLocalVolume"));
                return;
            }
            // The entire chain is inspected before enumerating profiles, even for a missing user-data folder.
            if (!CleanupPaths.IsSafeDirectory(userData, files)) return;
            OwnerProcessState state;
            try { state = environment.CheckOwnerProcess(process); }
            catch (Exception error) when (error is Win32Exception or InvalidOperationException or UnauthorizedAccessException)
            { state = OwnerProcessState.Unavailable; }
            if (state != OwnerProcessState.Closed)
            {
                warnings.Add(new(ruleId, userData, state == OwnerProcessState.Running ? "Cleanup.OwnerRunning" : "Cleanup.OwnerUnavailable"));
                return;
            }
            try
            {
                foreach (var profile in Directory.EnumerateDirectories(userData))
                {
                    var name = Path.GetFileName(profile);
                    if (name != "Default" && !(name.StartsWith("Profile ", StringComparison.Ordinal) && name.Length > 8)) continue;
                    Add(ruleId, Path.Combine(profile, "Cache", "Cache_Data"), null, process);
                }
            }
            catch (Exception error) when (error is IOException or UnauthorizedAccessException)
            { warnings.Add(new(ruleId, userData, "Cleanup.RootUnavailable")); }
        }
    }

    // Settings may persist this Path only after this explicit validation. On load they validate it again here.
    // A caller cannot turn a selected scan result or arbitrary UI path into a cleanup rule.
    public ApprovedTempRoot? ApproveCustomTempRoot(string path) =>
        CleanupPaths.TryNormalize(path, out var normalized) && IsApprovedCustomTemp(normalized) ? new(normalized) : null;

    private bool IsApprovedCustomTemp(string path)
    {
        var known = environment.KnownDirectories;
        if (!known.UserLibraryRootsVerified) return false;
        if (!CleanupPaths.TryNormalize(known.TempHint, out var hint) || !CleanupPaths.EqualsPath(path, hint)) return false;
        if (!IsLocalPath(path)) return false;
        if (Path.GetRelativePath(Path.GetPathRoot(path)!, path).Split(Path.DirectorySeparatorChar).Length < 2) return false;
        foreach (var blocked in new[] { known.Windows, known.ProgramFiles, known.ProgramFilesX86, known.ProgramData })
            if (CleanupPaths.TryNormalize(blocked, out var root) &&
                (CleanupPaths.IsWithin(path, root) || CleanupPaths.IsWithin(root, path))) return false;
        foreach (var broad in new[] { known.UserProfile, known.LocalAppData, Path.GetDirectoryName(known.LocalAppData)! }
                     .Concat(known.UserLibraryRoots))
            if (CleanupPaths.TryNormalize(broad, out var root) && CleanupPaths.IsWithin(root, path)) return false;
        return CleanupPaths.IsSafeDirectory(path, files);
    }

    // Drive letters can name mapped network shares. Validate locality before touching directory metadata.
    internal bool IsLocalPath(string path)
    {
        if (!CleanupPaths.TryNormalize(path, out var normalized)) return false;
        try { return driveType(Path.GetPathRoot(normalized)!) is DriveType.Fixed or DriveType.Removable or DriveType.Ram; }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException or Win32Exception or ArgumentException)
        { return false; }
    }

    internal OwnerProcessState CheckOwnerProcess(string processName)
    {
        try { return environment.CheckOwnerProcess(processName); }
        catch (Exception error) when (error is Win32Exception or InvalidOperationException or UnauthorizedAccessException)
        { return OwnerProcessState.Unavailable; }
    }

    private static DriveType ReadDriveType(string volumeRoot) => (DriveType)GetDriveTypeW(volumeRoot);

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern uint GetDriveTypeW(string rootPath);
}

public sealed class WindowsRuleEnvironment : IRuleEnvironment
{
    private readonly Func<string?> downloadsPath;
    public WindowsRuleEnvironment(Func<string?>? downloadsPath = null) => this.downloadsPath = downloadsPath ?? ReadDownloadsPath;
    public CleanupKnownDirectories KnownDirectories
    {
        get
        {
            string? downloads;
            try { downloads = downloadsPath(); }
            catch (Exception error) when (error is ExternalException or UnauthorizedAccessException or IOException)
            { downloads = null; }
            var libraries = new[] { Environment.SpecialFolder.DesktopDirectory, Environment.SpecialFolder.MyDocuments,
                Environment.SpecialFolder.MyPictures, Environment.SpecialFolder.MyMusic, Environment.SpecialFolder.MyVideos }
                .Select(Environment.GetFolderPath).ToArray();
            return new(
        Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        Environment.GetFolderPath(Environment.SpecialFolder.Windows),
        Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles),
        Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86),
        Environment.GetFolderPath(Environment.SpecialFolder.CommonApplicationData), Path.GetTempPath())
            {
                UserLibraryRoots = libraries.Append(downloads ?? "").Where(path => !string.IsNullOrWhiteSpace(path)).ToArray(),
                UserLibraryRootsVerified = !string.IsNullOrWhiteSpace(downloads) && Path.IsPathFullyQualified(downloads) &&
                    libraries.All(path => !string.IsNullOrWhiteSpace(path) && Path.IsPathFullyQualified(path))
            };
        }
    }

    private static string? ReadDownloadsPath()
    {
        // Current user, current redirected location, no existence probe or folder creation.
        var folder = new Guid("374DE290-123F-4565-9164-39C4925E467B");
        var comResult = CoInitializeEx(IntPtr.Zero, 0); // MTA; an existing STA apartment is also valid.
        if (comResult < 0 && comResult != unchecked((int)0x80010106)) return null;
        var resultPath = IntPtr.Zero;
        try { return SHGetKnownFolderPath(ref folder, 0x4000, IntPtr.Zero, out resultPath) == 0 ? Marshal.PtrToStringUni(resultPath) : null; }
        finally
        {
            if (resultPath != IntPtr.Zero) Marshal.FreeCoTaskMem(resultPath);
            if (comResult >= 0) CoUninitialize();
        }
    }

    [DllImport("shell32.dll", ExactSpelling = true)]
    private static extern int SHGetKnownFolderPath(ref Guid folder, uint flags, IntPtr token, out IntPtr path);
    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern int CoInitializeEx(IntPtr reserved, uint coInit);
    [DllImport("ole32.dll", ExactSpelling = true)]
    private static extern void CoUninitialize();

    public OwnerProcessState CheckOwnerProcess(string processName)
    {
        Process[] processes;
        try { processes = Process.GetProcessesByName(processName); }
        catch (Exception error) when (error is Win32Exception or InvalidOperationException or UnauthorizedAccessException)
        { return OwnerProcessState.Unavailable; }
        try { return processes.Length > 0 ? OwnerProcessState.Running : OwnerProcessState.Closed; }
        finally { foreach (var process in processes) process.Dispose(); }
    }
}

internal static class CleanupPaths
{
    public static bool TryNormalize(string? path, out string normalized)
    {
        normalized = "";
        if (string.IsNullOrWhiteSpace(path) || !Path.IsPathFullyQualified(path) || path.StartsWith(@"\\", StringComparison.Ordinal) ||
            path.Length < 3 || path[1] != ':' || path.AsSpan(2).Contains(':')) return false;
        try
        {
            normalized = Path.TrimEndingDirectorySeparator(Path.GetFullPath(path));
            return !normalized[3..].Split(Path.DirectorySeparatorChar).Any(segment => segment.EndsWith('.') || segment.EndsWith(' '));
        }
        catch (Exception error) when (error is ArgumentException or NotSupportedException or PathTooLongException) { return false; }
    }

    public static bool EqualsPath(string first, string second) => first.Equals(second, StringComparison.OrdinalIgnoreCase);
    public static bool IsWithin(string path, string root) => EqualsPath(path, root) ||
        path.StartsWith(Path.EndsInDirectorySeparator(root) ? root : root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase);

    public static bool IsSafeDirectory(string path, NativeFileApi files)
    {
        if (!TryNormalize(path, out var normalized)) return false;
        var components = new Stack<string>();
        for (var component = normalized; component is not null; component = Path.GetDirectoryName(component)) components.Push(component);
        try
        {
            while (components.TryPop(out var component))
            {
                var attributes = files.Inspect(component).Attributes;
                if (!attributes.HasFlag(FileAttributes.Directory) || attributes.HasFlag(FileAttributes.ReparsePoint) || NativeFileApi.IsCloud(attributes)) return false;
            }
            return true;
        }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException) { return false; }
    }
}
