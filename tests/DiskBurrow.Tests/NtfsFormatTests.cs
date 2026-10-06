using DiskBurrow.Windows.Files.Mft;
using System.Buffers.Binary;
using System.Text;
namespace DiskBurrow.Tests;
public sealed class NtfsFormatTests
{
    private static readonly NtfsGeometry Geometry=new(512,4096,1024,4096,64*1024*1024);
    [Fact] public void Boot_sector_rejects_bad_geometry_and_outside_Mft()
    {
        var boot=new byte[512];"NTFS    "u8.CopyTo(boot.AsSpan(3));W16(boot,11,512);boot[13]=8;boot[64]=unchecked((byte)-10);W64(boot,40,131072);W64(boot,48,1);W16(boot,510,0xAA55);
        Assert.Equal(Geometry,NtfsFormat.Geometry(boot));boot[13]=3;Assert.Throws<NtfsFormatException>(()=>NtfsFormat.Geometry(boot));
        boot[13]=8;W64(boot,48,16384);Assert.Throws<NtfsFormatException>(()=>NtfsFormat.Geometry(boot));
    }
    [Fact] public void Signed_run_deltas_and_sparse_extents_preserve_physical_locations()
    {
        var runs=NtfsFormat.Runs([0x11,2,10,0x11,1,unchecked((byte)-3),0x01,4,0],0,6,Geometry);
        Assert.Equal(new NtfsRun(0,40960,8192),runs[0]);Assert.Equal(new NtfsRun(2,28672,4096),runs[1]);Assert.Null(runs[2].Offset);Assert.Equal(16384,runs[2].Bytes);
    }
    [Theory] [InlineData(0)] [InlineData(1)] [InlineData(2)] [InlineData(3)]
    public void Corrupt_runs_never_become_plausible_sizes(int kind)
    {
        byte[] data=kind switch {0=>[0x11,0,1,0],1=>[0x91,1,1,0],2=>[0x11,1,255,0],_=>[0x11,1,1]};
        Assert.Throws<NtfsFormatException>(()=>NtfsFormat.Runs(data,0,0,Geometry));
    }
    [Fact] public void Record_fixup_checks_every_sector_and_attribute_bounds()
    {
        var record=Record(1,0,Resident(0x80,[1,2,3]));var parsed=NtfsFormat.Record(record,Geometry);Assert.True(parsed.InUse);Assert.Equal(3,parsed.Attributes[0].LogicalBytes);Assert.Empty(parsed.Attributes[0].Value!);
        record=Record(1,0,Resident(0x80,[1,2,3]));record[1022]=0;Assert.Throws<NtfsFormatException>(()=>NtfsFormat.Record(record,Geometry));
        record=Record(1,0,Resident(0x80,[1,2,3]));W32(record,60,900);Assert.Throws<NtfsFormatException>(()=>NtfsFormat.Record(record,Geometry));
    }
    [Fact] public void Compressed_sizes_use_real_allocation_and_continuations_do_not_double_count()
    {
        var a=Nonresident(0x80,0,8192,4096,0x8001,[0x11,2,2,0]);
        var parsed=NtfsFormat.Record(Record(1,0,a),Geometry);Assert.Equal(8192,parsed.Attributes[0].LogicalBytes);Assert.Equal(4096,parsed.Attributes[0].AllocatedBytes);
        a=Nonresident(0x80,2,8192,8192,0,[0x11,1,4,0]);parsed=NtfsFormat.Record(Record(1,((ulong)7<<48)|18,a),Geometry);
        Assert.Equal(0,parsed.Attributes[0].AllocatedBytes);Assert.Equal(((ulong)7<<48)|18,parsed.BaseReference);
    }
    [Fact] public void Names_preserve_parent_sequence_and_drop_Dos_aliases()
    {
        var value=new byte[66+8];W64(value,0,((ulong)42<<48)|19);value[64]=4;value[65]=1;Encoding.Unicode.GetBytes("name").CopyTo(value,66);
        var p=NtfsFormat.Record(Record(1,0,Resident(0x30,value)),Geometry);Assert.Single(p.Names);Assert.Equal(19,NtfsFormat.RecordNumber(p.Names[0].ParentReference));Assert.Equal(42,NtfsFormat.Sequence(p.Names[0].ParentReference));
        value[65]=2;Assert.Empty(NtfsFormat.Record(Record(1,0,Resident(0x30,value)),Geometry).Names);
        value[65]=1;Encoding.Unicode.GetBytes("../x").CopyTo(value,66);Assert.Throws<NtfsFormatException>(()=>NtfsFormat.Record(Record(1,0,Resident(0x30,value)),Geometry));
    }
    [Fact] public void Attribute_list_preserves_owner_reference_and_instance_and_rejects_truncation()
    {
        var v=new byte[32];W32(v,0,0x80);W16(v,4,32);W64(v,8,3);W64(v,16,((ulong)9<<48)|33);W16(v,24,4);
        Assert.Equal(new NtfsListEntry(0x80,4,3,((ulong)9<<48)|33,""),Assert.Single(NtfsFormat.AttributeList(v)));
        W16(v,4,0);Assert.Throws<NtfsFormatException>(()=>NtfsFormat.AttributeList(v));
    }
    internal static byte[] Record(ushort flags,ulong baseRef,params byte[][] attrs)
    {
        var b=new byte[1024];"FILE"u8.CopyTo(b);W16(b,4,48);W16(b,6,3);W16(b,16,7);W16(b,20,56);W16(b,22,flags);W32(b,28,1024);W64(b,32,baseRef);
        var at=56;foreach(var a in attrs){a.CopyTo(b,at);at+=a.Length;}W32(b,at,uint.MaxValue);W32(b,24,(uint)(at+8));
        W16(b,48,0x1234);W16(b,50,0);W16(b,52,0);W16(b,510,0x1234);W16(b,1022,0x1234);return b;
    }
    internal static byte[] Resident(uint type,byte[] value)
    {var b=new byte[(24+value.Length+7)&~7];W32(b,0,type);W32(b,4,(uint)b.Length);W32(b,16,(uint)value.Length);W16(b,20,24);value.CopyTo(b,24);return b;}
    internal static byte[] Nonresident(uint type,ulong lowest,ulong logical,ulong allocated,ushort flags,byte[] runs)
    {var offset=(flags&0x8001)!=0?72:64;var b=new byte[(offset+runs.Length+7)&~7];W32(b,0,type);W32(b,4,(uint)b.Length);b[8]=1;W16(b,12,flags);W64(b,16,lowest);W64(b,24,lowest+(ulong)runs[1]-1);W16(b,32,(ushort)offset);W64(b,40,allocated);W64(b,48,logical);if(offset==72)W64(b,64,allocated);runs.CopyTo(b,offset);return b;}
    internal static void W16(byte[]b,int at,ushort v)=>BinaryPrimitives.WriteUInt16LittleEndian(b.AsSpan(at),v);
    internal static void W32(byte[]b,int at,uint v)=>BinaryPrimitives.WriteUInt32LittleEndian(b.AsSpan(at),v);
    internal static void W64(byte[]b,int at,ulong v)=>BinaryPrimitives.WriteUInt64LittleEndian(b.AsSpan(at),v);
}
