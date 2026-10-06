using DiskBurrow.Core.Monitoring;
using DiskBurrow.Windows.Files;
using System.ComponentModel;
using System.Runtime.InteropServices;

namespace DiskBurrow.Windows.System;

public sealed class WindowsEnvironment : IMonitoringEnvironment
{
    public string SystemRoot { get; } = Path.GetPathRoot(Environment.GetFolderPath(Environment.SpecialFolder.Windows))
        ?? throw new IOException("Windows system volume is unavailable.");

    public bool IsOnBattery
    {
        get
        {
            if (!GetSystemPowerStatus(out var status)) throw new Win32Exception(Marshal.GetLastWin32Error());
            // Unknown AC status is conservative: defer costly background scans.
            return status.AcLineStatus != 1;
        }
    }

    public long GetFreeBytes(string root) => GetVolumeSpace(root).FreeBytes;
    public VolumeSpace GetVolumeSpace(string root)
    {
        if (!IsLocalRoot(root)) throw new ArgumentException("A confirmed local directory is required.", nameof(root));
        if (!GetDiskFreeSpaceEx(Path.GetFullPath(root), out _, out var total, out var free))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        return new(checked((long)total), checked((long)free));
    }

    public bool IsLocalRoot(string root)
    {
        try
        {
            if (!Path.IsPathFullyQualified(root) || root.StartsWith(@"\\", StringComparison.Ordinal)) return false;
            var path = Path.GetFullPath(root);
            if (!Directory.Exists(path)) return false;
            // Walk ancestors: a junction can redirect an apparently local path to a remote volume.
            var native = new NativeFileApi();
            for (var directory = new DirectoryInfo(path); directory is not null; directory = directory.Parent)
            {
                if ((directory.Attributes & FileAttributes.ReparsePoint) != 0) return false;
                using var handle = native.OpenMetadata(directory.FullName);
                var final = native.GetFinalPath(handle);
                var volumeRoot = Path.GetPathRoot(final);
                if (volumeRoot is null || final.StartsWith(@"UNC\", StringComparison.OrdinalIgnoreCase) ||
                    final.StartsWith(@"\\", StringComparison.Ordinal) ||
                    new DriveInfo(volumeRoot).DriveType is not (DriveType.Fixed or DriveType.Removable or DriveType.Ram)) return false;
            }
            return true;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or ArgumentException or Win32Exception)
        {
            return false;
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct PowerStatus
    {
        public byte AcLineStatus, BatteryFlag, BatteryLifePercent, SystemStatusFlag;
        public uint BatteryLifeTime, BatteryFullLifeTime;
    }
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetSystemPowerStatus(out PowerStatus status);
    [DllImport("kernel32.dll", EntryPoint = "GetDiskFreeSpaceExW", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetDiskFreeSpaceEx(string path, out ulong available, out ulong total, out ulong free);
}
