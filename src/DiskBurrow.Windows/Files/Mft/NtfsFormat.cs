// NTFS run/fixup and metadata interpretation adapted from disktree by Tobi Lütke,
// MIT, commit 6c8d4ce6211bf135ff4e42890faebff1040e1846. See THIRD_PARTY_NOTICES.
using System.Buffers.Binary;
using System.Text;

namespace DiskBurrow.Windows.Files.Mft;

public sealed class NtfsFormatException(string message) : IOException("NTFS metadata: " + message);
public readonly record struct NtfsGeometry(int SectorBytes, int ClusterBytes, int RecordBytes, long MftOffset, long VolumeBytes);
public readonly record struct NtfsRun(long Vcn, long? Offset, long Bytes);
public readonly record struct NtfsName(ulong ParentReference, string Name);
public readonly record struct NtfsListEntry(uint Type, ushort Instance, long LowestVcn, ulong Reference, string Name);
public sealed record NtfsAttribute(uint Type, ushort Instance, string Name, ushort Flags, long LowestVcn,
    long LogicalBytes, long AllocatedBytes, byte[]? Value, IReadOnlyList<NtfsRun> Runs);
public sealed record NtfsRecord(ushort Sequence, bool InUse, bool Directory, ulong BaseReference,
    FileAttributes Flags, long ModifiedFileTime, IReadOnlyList<NtfsName> Names, IReadOnlyList<NtfsAttribute> Attributes,bool StandardInformationValid);

/// <summary>Bounds-checked decoding. No disk reads or filesystem traversal occur here.</summary>
public static class NtfsFormat
{
    public const ulong ReferenceMask = 0x0000FFFFFFFFFFFF;
    public static int RecordNumber(ulong reference)
    {
        var number = reference & ReferenceMask;
        if (number > int.MaxValue) throw Bad("record reference exceeds the supported index");
        return (int)number;
    }
    public static ushort Sequence(ulong reference) => (ushort)(reference >> 48);
    public static NtfsGeometry Geometry(ReadOnlySpan<byte> boot)
    {
        if (boot.Length < 512 || !boot.Slice(3, 8).SequenceEqual("NTFS    "u8) || U16(boot,510) != 0xAA55) throw Bad("invalid NTFS boot sector");
        var sector = U16(boot,11); var sectors = boot[13];
        if (!Power(sector) || sector < 512 || sector > 4096 || !Power(sectors)) throw Bad("unsupported sector geometry");
        var cluster = checked(sector * sectors);
        var encoded = (sbyte)boot[64];
        var record = encoded < 0 && encoded >= -16 ? 1 << -encoded : encoded > 0 ? checked(cluster * encoded) : 0;
        if (cluster > 2*1024*1024 || !Power(record) || record < 1024 || record > 65536 || record % 512 != 0) throw Bad("unsupported record geometry");
        var volume = checked(Long(U64(boot,40)) * sector);
        var mft = checked(Long(U64(boot,48)) * cluster);
        if (volume <= 0 || mft < cluster || mft > volume - record) throw Bad("MFT lies outside volume");
        return new(sector,cluster,record,mft,volume);
    }
    public static NtfsRecord Record(Span<byte> bytes, NtfsGeometry geometry)
    {
        if (bytes.Length != geometry.RecordBytes || !bytes[..4].SequenceEqual("FILE"u8)) throw Bad("invalid FILE signature/record length");
        Fixup(bytes);
        var used = checked((int)U32(bytes,24)); var allocated = U32(bytes,28); var first = U16(bytes,20);
        if (used < 48 || used > bytes.Length || allocated != bytes.Length || first < 48 || first >= used || first % 8 != 0 || first<U16(bytes,4)+U16(bytes,6)*2) throw Bad("invalid record attribute bounds");
        var flags = U16(bytes,22); var names = new List<NtfsName>(); var attrs = new List<NtfsAttribute>();
        FileAttributes attributes = (flags & 2) != 0 ? FileAttributes.Directory : FileAttributes.Normal;
        long modified = 0; var ended = false;var standardCount=0;
        for (var at = (int)first; at <= used - 4;)
        {
            var type = U32(bytes,at); if (type == uint.MaxValue) { ended = true; break; }
            var length = checked((int)U32(bytes,at+4));
            if (length < 24 || length % 8 != 0 || length > used-at) throw Bad("invalid attribute length");
            var a = bytes.Slice(at,length); var nonresident = a[8];
            if (nonresident > 1) throw Bad("invalid attribute residency");
            var nameLength = a[9]; var nameOffset = U16(a,10); var af = U16(a,12); var instance = U16(a,14);
            var minimum = nonresident == 1 ? ((af & 0x8001) != 0 ? 72 : 64) : 24;
            if (length < minimum || (nameLength != 0 && nameOffset < minimum)) throw Bad("invalid attribute header/name");
            var name = nameLength == 0 ? "" : Utf16(Slice(a,nameOffset,nameLength*2));
            byte[]? value = null; long lowest = 0, logical = 0, spent = 0; IReadOnlyList<NtfsRun> runs = [];
            if (nonresident == 0)
            {
                var size = checked((int)U32(a,16)); var offset = U16(a,20);
                if (offset < minimum || offset < nameOffset+nameLength*2) throw Bad("overlapping resident value");
                var resident=Slice(a,offset,size);logical=size;
                // Raw MFT blocks contain resident file data. Retain its size, never a copy of its content.
                value=type==0x80?[]:resident.ToArray();
            }
            else
            {
                lowest = Long(U64(a,16)); var highest = Long(U64(a,24)); var runOffset = U16(a,32);
                if (runOffset < minimum || runOffset < nameOffset+nameLength*2 || highest < lowest) throw Bad("invalid nonresident extent");
                runs = Runs(Slice(a,runOffset,a.Length-runOffset),lowest,highest,geometry);
                if (lowest == 0) { logical = Long(U64(a,48)); spent = Long(U64(a,(af & 0x8001) != 0 ? 64 : 40)); }
                if (logical > geometry.VolumeBytes && (af & 0x8001) == 0 || spent > geometry.VolumeBytes) throw Bad("impossible stream size");
            }
            attrs.Add(new(type,instance,name,af,lowest,logical,spent,value,runs));
            if (type == 0x10)
            {
                if (value is null || value.Length < 36) throw Bad("invalid STANDARD_INFORMATION");
                standardCount++;modified = unchecked((long)U64(value,8)); attributes |= (FileAttributes)U32(value,32);
                if ((flags&2)!=0) attributes |= FileAttributes.Directory;
            }
            if(type==0xC0)attributes|=FileAttributes.ReparsePoint;
            if (type == 0x30)
            {
                if (value is null || value.Length < 66) throw Bad("invalid FILE_NAME");
                var ns = value[65]; if (ns > 3) throw Bad("invalid filename namespace");
                // Cached filename/reparse metadata can add a no-follow restriction; it cannot clear one.
                attributes|=(FileAttributes)U32(value,56)&(FileAttributes.ReparsePoint|FileAttributes.Offline|(FileAttributes)0x40000|(FileAttributes)0x400000);
                var n = Utf16(Slice(value,66,value[64]*2));
                // The volume root's self-name is '.', and DOS names duplicate Win32 entries.
                if (ns != 2 && n != ".") { ValidateName(n); names.Add(new(U64(value,0),n)); }
            }
            at = checked(at+length);
        }
        if (!ended) throw Bad("unterminated attribute list");
        return new(U16(bytes,16),(flags&1)!=0,(flags&2)!=0,U64(bytes,32),attributes,modified,names,attrs,standardCount==1);
    }
    public static void Fixup(Span<byte> bytes)
    {
        var at = U16(bytes,4); var count = U16(bytes,6);
        if (bytes.Length == 0 || bytes.Length%512!=0 || count != bytes.Length/512+1 || at < 42 || at%2!=0 || at > bytes.Length-count*2) throw Bad("invalid update sequence array");
        var check = U16(bytes,at);
        for (var i=1;i<count;i++)
        {
            var tail=i*512-2; if (U16(bytes,tail)!=check) throw Bad("torn record sector");
            BinaryPrimitives.WriteUInt16LittleEndian(bytes.Slice(tail,2),U16(bytes,at+i*2));
        }
    }
    public static IReadOnlyList<NtfsRun> Runs(ReadOnlySpan<byte> data,long lowest,long highest,NtfsGeometry geometry)
    {
        var result=new List<NtfsRun>(); long lcn=0,vcn=lowest; var terminated=false;
        for(var at=0;at<data.Length;)
        {
            var head=data[at++]; if(head==0){terminated=true;break;}
            var len=head&15; var delta=head>>4;
            if(len is <1 or >8 || delta>8)throw Bad("invalid run encoding");
            var count=Unsigned(Slice(data,at,len)); at+=len;
            if(count==0 || count>long.MaxValue)throw Bad("zero/overflow run length");
            long? offset=null;
            if(delta!=0)
            {
                var encoded=Unsigned(Slice(data,at,delta));
                if(delta<8 && (data[at+delta-1]&128)!=0)encoded|=ulong.MaxValue<<(delta*8);
                lcn=checked(lcn+unchecked((long)encoded));
                if(lcn<0)throw Bad("negative run location");
                offset=checked(lcn*geometry.ClusterBytes);
            }
            at+=delta; var bytes=checked((long)count*geometry.ClusterBytes);
            if(offset is {} o && (o>geometry.VolumeBytes || bytes>geometry.VolumeBytes-o))throw Bad("run outside volume");
            result.Add(new(vcn,offset,bytes)); vcn=checked(vcn+(long)count);
        }
        if(!terminated || vcn!=checked(highest+1))throw Bad("unterminated or incomplete run extent");
        return result;
    }
    public static IReadOnlyList<NtfsListEntry> AttributeList(ReadOnlySpan<byte> value)
    {
        var entries=new List<NtfsListEntry>();
        for(var at=0;at<value.Length;)
        {
            if(value.Length-at<26)throw Bad("truncated ATTRIBUTE_LIST");
            var type=U32(value,at); var size=U16(value,at+4); var len=value[at+6];var offset=value[at+7];
            if(size<26 || size>value.Length-at || (len>0 && offset<26))throw Bad("invalid ATTRIBUTE_LIST entry");
            var e=value.Slice(at,size); var name=len==0?"":Utf16(Slice(e,offset,len*2));
            entries.Add(new(type,U16(e,24),Long(U64(e,8)),U64(e,16),name));at+=size;
        }
        return entries;
    }
    public static void ValidateName(string name)
    { if(string.IsNullOrEmpty(name)||name is "." or ".."||name.IndexOfAny(['\\','/','\0',':'])>=0)throw Bad("unsafe filename"); }
    internal static ReadOnlySpan<byte> Slice(ReadOnlySpan<byte> b,int at,int count)
    {if(at<0||count<0||at>b.Length-count)throw Bad("truncated field");return b.Slice(at,count);}
    internal static ushort U16(ReadOnlySpan<byte>b,int at)=>BinaryPrimitives.ReadUInt16LittleEndian(Slice(b,at,2));
    internal static uint U32(ReadOnlySpan<byte>b,int at)=>BinaryPrimitives.ReadUInt32LittleEndian(Slice(b,at,4));
    internal static ulong U64(ReadOnlySpan<byte>b,int at)=>BinaryPrimitives.ReadUInt64LittleEndian(Slice(b,at,8));
    private static ulong Unsigned(ReadOnlySpan<byte>b){ulong v=0;for(var i=0;i<b.Length;i++)v|=(ulong)b[i]<<(i*8);return v;}
    private static long Long(ulong value)=>value<=long.MaxValue?(long)value:throw Bad("unsigned length overflow");
    private static bool Power(int value)=>value>0&&(value&(value-1))==0;
    private static string Utf16(ReadOnlySpan<byte>b)
    {try{return new UnicodeEncoding(false,false,true).GetString(b);}catch(DecoderFallbackException){throw Bad("invalid UTF-16 filename");}}
    private static NtfsFormatException Bad(string message)=>new(message);
}
