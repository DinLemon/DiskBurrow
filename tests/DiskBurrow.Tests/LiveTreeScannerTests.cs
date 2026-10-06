using DiskBurrow.Windows.Files;
namespace DiskBurrow.Tests;
public sealed class LiveTreeScannerTests
{
 [Fact] public async Task Scanner_indexes_every_file_beyond_largest_hundred()
 {
  using var fixture=new TempTree(); for(var i=0;i<151;i++) fixture.FileAt($"nested/f{i}.bin",i+1);
  var scan=await new WindowsDiskScanner().ScanAsync(fixture.Root,null,default);
  Assert.NotNull(scan.Tree); Assert.Equal(153,scan.Tree.Entries.Count); Assert.Equal(151,scan.Tree.Entries[0].FileCount);
  Assert.Equal(scan.Directories.First(x=>x.Path==fixture.Root).LogicalBytes,scan.Tree.Entries[0].LogicalBytes);
  Assert.Equal(scan.Directories.First(x=>x.Path==fixture.Root).AllocatedBytes,scan.Tree.Entries[0].AllocatedBytes);
  Assert.Equal(100,scan.LargestFiles.Count);
 }
 [Fact] public async Task Live_tree_preserves_hardlink_names_but_charges_allocation_once_and_skips_junction_descendants()
 {
  using var fixture=new TempTree();var original=fixture.FileAt("z/original.bin",8193);fixture.HardLink("a/canonical.bin",original);
  var junction=fixture.Junction("cycle",fixture.Root);var scan=await new WindowsDiskScanner().ScanAsync(fixture.Root,null,default);var tree=scan.Tree!;
  Assert.Equal(2,tree.Entries[0].FileCount);Assert.Equal(16386,tree.Entries[0].LogicalBytes);Assert.Null(tree.Entries[0].AllocatedBytes);
  var files=Enumerable.Range(0,tree.Entries.Count).Where(i=>!tree.Entries[i].IsDirectory).ToArray();Assert.Equal(2,files.Length);
  Assert.Equal(new NativeFileApi().Inspect(original).AllocatedBytes,tree.Entries[files.Single(i=>tree.GetPath(i).EndsWith("canonical.bin"))].AllocatedBytes);
  Assert.Equal(0,tree.Entries[files.Single(i=>tree.GetPath(i)==original)].AllocatedBytes);
  Assert.DoesNotContain(Enumerable.Range(0,tree.Entries.Count),i=>tree.GetPath(i).StartsWith(junction+"\\",StringComparison.OrdinalIgnoreCase));
 }
}
