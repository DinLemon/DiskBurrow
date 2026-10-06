using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Principal;

namespace DiskBurrow.Tests;

internal sealed class TempTree : IDisposable
{
    private readonly List<string> junctions = [];
    private readonly List<(string Path, string Sddl)> permissions = [];
    private readonly string fixtureParent = LocateFixtureParent();
    public string Root { get; }

    public TempTree()
    {
        Root = Path.Combine(fixtureParent, Guid.NewGuid().ToString("N"));
        EnsureInside(Root, fixtureParent);
        Directory.CreateDirectory(Root);
    }

    public string DirectoryAt(string relative)
    {
        var path = Resolve(relative);
        Directory.CreateDirectory(path);
        return path;
    }

    public string FileAt(string relative, int bytes)
    {
        var path = Resolve(relative);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllBytes(path, new byte[bytes]);
        return path;
    }

    public string HardLink(string relative, string existing)
    {
        var path = Resolve(relative);
        EnsureInside(existing, Root);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        if (!CreateHardLink(path, existing, IntPtr.Zero)) throw new Win32Exception(Marshal.GetLastWin32Error());
        return path;
    }

    public string SparseFile(string relative)
    {
        var path = FileAt(relative, 0);
        using var file = new FileStream(path, FileMode.Open, FileAccess.Write, FileShare.ReadWrite);
        if (!DeviceIoControl(file.SafeFileHandle, 0x900C4, IntPtr.Zero, 0, IntPtr.Zero, 0, out _, IntPtr.Zero))
            throw new Win32Exception(Marshal.GetLastWin32Error());
        file.SetLength(32 * 1024 * 1024);
        file.WriteByte(1);
        return path;
    }

    public string CompressedFile(string relative)
    {
        var path = FileAt(relative, 1024 * 1024);
        var start = new ProcessStartInfo("compact.exe")
        {
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true
        };
        start.ArgumentList.Add("/C");
        start.ArgumentList.Add("/I");
        start.ArgumentList.Add(path);
        using var process = Process.Start(start)!;
        var output = process.StandardOutput.ReadToEnd() + process.StandardError.ReadToEnd();
        process.WaitForExit();
        if (process.ExitCode != 0 || !File.GetAttributes(path).HasFlag(FileAttributes.Compressed)) throw new IOException(output);
        return path;
    }

    public string Junction(string relative, string target)
    {
        var path = Resolve(relative);
        EnsureInside(target, Root);
        using var process = Process.Start(new ProcessStartInfo("cmd.exe")
        {
            Arguments = $"/c mklink /J \"{path}\" \"{target}\"",
            CreateNoWindow = true, UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true
        })!;
        var output = process.StandardOutput.ReadToEnd() + process.StandardError.ReadToEnd();
        process.WaitForExit();
        if (process.ExitCode != 0) throw new IOException(output);
        junctions.Add(path);
        return path;
    }

    public void DenyListing(string path)
    {
        EnsureInside(path, Root);
        var directory = new DirectoryInfo(path);
        var original = directory.GetAccessControl();
        var denied = directory.GetAccessControl();
        denied.AddAccessRule(new FileSystemAccessRule(WindowsIdentity.GetCurrent().User!,
            FileSystemRights.ListDirectory, AccessControlType.Deny));
        permissions.Add((path, original.GetSecurityDescriptorSddlForm(AccessControlSections.Access)));
        directory.SetAccessControl(denied);
    }

    private string Resolve(string relative)
    {
        var path = Path.GetFullPath(Path.Combine(Root, relative));
        EnsureInside(path, Root);
        return path;
    }

    private static void EnsureInside(string path, string boundary)
    {
        var full = Path.GetFullPath(path);
        var root = Path.GetFullPath(boundary).TrimEnd(Path.DirectorySeparatorChar);
        if (!full.Equals(root, StringComparison.OrdinalIgnoreCase) &&
            !full.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Fixture path must remain under its temporary root.");
    }

    private static string LocateFixtureParent()
    {
        for (var directory = new DirectoryInfo(AppContext.BaseDirectory); directory is not null; directory = directory.Parent)
            if (File.Exists(Path.Combine(directory.FullName, "DiskBurrow.slnx")))
                return Path.GetFullPath(Path.Combine(directory.FullName, "work", "fixtures"));
        throw new InvalidOperationException("Tests must run under the repository; fixture location could not be verified.");
    }

    public void Dispose()
    {
        EnsureInside(Root, fixtureParent);
        foreach (var (path, sddl) in permissions)
        {
            var security = new DirectorySecurity();
            security.SetSecurityDescriptorSddlForm(sddl, AccessControlSections.Access);
            new DirectoryInfo(path).SetAccessControl(security);
        }
        foreach (var path in junctions) if (Directory.Exists(path)) Directory.Delete(path);
        if (Directory.Exists(Root)) Directory.Delete(Root, true);
    }

    [DllImport("kernel32.dll", EntryPoint = "CreateHardLinkW", CharSet = CharSet.Unicode, SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CreateHardLink(string newName, string existingName, IntPtr security);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DeviceIoControl(Microsoft.Win32.SafeHandles.SafeFileHandle handle, uint code,
        IntPtr input, uint inputSize, IntPtr output, uint outputSize, out uint returned, IntPtr overlapped);
}
