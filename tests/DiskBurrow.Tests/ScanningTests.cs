using DiskBurrow.Core.Scanning;
using DiskBurrow.Windows.Files;
using Microsoft.Win32.SafeHandles;
using System.Diagnostics;

namespace DiskBurrow.Tests;

public sealed class ScanningTests
{
    [Fact]
    public async Task TwoHardLinksCountOnePhysicalAllocation()
    {
        using var tree = new TempTree();
        var root = tree.Root;
        var firstPath = tree.FileAt("a.bin", 8193);
        var secondPath = tree.HardLink("b.bin", firstPath);
        var native = new NativeFileApi();
        var first = native.Inspect(firstPath);
        var second = native.Inspect(secondPath);
        Assert.Equal(first.Identity, second.Identity);
        Assert.Equal(2, first.LinkCount);
        var snapshot = await new WindowsDiskScanner().ScanAsync(root, null, default);
        Assert.Equal(first.AllocatedBytes, snapshot.Directories.Single(d => d.Path == root).AllocatedBytes);
        Assert.Equal(16386, snapshot.Directories.Single(d => d.Path == root).LogicalBytes);
    }

    [Fact]
    public async Task HardLinksChargeOnlyCanonicalAncestors()
    {
        using var tree = new TempTree();
        var later = tree.DirectoryAt("z");
        var earlier = tree.DirectoryAt("a");
        var original = tree.FileAt("z\\original.bin", 8193);
        tree.HardLink("a\\canonical.bin", original);
        var bytes = new NativeFileApi().Inspect(original).AllocatedBytes;
        var snapshot = await new WindowsDiskScanner().ScanAsync(tree.Root, null, default);
        Assert.Equal(bytes, snapshot.Directories.Single(d => d.Path == earlier).AllocatedBytes);
        Assert.Equal(0L, snapshot.Directories.Single(d => d.Path == later).AllocatedBytes);
    }

    [Fact]
    public async Task JunctionCycleIsSkipped()
    {
        using var tree = new TempTree();
        tree.FileAt("data.bin", 100);
        var junction = tree.Junction("cycle", tree.Root);
        var snapshot = await new WindowsDiskScanner().ScanAsync(tree.Root, null, default);
        Assert.Equal(100, snapshot.Directories.Single(d => d.Path == tree.Root).LogicalBytes);
        Assert.Contains(snapshot.Issues, i => i.Path == junction && i.Kind == ScanIssueKind.ReparseSkipped);
        Assert.DoesNotContain(snapshot.Directories, d => d.Path.StartsWith(junction + "\\", StringComparison.OrdinalIgnoreCase));
    }

    [Theory]
    [InlineData("child")]
    [InlineData("child\\grandchild")]
    public async Task RootBelowJunctionIsSkipped(string relativeChild)
    {
        using var tree = new TempTree();
        var target = tree.DirectoryAt("target");
        tree.FileAt(Path.Combine("target", relativeChild, "marker.bin"), 13);
        var junction = tree.Junction("link", target);
        var requestedRoot = Path.Combine(junction, relativeChild);
        var snapshot = await new WindowsDiskScanner().ScanAsync(requestedRoot, null, default);
        Assert.Empty(snapshot.LargestFiles);
        var root = Assert.Single(snapshot.Directories);
        Assert.Equal(requestedRoot, root.Path);
        Assert.Equal(0, root.LogicalBytes);
        Assert.Null(root.AllocatedBytes);
        Assert.False(root.CoverageComplete);
        Assert.Contains(snapshot.Issues, i => i.Path == junction && i.Kind == ScanIssueKind.ReparseSkipped);
    }

    [Fact]
    public async Task RootBelowOfflineDirectoryIsSkipped()
    {
        using var tree = new TempTree();
        var offline = tree.DirectoryAt("offline");
        var requestedRoot = tree.DirectoryAt("offline\\child");
        tree.FileAt("offline\\child\\marker.bin", 13);
        File.SetAttributes(offline, File.GetAttributes(offline) | FileAttributes.Offline);
        Assert.True(File.GetAttributes(offline).HasFlag(FileAttributes.Offline));
        var snapshot = await new WindowsDiskScanner().ScanAsync(requestedRoot, null, default);
        Assert.Empty(snapshot.LargestFiles);
        var root = Assert.Single(snapshot.Directories);
        Assert.Equal(0, root.LogicalBytes);
        Assert.Null(root.AllocatedBytes);
        Assert.False(root.CoverageComplete);
        Assert.Contains(snapshot.Issues, i => i.Path == offline && i.Kind == ScanIssueKind.CloudSkipped);
    }

    [Fact]
    public async Task CloudPlaceholderDoesNotReadContent()
    {
        using var tree = new TempTree();
        var path = tree.FileAt("cloud.bin", 100);
        var snapshot = await new WindowsDiskScanner(new NativeMetadataTests.CloudAttributes()).ScanAsync(tree.Root, null, default);
        Assert.Contains(snapshot.Issues, i => i.Path == path && i.Kind == ScanIssueKind.CloudSkipped);
        Assert.Null(snapshot.Directories.Single(d => d.Path == tree.Root).AllocatedBytes);
        Assert.False(snapshot.Directories.Single(d => d.Path == tree.Root).CoverageComplete);
    }

    [Fact]
    public async Task DeniedSubtreeMarksAncestorsIncomplete()
    {
        using var tree = new TempTree();
        var denied = tree.DirectoryAt("parent\\denied");
        var parent = Path.GetDirectoryName(denied)!;
        var sibling = tree.DirectoryAt("accessible");
        tree.FileAt("parent\\denied\\secret.bin", 5);
        tree.FileAt("accessible\\good.bin", 7);
        tree.DenyListing(denied);
        Assert.Throws<UnauthorizedAccessException>(() => Directory.GetFileSystemEntries(denied));
        var snapshot = await new WindowsDiskScanner().ScanAsync(tree.Root, null, default);
        Assert.True(snapshot.TraversalCompleted);
        Assert.Contains(snapshot.Issues, i => i.Path == denied && i.Kind == ScanIssueKind.AccessDenied);
        Assert.False(snapshot.Directories.Single(d => d.Path == denied).CoverageComplete);
        Assert.False(snapshot.Directories.Single(d => d.Path == parent).CoverageComplete);
        Assert.False(snapshot.Directories.Single(d => d.Path == tree.Root).CoverageComplete);
        Assert.True(snapshot.Directories.Single(d => d.Path == sibling).CoverageComplete);
        Assert.Null(snapshot.Directories.Single(d => d.Path == tree.Root).AllocatedBytes);
    }

    [Fact]
    public async Task CancellationDoesNotProduceCompletedSnapshot()
    {
        using var tree = new TempTree();
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();
        var cancelledToken = cancellation.Token;
        var scanner = new WindowsDiskScanner();
        var root = tree.Root;
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => scanner.ScanAsync(root, null, cancelledToken));
    }

    [Fact]
    public async Task EmptyAndLongPathTreesScan()
    {
        using var tree = new TempTree();
        var scanner = new WindowsDiskScanner();
        var empty = await scanner.ScanAsync(tree.Root, null, default);
        Assert.Empty(empty.LargestFiles);
        Assert.Equal(0L, Assert.Single(empty.Directories).AllocatedBytes);
        var relative = string.Join("\\", Enumerable.Repeat(new string('a', 60), 5)) + "\\long.bin";
        tree.FileAt(relative, 13);
        var snapshot = await scanner.ScanAsync(tree.Root, null, default);
        Assert.Equal(13, snapshot.Directories.Single(d => d.Path == tree.Root).LogicalBytes);
        Assert.True(snapshot.Directories.All(d => d.CoverageComplete));
    }

    [Fact]
    public async Task ChangedFileIsApproximate()
    {
        using var tree = new TempTree();
        var path = tree.FileAt("changing.bin", 1);
        var snapshot = await new WindowsDiskScanner(new ChangingMetadata(path)).ScanAsync(tree.Root, null, default);
        Assert.Contains(snapshot.Issues, i => i.Path == path && i.Kind == ScanIssueKind.ChangedDuringScan);
        Assert.False(snapshot.Directories.Single(d => d.Path == tree.Root).CoverageComplete);
        Assert.Null(snapshot.Directories.Single(d => d.Path == tree.Root).AllocatedBytes);
    }

    [Fact]
    public async Task UnknownAllocationIsNotReportedAsZero()
    {
        using var tree = new TempTree();
        var path = tree.FileAt("unknown.bin", 3);
        var snapshot = await new WindowsDiskScanner(new UnknownAllocation()).ScanAsync(tree.Root, null, default);
        Assert.Equal(3, snapshot.Directories.Single(d => d.Path == tree.Root).LogicalBytes);
        Assert.Null(snapshot.Directories.Single(d => d.Path == tree.Root).AllocatedBytes);
        Assert.Contains(snapshot.Issues, i => i.Path == path && i.Kind == ScanIssueKind.MetadataUnavailable);
    }

    [Fact]
    public async Task OnlyLargestHundredFilesAreRetained()
    {
        using var tree = new TempTree();
        for (var i = 1; i <= 125; i++) tree.FileAt($"{i:D3}.bin", i);
        var snapshot = await new WindowsDiskScanner().ScanAsync(tree.Root, null, default);
        Assert.Equal(100, snapshot.LargestFiles.Count);
        Assert.Equal(125, snapshot.LargestFiles[0].LogicalBytes);
        Assert.Equal(26, snapshot.LargestFiles[^1].LogicalBytes);
        Assert.Equal(7875, snapshot.Directories.Single(d => d.Path == tree.Root).LogicalBytes);
    }

    [Fact]
    public async Task CancellationDuringTraversalDoesNotProduceCompletedSnapshot()
    {
        using var tree = new TempTree();
        tree.FileAt("first.bin", 1);
        tree.FileAt("second.bin", 2);
        using var cancellation = new CancellationTokenSource();
        var scanner = new WindowsDiskScanner(new CancelAfterObservation(cancellation));
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => scanner.ScanAsync(tree.Root, null, cancellation.Token));
    }

    [Fact]
    public async Task ProgressIsThrottledAndMetadataHandlesAreDisposed()
    {
        using var tree = new TempTree();
        for (var i = 0; i < 70; i++) tree.FileAt($"{i}.bin", 10);
        var metadata = new SlowTrackedMetadata();
        var progress = new ProgressRecorder();
        var snapshot = await new WindowsDiskScanner(metadata).ScanAsync(tree.Root, progress, default);
        Assert.True(snapshot.TraversalCompleted);
        Assert.InRange(progress.Times.Count, 3, (int)(progress.Elapsed.TotalMilliseconds / 245) + 1);
        for (var i = 1; i < progress.Times.Count; i++)
            Assert.True(progress.Times[i] - progress.Times[i - 1] >= TimeSpan.FromMilliseconds(245));
        Assert.Equal(70, metadata.Opened);
        Assert.True(metadata.LastHandle!.IsClosed);
    }

    [Fact]
    public async Task UnknownIdentityMakesPhysicalTotalUnknown()
    {
        using var tree = new TempTree();
        tree.FileAt("unidentified.bin", 15);
        var snapshot = await new WindowsDiskScanner(new UnknownIdentity()).ScanAsync(tree.Root, null, default);
        Assert.Null(snapshot.Directories.Single(d => d.Path == tree.Root).AllocatedBytes);
        Assert.False(snapshot.Directories.Single(d => d.Path == tree.Root).CoverageComplete);
        Assert.Contains(snapshot.Issues, i => i.Kind == ScanIssueKind.MetadataUnavailable);
    }

    private sealed class ChangingMetadata(string target) : NativeFileApi
    {
        public override FileObservation Inspect(string path)
        {
            var observation = base.Inspect(path);
            if (path == target) File.WriteAllBytes(path, new byte[999]);
            return observation;
        }
    }

    private sealed class UnknownAllocation : NativeFileApi
    {
        public override FileObservation Inspect(string path) => base.Inspect(path) with { AllocatedBytes = null };
    }

    private sealed class UnknownIdentity : NativeFileApi
    {
        public override FileObservation Inspect(string path) => base.Inspect(path) with { Identity = null };
    }

    private sealed class CancelAfterObservation(CancellationTokenSource cancellation) : NativeFileApi
    {
        public override FileObservation Inspect(string path)
        {
            var observation = base.Inspect(path);
            cancellation.Cancel();
            return observation;
        }
    }

    private sealed class SlowTrackedMetadata : NativeFileApi
    {
        public SafeFileHandle? LastHandle { get; private set; }
        public int Opened { get; private set; }
        public override SafeFileHandle OpenMetadata(string path)
        {
            if (LastHandle is not null) Assert.True(LastHandle.IsClosed);
            LastHandle = base.OpenMetadata(path);
            Opened++;
            Thread.Sleep(15);
            return LastHandle;
        }
    }

    private sealed class ProgressRecorder : IProgress<ScanProgress>
    {
        private readonly Stopwatch clock = Stopwatch.StartNew();
        public TimeSpan Elapsed => clock.Elapsed;
        public List<TimeSpan> Times { get; } = [];
        public void Report(ScanProgress value) => Times.Add(clock.Elapsed);
    }
}
