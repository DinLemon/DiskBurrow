namespace DiskBurrow.App.ViewModels;
public sealed record DiskVolumeChoice(string Root,long TotalBytes,long FreeBytes)
{
 public long UsedBytes=>Math.Max(0,TotalBytes-FreeBytes);
 public double UsedPercent=>TotalBytes>0?100d*(TotalBytes-FreeBytes)/TotalBytes:0;
 public static IReadOnlyList<DiskVolumeChoice> ReadLocalVolumes()
 {
  var result=new List<DiskVolumeChoice>();
  foreach(var drive in DriveInfo.GetDrives())
  {
   try{if(drive.DriveType is not (DriveType.Fixed or DriveType.Removable)||!drive.IsReady)continue;result.Add(new(drive.RootDirectory.FullName,drive.TotalSize,drive.AvailableFreeSpace));}
   catch(Exception e) when(e is IOException or UnauthorizedAccessException){ }
  }
  return result.OrderByDescending(x=>x.UsedPercent).ThenBy(x=>x.Root,StringComparer.OrdinalIgnoreCase).ToArray();
 }
}
