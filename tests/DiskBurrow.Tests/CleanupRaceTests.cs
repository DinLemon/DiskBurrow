using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Windows.Cleanup;
using DiskBurrow.Windows.Files;
using Microsoft.Win32.SafeHandles;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace DiskBurrow.Tests;

public sealed class CleanupRaceTests
{
    [Fact]
    public async Task AncestorReplacementCannotDeleteOutsideRuleRoot()
    {
        for (var iteration = 0; iteration < 20; iteration++)
        {
            using var f = new CleanupFixture();
            var path = CleanupExecutionTests.Old(f, "parent/file");
            var parent = Path.GetDirectoryName(path)!;
            var outside = f.Tree.DirectoryAt("outside");
            var sentinel = f.Tree.FileAt("outside/file", 17);
            var plan = await CleanupExecutionTests.Preview(f);
            var api = new BeforeDirectory(Path.GetPathRoot(path)!, () =>
            {
                Directory.Move(parent, parent + ".saved");
                f.Tree.Junction(Path.GetRelativePath(f.Tree.Root, parent), outside);
            });
            var report = await CleanupExecutionTests.Run(f, plan, api);
            Assert.True(File.Exists(sentinel));
            Assert.Single(report.Items);
            Assert.All(report.Items, item => Assert.NotEqual(CleanupOutcome.Deleted, item.Outcome));
            Assert.True(File.Exists(Path.Combine(parent + ".saved", "file")));
        }
    }

    [Fact]
    public async Task LeasedAncestorsAndTargetCannotBeRenamedOrWritten()
    {
        using var f = new CleanupFixture();
        var path = CleanupExecutionTests.Old(f, "parent/file");
        var parent = Path.GetDirectoryName(path)!;
        var plan = await CleanupExecutionTests.Preview(f);
        var directoryBlocked = false;
        var api = new BeforeTarget(() =>
        {
            Assert.Throws<IOException>(() => Directory.Move(parent, parent + ".moved"));
            directoryBlocked = true;
        }, afterOpen: () =>
        {
            Assert.Throws<IOException>(() => File.Move(path, path + ".moved"));
            Assert.Throws<IOException>(() => File.WriteAllText(path, "changed"));
        });
        var report = await CleanupExecutionTests.Run(f, plan, api);
        Assert.True(directoryBlocked);
        Assert.Equal(CleanupOutcome.Deleted, Assert.Single(report.Items).Outcome);
        Assert.False(File.Exists(path));
        Assert.True(Directory.Exists(parent));
    }

    [Fact]
    public void LeasedDirectoryBlocksRealReparseMutation()
    {
        using var f = new CleanupFixture();
        var root = f.Tree.DirectoryAt("profile/AppData/Local/Temp");
        var outside = f.Tree.DirectoryAt("outside");
        var sentinel = f.Tree.FileAt("outside/sentinel", 17);
        var control = f.Tree.DirectoryAt("control");
        Assert.Equal(0, SetJunction(control, outside)); // Same real mutation succeeds without a lease.
        Directory.Delete(control);
        using (AncestorLease.OpenVerified(new("UserTemp", root, TimeSpan.FromDays(7), null), Path.Combine(root, "sentinel")))
        {
            Assert.Equal(32, SetJunction(root, outside)); // ERROR_SHARING_VIOLATION, not an ACL/privilege failure.
            Assert.Equal(5, SetJunction(root, outside, 0x80)); // Metadata-only handle cannot issue this mutation either.
            Assert.False(File.GetAttributes(root).HasFlag(FileAttributes.ReparsePoint));
            Assert.True(File.Exists(sentinel));
        }
    }

    [FileSymlinkFact]
    public async Task FileReplacedBySymlinkIsSkipped()
    {
        using var f = new CleanupFixture();
        var path = CleanupExecutionTests.Old(f, "file");
        var sentinel = f.Tree.FileAt("outside/sentinel", 17);
        var plan = await CleanupExecutionTests.Preview(f);
        var api = new BeforeDirectory(Path.GetPathRoot(path)!, () =>
        {
            File.Move(path, path + ".saved");
            File.CreateSymbolicLink(path, sentinel);
        });
        var report = await CleanupExecutionTests.Run(f, plan, api);
        Assert.True(File.Exists(sentinel));
        Assert.Equal(CleanupOutcome.SkippedPolicy, Assert.Single(report.Items).Outcome);
        Assert.True(File.GetAttributes(path).HasFlag(FileAttributes.ReparsePoint));
    }

    [Fact]
    public async Task FileReplacedByOutsideHardlinkIsSkipped()
    {
        using var f = new CleanupFixture();
        var path = CleanupExecutionTests.Old(f, "file");
        var sentinel = f.Tree.FileAt("outside/sentinel", 17);
        var plan = await CleanupExecutionTests.Preview(f);
        File.SetLastWriteTimeUtc(sentinel, plan.Candidates[0].File.ModifiedUtc.UtcDateTime);
        var api = new BeforeDirectory(Path.GetPathRoot(path)!, () =>
        {
            File.Move(path, path + ".saved");
            f.Tree.HardLink(Path.GetRelativePath(f.Tree.Root, path), sentinel);
        });
        var report = await CleanupExecutionTests.Run(f, plan, api);
        Assert.Equal(CleanupOutcome.SkippedChanged, Assert.Single(report.Items).Outcome);
        Assert.True(File.Exists(path));
        Assert.True(File.Exists(sentinel));
    }

    [Theory]
    [InlineData("mtime")]
    [InlineData("readonly")]
    [InlineData("offline")]
    public async Task MetadataIsCheckedAgainImmediatelyBeforeDisposition(string change)
    {
        using var f = new CleanupFixture();
        var path = CleanupExecutionTests.Old(f, "file");
        var plan = await CleanupExecutionTests.Preview(f);
        var api = new AfterInitialInspection(path, () =>
        {
            if (change == "mtime") File.SetLastWriteTimeUtc(path, DateTime.UtcNow);
            else File.SetAttributes(path, change == "readonly" ? FileAttributes.ReadOnly : FileAttributes.Offline);
        });
        try
        {
            var report = await CleanupExecutionTests.Run(f, plan, api);
            Assert.Equal(change == "mtime" ? CleanupOutcome.SkippedChanged : CleanupOutcome.SkippedPolicy, Assert.Single(report.Items).Outcome);
            Assert.True(File.Exists(path));
        }
        finally { File.SetAttributes(path, FileAttributes.Normal); }
    }

    [Fact]
    public async Task BrowserStateIsCheckedImmediatelyBeforeDisposition()
    {
        using var f = new CleanupFixture();
        var path = f.Tree.FileAt("profile/AppData/Local/Google/Chrome/User Data/Default/Cache/Cache_Data/data", 17);
        var plan = await CleanupExecutionTests.Preview(f);
        var api = new AfterInitialInspection(path, () => f.Environment.ProcessStates["chrome"] = OwnerProcessState.Running);
        var report = await CleanupExecutionTests.Run(f, plan, api);
        Assert.Equal(CleanupOutcome.SkippedPolicy, Assert.Single(report.Items).Outcome);
        Assert.True(File.Exists(path));
    }

    [Fact]
    public async Task CancellationAfterLeaseDoesNotDeleteTarget()
    {
        using var f = new CleanupFixture();
        var path = CleanupExecutionTests.Old(f, "file");
        var plan = await CleanupExecutionTests.Preview(f);
        using var cts = new CancellationTokenSource();
        var report = await new WindowsCleanupExecutor(f.Discovery(), new BeforeTarget(cts.Cancel))
            .ExecuteAsync(plan, plan.Candidates.Select(c => c.Id).ToHashSet(), cts.Token);
        Assert.Empty(report.Items);
        Assert.True(report.WasCancelled);
        Assert.True(File.Exists(path));
    }

    private sealed class BeforeDirectory(string target, Action before) : NativeFileApi
    {
        public override SafeFileHandle OpenCleanupDirectory(string path)
        {
            if (path.Equals(target, StringComparison.OrdinalIgnoreCase)) before();
            return base.OpenCleanupDirectory(path);
        }
    }
    private sealed class BeforeTarget(Action before, Action? afterOpen = null) : NativeFileApi
    {
        public override SafeFileHandle OpenCleanupTarget(string path)
        {
            before();
            var handle = base.OpenCleanupTarget(path);
            try { afterOpen?.Invoke(); return handle; }
            catch { handle.Dispose(); throw; }
        }
    }
    private sealed class AfterInitialInspection(string target, Action after) : NativeFileApi
    {
        private bool fired;
        public override FileObservation InspectHandle(SafeFileHandle file, string path)
        {
            var result = base.InspectHandle(file, path);
            if (!fired && path.Equals(target, StringComparison.OrdinalIgnoreCase)) { fired = true; after(); }
            return result;
        }
    }

    // Real FSCTL_SET_REPARSE_POINT, confined to the caller's verified TempTree paths.
    private static int SetJunction(string path, string target, uint access = 0x40000000)
    {
        var substitute = Encoding.Unicode.GetBytes(@"\??\" + target);
        var print = Encoding.Unicode.GetBytes(target);
        var data = new byte[16 + substitute.Length + 2 + print.Length + 2];
        BitConverter.GetBytes(0xA0000003u).CopyTo(data, 0);
        BitConverter.GetBytes((ushort)(data.Length - 8)).CopyTo(data, 4);
        BitConverter.GetBytes((ushort)substitute.Length).CopyTo(data, 10);
        BitConverter.GetBytes((ushort)(substitute.Length + 2)).CopyTo(data, 12);
        BitConverter.GetBytes((ushort)print.Length).CopyTo(data, 14);
        substitute.CopyTo(data, 16);
        print.CopyTo(data, 16 + substitute.Length + 2);
        using var handle = CreateFile(path, access, 7, IntPtr.Zero, 3, 0x02200000, IntPtr.Zero);
        if (handle.IsInvalid) return Marshal.GetLastWin32Error();
        return DeviceIoControl(handle, 0x900A4, data, data.Length, IntPtr.Zero, 0, out _, IntPtr.Zero) ? 0 : Marshal.GetLastWin32Error();
    }
    [DllImport("kernel32.dll", EntryPoint = "CreateFileW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFile(string path, uint access, uint share, IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool DeviceIoControl(SafeFileHandle handle, uint code, byte[] data, int size, IntPtr output, int outputSize, out int returned, IntPtr overlapped);
}

internal sealed class FileSymlinkFactAttribute : FactAttribute
{
    public FileSymlinkFactAttribute()
    {
        using var tree = new TempTree();
        var target = tree.FileAt("target", 1);
        try { File.CreateSymbolicLink(Path.Combine(tree.Root, "link"), target); }
        catch (IOException error) when ((error.HResult & 0xFFFF) == 1314)
        { Skip = "File symlink creation unavailable: Windows error 1314; no privilege or Developer Mode changes are allowed."; }
    }
}
