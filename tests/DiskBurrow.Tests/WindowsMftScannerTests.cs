using DiskBurrow.Core.Scanning;
using DiskBurrow.Windows.Files.Mft;
using System.Text;
using static DiskBurrow.Tests.NtfsFormatTests;
namespace DiskBurrow.Tests;
public sealed class WindowsMftScannerTests
{
    [Fact] public void Raw_chunks_build_full_tree_and_hardlinks_charge_one_canonical_name()
    {
        using var v=new Volume();v.Set(16,3,0,Name(5,"folder"));v.Set(17,1,0,Name(16,"first"),Name(5,"alias"),Nonresident(0x80,0,5000,8192,0,[0x11,2,20,0]));
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.True(s.TraversalCompleted);Assert.True(s.Directories[0].CoverageComplete);
        Assert.Equal(10000,s.Tree!.Entries[0].LogicalBytes);Assert.Equal(8192,s.Tree.Entries[0].AllocatedBytes);Assert.Equal(2,s.Tree.Entries[0].FileCount);
        Assert.Equal(4,s.Tree.Entries.Count);Assert.Contains(s.Tree.Entries,e=>e.Name=="first"&&e.AllocatedBytes==0);Assert.Contains(s.Tree.Entries,e=>e.Name=="alias"&&e.AllocatedBytes==8192);
        Assert.True(v.Reads<10);Assert.Equal(2,s.LargestFiles.Count);
    }
    [Fact] public void Cloud_and_reparse_directories_exclude_descendants_and_mark_coverage()
    {
        using var v=new Volume();v.Set(16,3,0,Name(5,"cloud"),Standard(FileAttributes.Directory|FileAttributes.Offline));v.Set(17,1,0,Name(16,"unread"),Resident(0x80,[1,2,3]));
        v.Set(18,3,0,Name(5,"junction"),Standard(FileAttributes.Directory|FileAttributes.ReparsePoint));v.Set(19,1,0,Name(18,"elsewhere"),Resident(0x80,[7]));
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.Single(s.Tree!.Entries);Assert.False(s.Directories[0].CoverageComplete);Assert.Equal(2,s.Issues.Count);Assert.Empty(s.LargestFiles);
    }
    [Fact] public void Stale_parent_and_cycles_never_claim_a_complete_volume()
    {
        using var v=new Volume();v.Set(16,3,0,Name(5,"folder"));v.Set(17,1,0,Name(16,"stale",8),Resident(0x80,[1]));v.Set(18,3,0,Name(19,"cycle-a"));v.Set(19,3,0,Name(18,"cycle-b"));
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.False(s.Directories[0].CoverageComplete);Assert.Null(s.Directories[0].AllocatedBytes);Assert.Empty(s.LargestFiles);
        Assert.False(s.Directories.Single(d=>d.Path==@"C:\folder").CoverageComplete);
    }
    [Fact] public void Attribute_extension_names_and_data_require_matching_base_owner_and_sequence()
    {
        using var v=new Volume();v.Set(16,1,0,Name(5,"main"),List((0x80,22,7,0)));v.Set(22,1,((ulong)7<<48)|16,Nonresident(0x80,0,5000,8192,0,[0x11,2,20,0]));
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.True(s.Directories[0].CoverageComplete);Assert.Equal(5000,Assert.Single(s.LargestFiles).LogicalBytes);
        v.Set(22,1,((ulong)8<<48)|16,Nonresident(0x80,0,5000,8192,0,[0x11,2,20,0]));s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.False(s.Directories[0].CoverageComplete);
    }
    [Fact] public void Named_stream_physical_bytes_are_included_but_logical_size_is_unnamed_only()
    {
        using var v=new Volume();var named=Nonresident(0x80,0,1,4096,0,[0x11,1,24,0]);
        // Move the mapping pairs after the named attribute header.
        var expanded=new byte[88];named.CopyTo(expanded,0);W32(expanded,4,88);expanded[9]=1;W16(expanded,10,64);W16(expanded,32,72);Encoding.Unicode.GetBytes("x").CopyTo(expanded,64);new byte[]{0x11,1,24,0}.CopyTo(expanded,72);
        v.Set(16,1,0,Name(5,"streams"),Resident(0x80,[1,2]),expanded);
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.Equal(2,Assert.Single(s.LargestFiles).LogicalBytes);Assert.Equal(4096,s.Tree!.Entries[0].AllocatedBytes);
    }
    [Fact] public void Too_short_bitmap_and_cancel_fail_without_success_snapshot()
    {
        using var v=new Volume();v.ShortenBitmap();Assert.Throws<NtfsFormatException>(()=>WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default));
        using var cancellation=new CancellationTokenSource();cancellation.Cancel();Assert.Throws<OperationCanceledException>(()=>WindowsMftScanner.ReadSnapshot(v,@"C:\",null,cancellation.Token));
    }
    [Fact] public void Reserved_system_container_descendants_are_not_false_orphans()
    {
        using var v=new Volume();v.Set(11,3,0,Name(5,"$Extend"));v.Set(16,1,0,Name(11,"$UsnJrnl"),Resident(0x80,[0]));
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.True(s.Directories[0].CoverageComplete);Assert.Single(s.Tree!.Entries);
    }
    [Fact] public void Mft_bootstrap_resolves_attribute_list_data_extents_without_counting_metadata()
    {
        using var v=new Volume();v.Set(16,1,0,Name(5,"user"),Resident(0x80,[1,2,3]));v.SplitMftData();
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);Assert.True(s.Directories[0].CoverageComplete);Assert.Equal(3,Assert.Single(s.LargestFiles).LogicalBytes);
    }
    [Theory] [InlineData(0)] [InlineData(1)] [InlineData(2)] [InlineData(3)] [InlineData(4)] [InlineData(5)] [InlineData(6)] [InlineData(7)]
    public void Untrusted_base_or_data_streams_never_claim_complete_coverage(int kind)
    {
        using var v=new Volume();v.Set(16,3,0,Name(5,"folder"));
        var attributes=new List<byte[]> {Name(16,"uncertain"),Standard(FileAttributes.Normal)};
        switch(kind)
        {
            case 0: attributes.Add(Nonresident(0x80,0,8192,8192,0,[0x11,1,20,0]));break;
            case 1: attributes.Add(Nonresident(0x80,0,8192,4096,0,[0x11,2,20,0]));break;
            case 2: attributes.Add(Nonresident(0x80,0,4096,0,0,[0x01,1,0]));break;
            case 3: break;
            case 4: attributes.RemoveAt(1);attributes.Add(Resident(0x80,[1]));break;
            case 5: attributes.RemoveAt(1);attributes.Add(Resident(0x80,[1]));attributes.Add(Resident(0xC0,new byte[8]));break;
            case 6: attributes.RemoveAt(1);attributes[0]=Name(16,"uncertain",7,FileAttributes.ReparsePoint);attributes.Add(Resident(0x80,[1]));break;
            case 7: attributes[1]=Standard(FileAttributes.ReparsePoint);attributes.Add(Standard(FileAttributes.Normal));attributes.Add(Resident(0x80,[1]));break;
        }
        v.SetRaw(17,1,0,attributes.ToArray());var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);
        Assert.False(s.Directories[0].CoverageComplete);Assert.Null(s.Directories[0].AllocatedBytes);Assert.NotEmpty(s.Issues);
        Assert.False(s.Directories.Single(d=>d.Path==@"C:\folder").CoverageComplete);
    }
    [Theory] [InlineData(0x8000)] [InlineData(0x0001)]
    public void Valid_sparse_and_compressed_streams_preserve_actual_physical_bytes(ushort flag)
    {
        using var v=new Volume();var data=Nonresident(0x80,0,8192,4096,flag,[0x11,1,20,0x01,1,0]);W64(data,24,1);
        v.Set(16,1,0,Name(5,"sparse"),data);var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);
        Assert.True(s.Directories[0].CoverageComplete);Assert.Equal(8192,s.Directories[0].LogicalBytes);Assert.Equal(4096,s.Directories[0].AllocatedBytes);
    }
    [Fact] public void Torn_record_with_untrusted_parent_never_leaves_child_directories_comparable()
    {
        using var v=new Volume();v.Set(16,3,0,Name(5,"folder"));v.Set(17,1,0,Name(16,"torn"),Resident(0x80,[1]));v.Tear(17);
        var s=WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default);
        Assert.All(s.Directories,d=>{Assert.False(d.CoverageComplete);Assert.Null(d.AllocatedBytes);});
    }
    [Fact] public void Retained_attribute_lists_and_extension_metadata_are_charged_to_the_memory_budget()
    {
        using var v=new Volume();v.Set(16,1,0,Name(5,"listed"),Resident(0x80,[1]),List((0x80,16,7,0)));
        var error=Assert.Throws<IOException>(()=>WindowsMftScanner.ReadSnapshot(v,@"C:\",null,default,3000));Assert.Contains("memory budget",error.Message);
    }
    private static byte[] Name(int parent,string name,ushort sequence=7,FileAttributes flags=FileAttributes.Normal)
    {var v=new byte[66+name.Length*2];W64(v,0,((ulong)sequence<<48)|(uint)parent);W32(v,56,(uint)flags);v[64]=(byte)name.Length;v[65]=1;Encoding.Unicode.GetBytes(name).CopyTo(v,66);return Resident(0x30,v);}
    private static byte[] Standard(FileAttributes flags){var v=new byte[48];W64(v,8,(ulong)DateTime.UtcNow.ToFileTimeUtc());W32(v,32,(uint)flags);return Resident(0x10,v);}
    private static byte[] List(params (uint Type,int Record,ushort Sequence,ulong Vcn)[]entries)
    {var v=new byte[entries.Length*32];for(var i=0;i<entries.Length;i++){var e=entries[i];var at=i*32;W32(v,at,e.Type);W16(v,at+4,32);W64(v,at+8,e.Vcn);W64(v,at+16,((ulong)e.Sequence<<48)|(uint)e.Record);}return Resident(0x20,v);}
    private sealed class Volume:INtfsMetadataReader
    {
        private readonly byte[] bytes=new byte[1024*1024];private readonly byte[] bitmap=new byte[4];public int Reads{get;private set;}
        public Volume(){"NTFS    "u8.CopyTo(bytes.AsSpan(3));W16(bytes,11,512);bytes[13]=8;bytes[64]=unchecked((byte)-10);W64(bytes,40,2048);W64(bytes,48,1);W16(bytes,510,0xAA55);Set(5,3,0);Rebuild();}
        public void Set(int number,ushort flags,ulong baseRef,params byte[][]attrs)
        {if(baseRef==0 && !attrs.Any(a=>System.Buffers.Binary.BinaryPrimitives.ReadUInt32LittleEndian(a)==0x10))attrs=[Standard((flags&2)!=0?FileAttributes.Directory:FileAttributes.Normal),..attrs];SetRaw(number,flags,baseRef,attrs);}
        public void SetRaw(int number,ushort flags,ulong baseRef,params byte[][]attrs){Record(flags,baseRef,attrs).CopyTo(bytes,4096+number*1024);bitmap[number>>3]|=(byte)(1<<(number&7));Rebuild();}
        private void Rebuild(){bitmap[0]|=1;Record(1,0,Nonresident(0x80,0,32768,32768,0,[0x11,8,1,0]),Resident(0xB0,bitmap)).CopyTo(bytes,4096);}
        public void ShortenBitmap()=>Record(1,0,Nonresident(0x80,0,32768,32768,0,[0x11,8,1,0]),Resident(0xB0,[1])).CopyTo(bytes,4096);
        public void SplitMftData()
        {
            bitmap[0]|=1<<6;
            Record(1,(ulong)7<<48,Nonresident(0x80,4,32768,32768,0,[0x11,4,5,0])).CopyTo(bytes,4096+6*1024);
            Record(1,0,Nonresident(0x80,0,32768,32768,0,[0x11,4,1,0]),List((0x80,0,7,0),(0x80,6,7,4)),Resident(0xB0,bitmap)).CopyTo(bytes,4096);
        }
        public void Tear(int number)=>bytes[4096+number*1024+1022]=0;
        public byte[] ReadAt(long offset,int length){Reads++;return bytes.AsSpan((int)offset,length).ToArray();}
        public void Dispose(){}
    }
}
