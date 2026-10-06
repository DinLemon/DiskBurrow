using DiskBurrow.Core.Scanning;

namespace DiskBurrow.App.ViewModels;

public abstract class LargestSelectionRow(ManualDeletionViewModel owner) : ObservableModel
{
    public abstract string Path {get;}
    public abstract long LogicalBytes {get;}
    public abstract long? AllocatedBytes {get;}
    public bool Selected {get=>owner.IsSelected(Path);set{owner.Select(Path,value);Changed();}}
    public void RefreshSelection()=>Changed(nameof(Selected));
}
public sealed class DirectorySelectionRow(DirectoryObservation observation,ManualDeletionViewModel owner) : LargestSelectionRow(owner)
{
    public DirectoryObservation Observation=>observation;
    public override string Path=>observation.Path;
    public override long LogicalBytes=>observation.LogicalBytes;
    public override long? AllocatedBytes=>observation.AllocatedBytes;
    public bool CoverageComplete=>observation.CoverageComplete;
}
public sealed class FileSelectionRow(FileObservation observation,ManualDeletionViewModel owner) : LargestSelectionRow(owner)
{
    public FileObservation Observation=>observation;
    public override string Path=>observation.Path;
    public override long LogicalBytes=>observation.LogicalBytes;
    public override long? AllocatedBytes=>observation.AllocatedBytes;
}
