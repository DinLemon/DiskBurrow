using DiskBurrow.Core.Scanning;
using System.Text.Json;
namespace DiskBurrow.Tests;
public sealed class ScanTreeTests
{
    [Fact] public void Full_live_index_aggregates_all_files_and_reconstructs_paths()
    {
        var builder = new ScanTreeBuilder(@"C:\fixture"); var folder = builder.AddDirectory(0,"nested");
        for(var i=0;i<151;i++) builder.AddFile(folder,$"f{i}.bin",2,4,DateTimeOffset.UnixEpoch);
        var tree=builder.Build(); Assert.Equal(153,tree.Entries.Count); Assert.Equal(302,tree.Entries[0].LogicalBytes);
        Assert.Equal(604,tree.Entries[0].AllocatedBytes); Assert.Equal(151,tree.Entries[0].FileCount);
        Assert.Equal(@"C:\fixture\nested\f150.bin",tree.GetPath(152));
    }
    [Fact] public void Unknown_and_incomplete_children_propagate_without_hiding_other_files()
    {
        var b=new ScanTreeBuilder(@"C:\fixture"); var d=b.AddDirectory(0,"d"); b.AddFile(d,"a",10,null,default); b.MarkIncomplete(d);
        var t=b.Build(); Assert.False(t.Entries[0].CoverageComplete); Assert.Null(t.Entries[0].AllocatedBytes); Assert.Equal(10,t.Entries[0].LogicalBytes);
    }
    [Fact] public void Live_index_is_not_persisted_in_snapshot_history()
    {
        var b=new ScanTreeBuilder(@"C:\fixture"); b.AddFile(0,"private-full-index",1,1,default);
        var s=new ScanSnapshot(Guid.NewGuid(),@"C:\fixture",default,default,true,[],[],[]) {Tree=b.Build()};
        var json=JsonSerializer.Serialize(s); Assert.DoesNotContain("private-full-index",json); Assert.Null(JsonSerializer.Deserialize<ScanSnapshot>(json)!.Tree);
    }
    [Fact] public void Repeated_build_does_not_double_count_and_parent_contract_rejects_escape()
    {
        var b=new ScanTreeBuilder(@"C:\fixture"); b.AddFile(0,"x",7,8,default); Assert.Equal(7,b.Build().Entries[0].LogicalBytes); Assert.Equal(7,b.Build().Entries[0].LogicalBytes);
        Assert.Throws<ArgumentException>(()=>b.AddFile(0,"..",1,1,default)); Assert.Throws<ArgumentException>(()=>b.AddDirectory(1,"child"));
    }
}
