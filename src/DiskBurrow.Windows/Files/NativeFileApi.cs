using DiskBurrow.Core.Scanning;
using Microsoft.Win32.SafeHandles;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

namespace DiskBurrow.Windows.Files;

public class NativeFileApi
{
    // Deny data-write/delete sharing. Attribute writes are NOT prevented by share flags.
    // FILE_LIST_DIRECTORY is needed: attribute-only opens do not enforce the sharing restrictions.
    public virtual SafeFileHandle OpenCleanupDirectory(string path) => OpenCleanup(path, ReadAttributesAccess | 1);
    public virtual SafeFileHandle OpenCleanupTarget(string path) => OpenCleanup(path, 0x00010000 | ReadAttributesAccess);

    private static SafeFileHandle OpenCleanup(string path, uint access)
    {
        var handle = CreateFile(ExtendedPath(path), access, 1, IntPtr.Zero, OpenExisting, MetadataFlags, IntPtr.Zero);
        if (!handle.IsInvalid) return handle;
        var error = Marshal.GetLastWin32Error();
        handle.Dispose();
        throw MetadataError(path, error);
    }

    public virtual string GetFinalPath(SafeFileHandle file)
    {
        var buffer = new StringBuilder(32768);
        var length = GetFinalPathNameByHandle(file, buffer, (uint)buffer.Capacity, 0);
        if (length == 0) throw MetadataError("handle", Marshal.GetLastWin32Error());
        if (length >= buffer.Capacity) throw new IOException("Final path exceeds supported length.");
        var path = buffer.ToString();
        return path.StartsWith(@"\\?\", StringComparison.Ordinal) ? path[4..] : path;
    }

    public virtual void MarkForDeletion(SafeFileHandle file)
    {
        var information = new DispositionInfo { DeleteFile = 1 };
        if (!SetDisposition(file, 4, ref information, (uint)Marshal.SizeOf<DispositionInfo>()))
            throw MetadataError("handle", Marshal.GetLastWin32Error());
    }
    private const uint ReadAttributesAccess = 0x80;
    private const uint ShareReadWriteDelete = 7;
    private const uint OpenExisting = 3;
    // Metadata only; never recall remote content, and never resolve the final reparse point.
    private const uint MetadataFlags = 0x02000000 | 0x00200000 | 0x00100000;
    private const FileAttributes CloudAttributes = FileAttributes.Offline | (FileAttributes)0x40000 | (FileAttributes)0x400000;

    public static bool IsCloud(FileAttributes attributes) => (attributes & CloudAttributes) != 0;
    protected virtual FileAttributes ReadAttributes(string path) => File.GetAttributes(path);

    public virtual SafeFileHandle OpenMetadata(string path)
    {
        var handle = CreateFile(ExtendedPath(path), ReadAttributesAccess, ShareReadWriteDelete,
            IntPtr.Zero, OpenExisting, MetadataFlags, IntPtr.Zero);
        if (!handle.IsInvalid) return handle;
        var error = Marshal.GetLastWin32Error();
        handle.Dispose();
        throw MetadataError(path, error);
    }

    public virtual FileObservation Inspect(string path)
    {
        var attributes = ReadAttributes(path);
        if (IsCloud(attributes) || attributes.HasFlag(FileAttributes.ReparsePoint))
            return new(path, null, 0, null, DateTimeOffset.MinValue, 0, attributes);

        using var handle = OpenMetadata(path);
        return InspectHandle(handle, path);
    }

    public virtual FileObservation InspectHandle(SafeFileHandle handle, string path)
    {
        if (!GetBasic(handle, 0, out var basic, (uint)Marshal.SizeOf<BasicInfo>()))
            throw MetadataError(path, Marshal.GetLastWin32Error());
        var attributes = (FileAttributes)basic.Attributes;
        if (IsCloud(attributes) || attributes.HasFlag(FileAttributes.ReparsePoint))
            return new(path, null, 0, null, DateTimeOffset.MinValue, 0, attributes);
        if (!GetStandard(handle, 1, out var standard, (uint)Marshal.SizeOf<StandardInfo>()))
            throw MetadataError(path, Marshal.GetLastWin32Error());

        FileIdentity? identity = null;
        if (GetId(handle, 18, out var id, (uint)Marshal.SizeOf<IdInfo>()))
            identity = new(id.VolumeSerialNumber, $"{id.FileIdLow:X16}{id.FileIdHigh:X16}");
        long? allocated = standard.AllocationSize >= 0 ? standard.AllocationSize : null;
        if ((attributes & (FileAttributes.SparseFile | FileAttributes.Compressed)) != 0)
        {
            // AllocationSize may describe the logical allocation; query the actual compressed/sparse storage.
            allocated = GetCompression(handle, 8, out var compression, (uint)Marshal.SizeOf<CompressionInfo>())
                && compression.CompressedFileSize >= 0 ? compression.CompressedFileSize : null;
        }
        // Without identity the scanner cannot establish whether allocation is already counted elsewhere.
        if (identity is null) allocated = null;
        return new(path, identity, standard.EndOfFile, allocated,
            new DateTimeOffset(DateTime.FromFileTimeUtc(basic.LastWriteTime)), checked((int)standard.NumberOfLinks), attributes);
    }

    private static string ExtendedPath(string path)
    {
        var full = Path.GetFullPath(path);
        if (full.StartsWith(@"\\?\", StringComparison.Ordinal)) return full;
        return full.StartsWith(@"\\", StringComparison.Ordinal) ? @"\\?\UNC\" + full[2..] : @"\\?\" + full;
    }

    private static Exception MetadataError(string path, int error)
    {
        var native = new Win32Exception(error);
        return error == 5 ? new UnauthorizedAccessException($"Metadata unavailable: {path}", native)
            : new IOException($"Metadata unavailable: {path}", native);
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BasicInfo
    {
        public long CreationTime, LastAccessTime, LastWriteTime, ChangeTime;
        public uint Attributes;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct StandardInfo
    {
        public long AllocationSize, EndOfFile;
        public uint NumberOfLinks;
        public byte DeletePending, Directory;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct IdInfo
    {
        public ulong VolumeSerialNumber, FileIdLow, FileIdHigh;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct CompressionInfo
    {
        public long CompressedFileSize;
        public ushort CompressionFormat;
        public byte CompressionUnitShift, ChunkShift, ClusterShift, Reserved1, Reserved2, Reserved3;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct DispositionInfo { public byte DeleteFile; }

    [DllImport("kernel32.dll", EntryPoint = "GetFinalPathNameByHandleW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern uint GetFinalPathNameByHandle(SafeFileHandle file, StringBuilder path, uint length, uint flags);
    [DllImport("kernel32.dll", EntryPoint = "SetFileInformationByHandle", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool SetDisposition(SafeFileHandle file, int informationClass, ref DispositionInfo information, uint size);

    [DllImport("kernel32.dll", EntryPoint = "CreateFileW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern SafeFileHandle CreateFile(string path, uint access, uint share, IntPtr security,
        uint disposition, uint flags, IntPtr template);

    [DllImport("kernel32.dll", EntryPoint = "GetFileInformationByHandleEx", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetBasic(SafeFileHandle handle, int informationClass, out BasicInfo information, uint size);
    [DllImport("kernel32.dll", EntryPoint = "GetFileInformationByHandleEx", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetStandard(SafeFileHandle handle, int informationClass, out StandardInfo information, uint size);
    [DllImport("kernel32.dll", EntryPoint = "GetFileInformationByHandleEx", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetId(SafeFileHandle handle, int informationClass, out IdInfo information, uint size);
    [DllImport("kernel32.dll", EntryPoint = "GetFileInformationByHandleEx", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetCompression(SafeFileHandle handle, int informationClass, out CompressionInfo information, uint size);
}
