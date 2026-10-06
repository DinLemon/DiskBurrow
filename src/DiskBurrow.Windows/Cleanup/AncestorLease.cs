using DiskBurrow.Core.Cleanup;
using DiskBurrow.Windows.Files;
using DiskBurrow.Core.Scanning;
using Microsoft.Win32.SafeHandles;

namespace DiskBurrow.Windows.Cleanup;

public sealed class AncestorLease : IDisposable
{
    private readonly NativeFileApi files;
    private readonly List<(string Path, SafeFileHandle Handle, FileIdentity Identity)> ancestors = [];
    private AncestorLease(NativeFileApi files) => this.files = files;
    public ulong Volume => ancestors[0].Identity.Volume;

    public static AncestorLease OpenVerified(RuleRoot root, string candidatePath, NativeFileApi? files = null)
    {
        if (!CleanupPaths.TryNormalize(root.Path, out var boundary) ||
            !CleanupPaths.TryNormalize(candidatePath, out var candidate) ||
            !CleanupPaths.IsWithin(candidate, boundary) || CleanupPaths.EqualsPath(candidate, boundary))
            throw new InvalidDataException("Cleanup.UnsafePath");
        var lease = new AncestorLease(files ?? new NativeFileApi());
        try
        {
            var components = new Stack<string>();
            for (var path = Path.GetDirectoryName(candidate); path is not null; path = Path.GetDirectoryName(path)) components.Push(path);
            // Each parent is already pinned before its child's name is resolved.
            while (components.TryPop(out var path))
            {
                var handle = lease.files.OpenCleanupDirectory(path);
                try
                {
                    var observation = lease.files.InspectHandle(handle, path);
                    lease.CheckDirectory(path, handle, observation);
                    if (lease.ancestors.Count > 0 && observation.Identity!.Volume != lease.Volume)
                        throw new InvalidDataException("Cleanup.VolumeChanged");
                    lease.ancestors.Add((path, handle, observation.Identity!));
                }
                catch { handle.Dispose(); throw; }
            }
            return lease;
        }
        catch { lease.Dispose(); throw; }
    }

    public void Verify()
    {
        foreach (var ancestor in ancestors)
        {
            var observed = files.InspectHandle(ancestor.Handle, ancestor.Path);
            CheckDirectory(ancestor.Path, ancestor.Handle, observed);
            if (observed.Identity != ancestor.Identity) throw new InvalidDataException("Cleanup.AncestorChanged");
        }
    }

    private void CheckDirectory(string path, SafeFileHandle handle, FileObservation observed)
    {
        if (observed.Identity is null || !observed.Attributes.HasFlag(FileAttributes.Directory) ||
            observed.Attributes.HasFlag(FileAttributes.ReparsePoint) || NativeFileApi.IsCloud(observed.Attributes) ||
            !CleanupPaths.TryNormalize(files.GetFinalPath(handle), out var final) || !CleanupPaths.EqualsPath(path, final))
            throw new InvalidDataException("Cleanup.UnsafeAncestor");
    }

    public void Dispose()
    {
        for (var index = ancestors.Count - 1; index >= 0; index--) ancestors[index].Handle.Dispose();
        ancestors.Clear();
    }
}
