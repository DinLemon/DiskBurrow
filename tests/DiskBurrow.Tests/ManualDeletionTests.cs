using DiskBurrow.Core.Cleanup;
using DiskBurrow.Windows.Cleanup;
using DiskBurrow.Windows.Files;
using Microsoft.Win32.SafeHandles;
using System.Runtime.InteropServices;
using System.Text;

namespace DiskBurrow.Tests;

public sealed class ManualDeletionTests
{
    private static string At(TempTree tree, string relative) => Path.GetFullPath(Path.Combine(tree.Root, relative));
    private static WindowsManualDeletionService Service(TempTree tree, NativeFileApi? files = null) =>
        new(new(tree.DirectoryAt("profile"), tree.DirectoryAt("profile/AppData/Local"), tree.DirectoryAt("Windows"),
            tree.DirectoryAt("Program Files"), tree.DirectoryAt("Program Files x86"), tree.DirectoryAt("ProgramData"), tree.Root),
            tree.DirectoryAt("application"), tree.DirectoryAt("data"), files);

    [Fact]
    public async Task UnresolvedUserLibraryRootsBlockManualPreview()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/file", 17);
        var known = new CleanupKnownDirectories(tree.DirectoryAt("profile"), tree.DirectoryAt("profile/AppData/Local"),
            tree.DirectoryAt("Windows"), tree.DirectoryAt("Program Files"), tree.DirectoryAt("Program Files x86"),
            tree.DirectoryAt("ProgramData"), tree.Root) { UserLibraryRootsVerified = false };
        var service = new WindowsManualDeletionService(known, tree.DirectoryAt("application"), tree.DirectoryAt("data"));
        var plan = await service.PreviewAsync([file], default);
        Assert.False(plan.CanExecute);
        Assert.Contains(plan.Warnings, warning => warning.ReasonKey == "Manual.ProtectedPath");
        Assert.True(File.Exists(file));
        await Assert.ThrowsAsync<InvalidOperationException>(() => service.ExecuteConfirmedAsync(plan.Id, true, default));
    }

    [Fact]
    public async Task SharedAncestorIdentityChangeBetweenSelectedRootsBlocksWholePreview()
    {
        using var tree = new TempTree();
        var first = tree.FileAt("selected/a", 17);
        var second = tree.FileAt("selected/b", 19);
        var common = Path.GetDirectoryName(first)!;
        var api = new ChangedSharedAncestor(common);
        var service = Service(tree, api);
        var plan = await service.PreviewAsync([first, second], default);
        Assert.True(api.Changed);
        Assert.False(plan.CanExecute);
        Assert.Contains(plan.Warnings, warning => warning.ReasonKey == "Manual.Changed");
        Assert.True(File.Exists(first));
        Assert.True(File.Exists(second));
        await Assert.ThrowsAsync<InvalidOperationException>(() => service.ExecuteConfirmedAsync(plan.Id, true, default));
    }

    [Fact]
    public async Task NativeSharedAncestorReplacementBetweenRootsIsRejected()
    {
        using var tree = new TempTree();
        var first = tree.FileAt("selected/a", 17);
        var second = tree.FileAt("selected/b", 19);
        var common = Path.GetDirectoryName(first)!;
        var moved = At(tree, "moved");
        var api = new BeforeSecondRoot(Path.GetPathRoot(first)!, () =>
        {
            Directory.Move(common, moved);
            Directory.CreateDirectory(common);
            File.Move(Path.Combine(moved, "a"), first);
            File.Move(Path.Combine(moved, "b"), second);
        });
        var service = Service(tree, api);
        var plan = await service.PreviewAsync([first, second], default);
        Assert.True(api.Replaced);
        Assert.False(plan.CanExecute);
        Assert.Contains(plan.Warnings, warning => warning.ReasonKey == "Manual.Changed");
        Assert.True(File.Exists(first));
        Assert.True(File.Exists(second));
    }

    [Fact]
    public async Task PreviewIsReadOnlyNormalizesSelectionsAndDeletesOnlyReviewedTree()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/sub/file", 17);
        var sibling = tree.FileAt("unselected/file", 19);
        var service = Service(tree);
        var root = Path.GetDirectoryName(Path.GetDirectoryName(file))!;
        var plan = await service.PreviewAsync([root, file, root.ToUpperInvariant()], default);
        Assert.True(plan.CanExecute);
        Assert.Single(plan.Roots);
        Assert.Equal(3, plan.Entries.Count);
        Assert.Equal(17, plan.EstimatedDataBytes);
        Assert.True(File.Exists(file));
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.Equal(3, report.Items.Count);
        Assert.All(report.Items, item => Assert.Equal(CleanupOutcome.Deleted, item.Outcome));
        Assert.All(report.Items, item => Assert.NotNull(item.Audit));
        Assert.False(Directory.Exists(root));
        Assert.True(File.Exists(sibling));
        await Assert.ThrowsAsync<InvalidOperationException>(() => service.ExecuteConfirmedAsync(plan.Id, true, default));
    }

    [Fact]
    public async Task ChangedInventoryPreventsRootDeletionAndNewFilesSurvive()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/file", 17);
        var service = Service(tree);
        var root = Path.GetDirectoryName(file)!;
        var plan = await service.PreviewAsync([root], default);
        var arrived = tree.FileAt("selected/new", 20);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.All(report.Items, item => Assert.Equal(CleanupOutcome.SkippedChanged, item.Outcome));
        Assert.True(File.Exists(file));
        Assert.True(File.Exists(arrived));
    }

    [Fact]
    public async Task ChangedFileAndBusyFileArePreserved()
    {
        using var tree = new TempTree();
        var changed = tree.FileAt("changed", 17);
        var busy = tree.FileAt("busy", 19);
        var service = Service(tree);
        var plan = await service.PreviewAsync([changed, busy], default);
        File.WriteAllText(changed, "different");
        using var stream = new FileStream(busy, FileMode.Open, FileAccess.Read, FileShare.Read);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.Contains(report.Items, i => i.Outcome == CleanupOutcome.SkippedChanged);
        Assert.Contains(report.Items, i => i.Outcome == CleanupOutcome.SkippedBusy);
        Assert.True(File.Exists(changed));
        Assert.True(File.Exists(busy));
    }

    [Fact]
    public async Task ProtectedContainersAndNoncanonicalPathsCannotProduceExecutablePlan()
    {
        using var tree = new TempTree();
        var service = Service(tree);
        foreach (var path in new[] { Path.GetPathRoot(tree.Root)!, At(tree, "profile"), At(tree, "profile/AppData"),
                     At(tree, "profile/AppData/Local"), At(tree, "Windows"), At(tree, "application"),
                     At(tree, "data"), tree.Root + "\\..\\selected", @"\\server\share\folder", tree.Root + ":stream" })
        {
            var plan = await service.PreviewAsync([path], default);
            Assert.False(plan.CanExecute);
            Assert.NotEmpty(plan.Warnings);
            await Assert.ThrowsAsync<InvalidOperationException>(() => service.ExecuteConfirmedAsync(plan.Id, true, default));
        }
    }

    [Fact]
    public async Task OrdinaryAppDataFolderIsAllowedAndHardlinkOtherNameSurvives()
    {
        using var tree = new TempTree();
        var original = tree.FileAt("profile/AppData/Local/MyApp/file", 17);
        var alias = tree.HardLink("elsewhere/file", original);
        var service = Service(tree);
        var plan = await service.PreviewAsync([Path.GetDirectoryName(original)!], default);
        Assert.True(plan.CanExecute);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.All(report.Items, i => Assert.Equal(CleanupOutcome.Deleted, i.Outcome));
        Assert.True(File.Exists(alias));
        Assert.Equal(2, report.Items.Single(i => i.Audit!.Path == original).Audit!.LinkCount);
    }

    [Fact]
    public async Task ReparseDescendantAndReadOnlyFileMakeWholePreviewNonExecutable()
    {
        using var tree = new TempTree();
        tree.FileAt("outside/sentinel", 17);
        tree.DirectoryAt("selected");
        tree.Junction("selected/junction", At(tree, "outside"));
        var service = Service(tree);
        var unsafePlan = await service.PreviewAsync([At(tree, "selected")], default);
        Assert.False(unsafePlan.CanExecute);
        Assert.Equal(At(tree, "selected/junction"), Assert.Single(unsafePlan.Warnings).Path);
        var readOnly = tree.FileAt("readonly", 17);
        File.SetAttributes(readOnly, FileAttributes.ReadOnly);
        try { Assert.False((await service.PreviewAsync([readOnly], default)).CanExecute); }
        finally { File.SetAttributes(readOnly, FileAttributes.Normal); }
    }

    [Fact]
    public async Task CancellationReturnsAuditedUnattemptedItemsWithoutDeletingAnything()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/file", 17);
        var service = Service(tree);
        var plan = await service.PreviewAsync([Path.GetDirectoryName(file)!], default);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, new CancellationToken(true));
        Assert.True(report.WasCancelled);
        Assert.Equal(plan.Entries.Count, report.Items.Count);
        Assert.All(report.Items, i => Assert.NotNull(i.Audit));
        Assert.True(File.Exists(file));
    }

    [Fact]
    public async Task NewChildDuringDeletionIsNeverDeletedAndPreventsDirectoryDisposition()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/file", 17);
        var arrived = At(tree, "selected/new");
        var api = new BeforeDisposition(file, () => File.WriteAllText(arrived, "new"));
        var service = Service(tree, api);
        var plan = await service.PreviewAsync([Path.GetDirectoryName(file)!], default);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.Contains(report.Items, i => i.Outcome == CleanupOutcome.Deleted && i.Audit!.Path == file);
        Assert.Contains(report.Items, i => i.Outcome != CleanupOutcome.Deleted && i.Audit!.Path == Path.GetDirectoryName(file));
        Assert.True(File.Exists(arrived));
    }

    [Fact]
    public async Task DeclinedConfirmationDoesNotConsumePlanAndCollectionsCannotBeMutated()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected", 17);
        var service = Service(tree);
        var plan = await service.PreviewAsync([file], default);
        await Assert.ThrowsAsync<InvalidOperationException>(() => service.ExecuteConfirmedAsync(plan.Id, false, default));
        Assert.Throws<NotSupportedException>(() => ((IList<string>)plan.Roots)[0] = At(tree, "outside"));
        Assert.Throws<NotSupportedException>(() => ((IList<ManualDeleteEntry>)plan.Entries).Clear());
        Assert.True(File.Exists(file));
        Assert.Equal(CleanupOutcome.Deleted, Assert.Single((await service.ExecuteConfirmedAsync(plan.Id, true, default)).Items).Outcome);
    }

    [Fact]
    public async Task KernelRejectsDirectoryChildArrivingAfterFinalEmptyCheck()
    {
        using var tree = new TempTree();
        var root = tree.DirectoryAt("selected");
        var arrived = At(tree, "selected/new");
        var service = Service(tree, new BeforeDisposition(root, () => File.WriteAllText(arrived, "new")));
        var plan = await service.PreviewAsync([root], default);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.NotEqual(CleanupOutcome.Deleted, Assert.Single(report.Items).Outcome);
        Assert.True(File.Exists(arrived));
    }

    [Fact]
    public async Task AncestorReplacedByJunctionCannotDeleteExternalSentinel()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/file", 17);
        var sentinel = tree.FileAt("outside/file", 17);
        var root = Path.GetDirectoryName(file)!;
        var api = new BeforeDirectory();
        var service = Service(tree, api);
        var plan = await service.PreviewAsync([root], default);
        api.Action = () =>
        {
            Directory.Move(root, root + ".saved");
            tree.Junction("selected", At(tree, "outside"));
        };
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.All(report.Items, item => Assert.NotEqual(CleanupOutcome.Deleted, item.Outcome));
        Assert.True(File.Exists(sentinel));
        Assert.True(File.Exists(Path.Combine(root + ".saved", "file")));
    }

    [Fact]
    public async Task TargetAndParentsCannotBeRenamedOrWrittenDuringFinalDisposition()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/file", 17);
        var root = Path.GetDirectoryName(file)!;
        var service = Service(tree, new BeforeDisposition(file, () =>
        {
            Assert.Throws<IOException>(() => Directory.Move(root, root + ".saved"));
            Assert.Throws<IOException>(() => File.Move(file, file + ".saved"));
            Assert.Throws<IOException>(() => File.WriteAllText(file, "changed"));
        }));
        var plan = await service.PreviewAsync([root], default);
        Assert.All((await service.ExecuteConfirmedAsync(plan.Id, true, default)).Items,
            item => Assert.Equal(CleanupOutcome.Deleted, item.Outcome));
    }

    [Fact]
    public async Task CancellationAfterFirstDeletionRetainsRemainingFilesAndRecordsPartialAudit()
    {
        using var tree = new TempTree();
        var first = tree.FileAt("selected/long-name", 17);
        var second = tree.FileAt("selected/file", 19);
        using var cancellation = new CancellationTokenSource();
        var service = Service(tree, new BeforeDisposition(first, cancellation.Cancel));
        var plan = await service.PreviewAsync([Path.GetDirectoryName(first)!], default);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, cancellation.Token);
        Assert.True(report.WasCancelled);
        Assert.Contains(report.Items, item => item.Outcome == CleanupOutcome.Deleted && item.Audit!.Path == first);
        Assert.True(File.Exists(second));
        Assert.Equal(plan.Entries.Count, report.Items.Count);
        Assert.False(report.FreeSpaceDeltaAvailable);
        await Assert.ThrowsAsync<InvalidOperationException>(() => service.ExecuteConfirmedAsync(plan.Id, true, default));
    }

    [Fact]
    public async Task CloudMetadataIsRejectedWithoutInspectingDescendants()
    {
        using var tree = new TempTree();
        var root = tree.DirectoryAt("selected");
        tree.FileAt("selected/file", 17);
        var api = new CloudDirectory(root);
        var service = Service(tree, api);
        var plan = await service.PreviewAsync([root], default);
        Assert.False(plan.CanExecute);
        Assert.DoesNotContain(api.Inspected, path => path.Equals(Path.Combine(root, "file"), StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public async Task MissingOrDeniedDescendantBlocksEntirePlanWithExactWarningPath()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected/sub/file", 17);
        var service = Service(tree, new DeniedMetadata(Path.GetDirectoryName(file)!));
        var plan = await service.PreviewAsync([At(tree, "selected")], default);
        Assert.False(plan.CanExecute);
        Assert.Equal(Path.GetDirectoryName(file), Assert.Single(plan.Warnings).Path);
        Assert.True(File.Exists(file));
        Assert.False((await service.PreviewAsync([At(tree, "missing")], default)).CanExecute);
    }

    [Fact]
    public async Task FileIdentityReplacementEvenWithPreservedLengthAndTimestampIsNotDeleted()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected", 17);
        var service = Service(tree);
        var plan = await service.PreviewAsync([file], default);
        File.Move(file, file + ".old");
        File.WriteAllBytes(file, new byte[17]);
        File.SetLastWriteTimeUtc(file, plan.Entries[0].File.ModifiedUtc.UtcDateTime);
        var report = await service.ExecuteConfirmedAsync(plan.Id, true, default);
        Assert.Equal(CleanupOutcome.SkippedChanged, Assert.Single(report.Items).Outcome);
        Assert.True(File.Exists(file));
        Assert.True(File.Exists(file + ".old"));
    }

    private sealed class DeniedMetadata(string denied) : NativeFileApi
    {
        public override SafeFileHandle OpenMetadata(string path) => path.Equals(denied, StringComparison.OrdinalIgnoreCase)
            ? throw new UnauthorizedAccessException("Fixture inaccessible") : base.OpenMetadata(path);
    }

    [Fact]
    public async Task StartingCancelledNewPreviewInvalidatesPreviouslyIssuedPlan()
    {
        using var tree = new TempTree();
        var file = tree.FileAt("selected", 17);
        var service = Service(tree);
        var previous = await service.PreviewAsync([file], default);
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => service.PreviewAsync([file], new CancellationToken(true)));
        await Assert.ThrowsAsync<InvalidOperationException>(() => service.ExecuteConfirmedAsync(previous.Id, true, default));
        Assert.True(File.Exists(file));
    }

    [Fact]
    public async Task DirectoryDeletionHandleBlocksActualReparseMutation()
    {
        using var tree = new TempTree();
        var root = tree.DirectoryAt("selected");
        var outside = tree.DirectoryAt("outside");
        var sentinel = tree.FileAt("outside/file", 17);
        var control = tree.DirectoryAt("control");
        Assert.Equal(0, SetJunction(control, outside));
        Directory.Delete(control); // Remove only the owned junction itself.
        var service = Service(tree, new BeforeDisposition(root, () =>
        {
            Assert.Equal(32, SetJunction(root, outside));
            Assert.Equal(5, SetJunction(root, outside, 0x80));
        }));
        var plan = await service.PreviewAsync([root], default);
        Assert.Equal(CleanupOutcome.Deleted, Assert.Single((await service.ExecuteConfirmedAsync(plan.Id, true, default)).Items).Outcome);
        Assert.True(File.Exists(sentinel));
    }

    // Real FSCTL_SET_REPARSE_POINT, called only with verified own TempTree paths.
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

    private sealed class BeforeSecondRoot(string volumeRoot, Action replace) : NativeFileApi
    {
        private int opens;
        public bool Replaced { get; private set; }
        public override SafeFileHandle OpenCleanupDirectory(string path)
        {
            if (path.Equals(volumeRoot, StringComparison.OrdinalIgnoreCase) && ++opens == 3)
            { replace(); Replaced = true; }
            return base.OpenCleanupDirectory(path);
        }
    }

    private sealed class ChangedSharedAncestor(string common) : NativeFileApi
    {
        private int opens;
        public bool Changed { get; private set; }
        public override SafeFileHandle OpenCleanupDirectory(string path)
        {
            if (path.Equals(common, StringComparison.OrdinalIgnoreCase) && ++opens == 3)
                Changed = true;
            return base.OpenCleanupDirectory(path);
        }
        public override DiskBurrow.Core.Scanning.FileObservation InspectHandle(SafeFileHandle handle, string path)
        {
            var observed = base.InspectHandle(handle, path);
            return Changed && path.Equals(common, StringComparison.OrdinalIgnoreCase)
                ? observed with { Identity = observed.Identity! with { FileId = "changed-shared-ancestor" } } : observed;
        }
    }

    private sealed class CloudDirectory(string cloud) : NativeFileApi
    {
        public List<string> Inspected { get; } = [];
        public override DiskBurrow.Core.Scanning.FileObservation InspectHandle(SafeFileHandle handle, string path)
        {
            Inspected.Add(path);
            var observation = base.InspectHandle(handle, path);
            return path.Equals(cloud, StringComparison.OrdinalIgnoreCase)
                ? observation with { Attributes = observation.Attributes | FileAttributes.Offline } : observation;
        }
    }

    private sealed class BeforeDirectory : NativeFileApi
    {
        public Action? Action { get; set; }
        public override SafeFileHandle OpenCleanupDirectory(string path)
        {
            var action = Action;
            Action = null;
            action?.Invoke();
            return base.OpenCleanupDirectory(path);
        }
    }

    private sealed class BeforeDisposition(string path, Action action) : NativeFileApi
    {
        private bool fired;
        public override void MarkForDeletion(SafeFileHandle handle)
        {
            if (!fired && GetFinalPath(handle).Equals(path, StringComparison.OrdinalIgnoreCase)) { fired = true; action(); }
            base.MarkForDeletion(handle);
        }
    }
}
