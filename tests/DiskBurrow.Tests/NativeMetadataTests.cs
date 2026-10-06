using DiskBurrow.Windows.Files;
using Microsoft.Win32.SafeHandles;
using System.Runtime.InteropServices;

namespace DiskBurrow.Tests;

public sealed class NativeMetadataTests
{
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void SparseAndCompressedFilesUseActualPhysicalAllocation(bool compressed)
    {
        using var tree = new TempTree();
        var path = compressed ? tree.CompressedFile("compressed.bin") : tree.SparseFile("sparse.bin");
        var allocated = GetCompressedFileSize(path, out var high);
        Assert.NotEqual(uint.MaxValue, allocated);
        var expected = (long)(((ulong)high << 32) | allocated);
        var observation = new NativeFileApi().Inspect(path);
        Assert.Equal(expected, observation.AllocatedBytes);
        Assert.True(observation.AllocatedBytes < observation.LogicalBytes / 2);
    }

    [Fact]
    public void NativeMetadataUsesIdentityAllocationAndTimestamp()
    {
        using var tree = new TempTree();
        var path = tree.FileAt("file.bin", 8193);
        var expectedTime = new DateTime(2025, 1, 2, 3, 4, 5, DateTimeKind.Utc);
        File.SetLastWriteTimeUtc(path, expectedTime);
        var observation = new NativeFileApi().Inspect(path);
        Assert.NotNull(observation.Identity);
        Assert.Equal(8193, observation.LogicalBytes);
        Assert.True(observation.AllocatedBytes >= 8193);
        Assert.Equal(new DateTimeOffset(expectedTime), observation.ModifiedUtc);
        Assert.Equal(1, observation.LinkCount);
    }

    [Fact]
    public void MetadataReadSucceedsWhileContentReadIsDeniedByShareMode()
    {
        using var tree = new TempTree();
        var path = tree.FileAt("busy.bin", 1024);
        using var content = new FileStream(path, FileMode.Open, FileAccess.ReadWrite, FileShare.None);
        var observation = new NativeFileApi().Inspect(path);
        Assert.Equal(1024, observation.LogicalBytes);
        Assert.NotNull(observation.Identity);
    }

    [Fact]
    public void CloudPlaceholderDoesNotReadContent()
    {
        using var tree = new TempTree();
        var path = tree.FileAt("placeholder.bin", 1024);
        var observation = new CloudAttributes().Inspect(path);
        Assert.Null(observation.AllocatedBytes);
        Assert.Null(observation.Identity);
        Assert.True(observation.Attributes.HasFlag(FileAttributes.Offline));
    }

    [Theory]
    [InlineData(0x40000)]
    [InlineData(0x400000)]
    public void RecallAttributesAreRejectedBeforeOpeningMetadata(int cloudFlag)
    {
        using var tree = new TempTree();
        var path = tree.FileAt("recall.bin", 50);
        var observation = new RecallAttributes((FileAttributes)cloudFlag).Inspect(path);
        Assert.Null(observation.Identity);
        Assert.Null(observation.AllocatedBytes);
    }

    internal sealed class CloudAttributes : NativeFileApi
    {
        protected override FileAttributes ReadAttributes(string path) => base.ReadAttributes(path) | FileAttributes.Offline;
        public override SafeFileHandle OpenMetadata(string path) => throw new InvalidOperationException("Cloud metadata must be rejected before a handle is opened.");
    }

    private sealed class RecallAttributes(FileAttributes cloudFlag) : NativeFileApi
    {
        protected override FileAttributes ReadAttributes(string path) => base.ReadAttributes(path) | cloudFlag;
        public override SafeFileHandle OpenMetadata(string path) => throw new InvalidOperationException("Recall metadata must be rejected before a handle is opened.");
    }

    [DllImport("kernel32.dll", EntryPoint = "GetCompressedFileSizeW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern uint GetCompressedFileSize(string path, out uint high);
}
