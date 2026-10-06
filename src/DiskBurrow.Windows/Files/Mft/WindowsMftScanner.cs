// Run mapping and MFT bootstrap adapted from disktree, MIT, Tobi Lütke (2026).
// Pinned source: 6c8d4ce6211bf135ff4e42890faebff1040e1846. See THIRD_PARTY_NOTICES.
using DiskBurrow.Core.Scanning;
using Microsoft.Win32.SafeHandles;
using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Runtime.CompilerServices;

namespace DiskBurrow.Windows.Files.Mft;

public interface INtfsMetadataReader : IDisposable { byte[] ReadAt(long offset, int bytes); }

/// <summary>Reads raw NTFS metadata; never opens file content, follows reparse points, or writes the volume.</summary>
public sealed class WindowsMftScanner : IDiskScanner
{
    public static string ValidateRoot(string root)
    {
        if (root.Length != 3 || !char.IsAsciiLetter(root[0]) || root[1] != ':' || root[2] != '\\')
            throw new ArgumentException("Fast NTFS scan requires a whole local drive, such as C:\\.",nameof(root));
        var canonical=char.ToUpperInvariant(root[0])+":\\";
        var drive=new DriveInfo(canonical);
        if(!drive.IsReady || drive.DriveType is not (DriveType.Fixed or DriveType.Removable) || drive.DriveFormat!="NTFS")
            throw new NotSupportedException("Fast scan requires a ready local NTFS volume.");
        return canonical;
    }
    public Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? progress,CancellationToken ct)
    {
        var canonical=ValidateRoot(root);
        return Task.Run(()=>{using var reader=new RawNtfsReader(canonical);return ReadSnapshot(reader,canonical,progress,ct);},ct);
    }

    /// <summary>Pure metadata-source entry point, also used with synthetic raw-volume fixtures.</summary>
    public static ScanSnapshot ReadSnapshot(INtfsMetadataReader reader,string root,IProgress<ScanProgress>? progress,CancellationToken ct,long maximumMetadataResident=1536L*1024*1024)
    {
        if(maximumMetadataResident<=0||maximumMetadataResident>1536L*1024*1024)throw new ArgumentOutOfRangeException(nameof(maximumMetadataResident));
        var started=DateTimeOffset.UtcNow;ct.ThrowIfCancellationRequested();
        try
        {
            var boot=reader.ReadAt(0,4096);var geometry=NtfsFormat.Geometry(boot);var serial=NtfsFormat.U64(boot,72);
            var layout=ReadLayout(reader,geometry,ct);
            var count=checked((int)((layout.Bytes+geometry.RecordBytes-1)/geometry.RecordBytes));
            // Bound allocations from untrusted on-disk lengths. Above this the ordinary walker is explicit.
            if(count<16 || count>12_000_000 || layout.Bitmap.Length<(count+7L)/8)throw new NtfsFormatException("MFT record count/bitmap exceeds supported bounds");
            long metadataResident=(long)count*Unsafe.SizeOf<Info>();
            if(metadataResident>maximumMetadataResident)throw new IOException("NTFS live metadata index exceeds the supported memory budget.");
            var infos=new Info[count];var names=new List<Name>();var lists=new Dictionary<int,IReadOnlyList<NtfsListEntry>>();
            void ReserveMetadata(long bytes)
            {metadataResident=checked(metadataResident+bytes);if(metadataResident>maximumMetadataResident)throw new IOException("NTFS live metadata index exceeds the supported memory budget.");}
            var extensionAttributes=new Dictionary<int,IReadOnlyList<NtfsAttribute>>();
            var listSources=new Dictionary<int,IReadOnlyList<NtfsAttribute>>();var corrupt=false;var uncertainOwners=new HashSet<int>();
            long visitedFiles=0,visitedDirectories=0;var clock=Stopwatch.StartNew();var last=TimeSpan.Zero;
            foreach(var run in layout.Runs)
            {
                if(run.Offset is null)throw new NtfsFormatException("sparse MFT data is unsupported");
                for(long position=0;position<run.Bytes && checked(run.Vcn*geometry.ClusterBytes+position)<layout.Bytes;)
                {
                    ct.ThrowIfCancellationRequested();var size=(int)Math.Min(4*1024*1024,run.Bytes-position);
                    var first=checked((int)((run.Vcn*geometry.ClusterBytes+position)/geometry.RecordBytes));
                    // Avoid reads for entirely unused 4 MiB ranges; unlike per-record FSCTL this streams raw extents.
                    var end=Math.Min(count,first+size/geometry.RecordBytes);
                    if(Enumerable.Range(first,end-first).Any(i=>Used(layout.Bitmap,i)))
                    {
                        var chunk=reader.ReadAt(checked(run.Offset.Value+position),size);
                        for(var number=first;number<end;number++)
                        {
                            if(!Used(layout.Bitmap,number))continue;ct.ThrowIfCancellationRequested();
                            try
                            {
                                var record=NtfsFormat.Record(chunk.AsSpan((number-first)*geometry.RecordBytes,geometry.RecordBytes),geometry);
                                if(!record.InUse){corrupt=true;continue;}
                                long logical=0,spent=0;bool hasLogical=false;
                                foreach(var attribute in record.Attributes.Where(a=>a.Type==0x80))
                                {
                                    spent=checked(spent+attribute.AllocatedBytes);
                                    if(attribute.Name.Length==0 && attribute.LowestVcn==0){if(hasLogical)throw new NtfsFormatException("duplicate unnamed DATA start");logical=attribute.LogicalBytes;hasLogical=true;}
                                }
                                infos[number]=new(record.Sequence,true,record.Directory,record.BaseReference,record.Flags,record.ModifiedFileTime,logical,spent,hasLogical);
                                // Reserved filesystem records and their extensions were consumed by bootstrap,
                                // or are outside the user namespace. Do not misclassify a split $MFT list as a user orphan.
                                var baseNumber=NtfsFormat.RecordNumber(record.BaseReference);
                                if(number<16 && number!=5 || record.BaseReference!=0 && baseNumber<16 && baseNumber!=5)continue;
                                if(record.BaseReference==0 && !record.StandardInformationValid)uncertainOwners.Add(number);
                                if(record.BaseReference==0 && !record.Directory && !record.Attributes.Any(a=>a.Type==0x20))
                                {try{ValidateDataStreams(record.Attributes,geometry);}catch(NtfsFormatException){uncertainOwners.Add(number);}}
                                foreach(var name in record.Names)
                                {
                                    // Includes compact names, tree copies, physical-identity and coverage indices.
                                    // Do not retain a full path per file merely to choose a hardlink representative.
                                    ReserveMetadata(320+name.Name.Length*2+(record.Directory?256:0));
                                    names.Add(new(number,name.ParentReference,name.Name));
                                }
                                var hasAttributeList=record.Attributes.Any(a=>a.Type==0x20);
                                if(hasAttributeList||record.BaseReference!=0)ReserveMetadata(RetainedAttributeBytes(record));
                                if(hasAttributeList)
                                {
                                    var parts=record.Attributes.Where(a=>a.Type==0x20 && a.Name.Length==0).ToArray();
                                    var start=parts.SingleOrDefault(a=>a.LowestVcn==0)??throw new NtfsFormatException("missing ATTRIBUTE_LIST start");
                                    // Reserve entry structs/list slack/UTF-16 strings and temporary content before decoding.
                                    ReserveMetadata(checked(start.LogicalBytes*8+4096));
                                    lists[number]=NtfsFormat.AttributeList(ReadContents(reader,parts,geometry,64*1024*1024));
                                    listSources[number]=record.Attributes;
                                }
                                if(record.BaseReference!=0)extensionAttributes[number]=record.Attributes;
                                if(record.BaseReference==0){if(record.Directory)visitedDirectories++;else visitedFiles++;}
                            }
                            catch(Exception e)when(e is NtfsFormatException or OverflowException or ArgumentException){if(number>=16||number==5){corrupt=true;uncertainOwners.Add(number);}}
                        }
                    }
                    position=checked(position+size);
                    if(clock.Elapsed-last>TimeSpan.FromMilliseconds(200)){last=clock.Elapsed;progress?.Report(new(visitedFiles,visitedDirectories,root));}
                }
            }
            // Extensions must be explicitly owned, match the live sequence, and appear in the base ATTRIBUTE_LIST.
            var validExtensions=new HashSet<int>();
            foreach(var pair in extensionAttributes)
            {
                var number=pair.Key;var extension=infos[number];var owner=NtfsFormat.RecordNumber(extension.Base);
                if(owner>=infos.Length || !infos[owner].InUse || infos[owner].Base!=0 || infos[owner].Sequence!=NtfsFormat.Sequence(extension.Base) || !lists.TryGetValue(owner,out var list))
                {corrupt=true;if(owner<infos.Length)uncertainOwners.Add(owner);continue;}
                if(pair.Value.Any(a=>!list.Any(l=>NtfsFormat.RecordNumber(l.Reference)==number && NtfsFormat.Sequence(l.Reference)==extension.Sequence && l.Type==a.Type && l.Instance==a.Instance && l.Name==a.Name && l.LowestVcn==a.LowestVcn)))
                {corrupt=true;uncertainOwners.Add(owner);continue;}
                validExtensions.Add(number);
                var parent=infos[owner];
                if(extension.HasLogical && parent.HasLogical) {corrupt=true;uncertainOwners.Add(owner);validExtensions.Remove(number);continue;}
                infos[owner]=parent with {Logical=extension.HasLogical?extension.Logical:parent.Logical,HasLogical=parent.HasLogical||extension.HasLogical,Allocated=checked(parent.Allocated+extension.Allocated),Flags=parent.Flags|extension.Flags};
            }
            foreach(var pair in lists)
            {
                var owner=pair.Key;if(infos[owner].Base!=0)continue;
                foreach(var item in pair.Value)
                {
                    var number=NtfsFormat.RecordNumber(item.Reference);
                    if(number>=infos.Length || !infos[number].InUse || infos[number].Sequence!=NtfsFormat.Sequence(item.Reference) || (number!=owner && (!validExtensions.Contains(number) || NtfsFormat.RecordNumber(infos[number].Base)!=owner))) {corrupt=true;uncertainOwners.Add(owner);continue;}
                    var source=number==owner?listSources[owner]:extensionAttributes[number];
                    if(!source.Any(a=>a.Type==item.Type&&a.Instance==item.Instance&&a.Name==item.Name&&a.LowestVcn==item.LowestVcn)){corrupt=true;uncertainOwners.Add(owner);}
                }
                var complete=listSources[owner].Concat(validExtensions.Where(n=>NtfsFormat.RecordNumber(infos[n].Base)==owner).SelectMany(n=>extensionAttributes[n]));
                if(!infos[owner].Directory)try{ValidateDataStreams(complete,geometry);}catch(NtfsFormatException){corrupt=true;uncertainOwners.Add(owner);}
            }
            var mergedNames=new List<Name>(names.Count);
            foreach(var name in names)
            {
                var owner=name.Owner;if(infos[owner].Base!=0){if(!validExtensions.Contains(owner))continue;owner=NtfsFormat.RecordNumber(infos[owner].Base);}
                mergedNames.Add(name with {Owner=owner});
            }
            return BuildSnapshot(root,serial,infos,mergedNames,uncertainOwners,corrupt,started,progress,ct);
        }
        catch(OverflowException e){throw new NtfsFormatException("metadata arithmetic overflow: "+e.Message);}
    }

    private static Layout ReadLayout(INtfsMetadataReader reader,NtfsGeometry geometry,CancellationToken ct)
    {
        NtfsRecord ReadRecord(long offset)=>NtfsFormat.Record(reader.ReadAt(offset,geometry.RecordBytes),geometry);
        var first=ReadRecord(geometry.MftOffset);
        if(!first.InUse || first.BaseReference!=0)throw new NtfsFormatException("invalid MFT base record");
        var records=new List<NtfsRecord>{first};var data=first.Attributes.Where(a=>a.Type==0x80 && a.Name.Length==0).ToArray();
        var preliminary=CombineRuns(data,geometry,true);var extensions=new HashSet<int>();
        if(first.Attributes.Any(a=>a.Type==0x20))
        {
            // Bootstrap is bounded independently, before the main record/index budget exists.
            var list=NtfsFormat.AttributeList(ReadContents(reader,first.Attributes.Where(a=>a.Type==0x20 && a.Name.Length==0).ToArray(),geometry,4*1024*1024));
            long retained=4L*1024*1024*8+RetainedAttributeBytes(first);
            foreach(var group in list.GroupBy(a=>NtfsFormat.RecordNumber(a.Reference)))
            {
                ct.ThrowIfCancellationRequested();if(group.Key==0)continue;
                var bytes=ReadMapped(reader,preliminary,checked((long)group.Key*geometry.RecordBytes),geometry.RecordBytes,geometry);
                var extension=NtfsFormat.Record(bytes,geometry);
                if(!extensions.Add(group.Key) || !extension.InUse || NtfsFormat.RecordNumber(extension.BaseReference)!=0 || NtfsFormat.Sequence(extension.BaseReference)!=first.Sequence || extension.Sequence!=NtfsFormat.Sequence(group.First().Reference))throw new NtfsFormatException("invalid MFT extension owner/sequence");
                foreach(var attribute in extension.Attributes)
                    if(!group.Any(l=>l.Type==attribute.Type && l.Instance==attribute.Instance && l.Name==attribute.Name && l.LowestVcn==attribute.LowestVcn))throw new NtfsFormatException("unlisted MFT extension attribute");
                retained=checked(retained+RetainedAttributeBytes(extension));
                if(retained>256L*1024*1024)throw new IOException("NTFS MFT bootstrap exceeds the supported memory budget.");
                records.Add(extension);
            }
        }
        var attrs=records.SelectMany(r=>r.Attributes).ToArray();
        var mftData=attrs.Where(a=>a.Type==0x80 && a.Name.Length==0).ToArray();var runs=CombineRuns(mftData,geometry,true);
        var start=mftData.SingleOrDefault(a=>a.LowestVcn==0)??throw new NtfsFormatException("missing MFT DATA");
        if(start.LogicalBytes<16L*geometry.RecordBytes || start.LogicalBytes>geometry.VolumeBytes || start.LogicalBytes%geometry.RecordBytes!=0 || runs.Sum(r=>r.Bytes)<start.LogicalBytes)throw new NtfsFormatException("invalid MFT length");
        if(runs.Any(r=>r.Vcn*geometry.ClusterBytes%geometry.RecordBytes!=0||r.Bytes%geometry.RecordBytes!=0))throw new NtfsFormatException("MFT extents crossing record boundaries are unsupported");
        var bitmap=ReadContents(reader,attrs.Where(a=>a.Type==0xB0 && a.Name.Length==0).ToArray(),geometry,64*1024*1024);
        return new(start.LogicalBytes,runs,bitmap);
    }
    private static IReadOnlyList<NtfsRun> CombineRuns(IReadOnlyList<NtfsAttribute> attrs,NtfsGeometry geometry,bool noSparse)
    {
        if(attrs.Count==0 || attrs.Any(a=>a.Value!=null))throw new NtfsFormatException("nonresident metadata stream required");
        var runs=attrs.OrderBy(a=>a.LowestVcn).SelectMany(a=>a.Runs).ToArray();long next=0;
        foreach(var run in runs){if(run.Vcn!=next || noSparse&&run.Offset is null)throw new NtfsFormatException("gap/overlap/sparse metadata extent");next=checked(next+run.Bytes/geometry.ClusterBytes);}
        return runs;
    }
    private static byte[] ReadContents(INtfsMetadataReader reader,IReadOnlyList<NtfsAttribute> attrs,NtfsGeometry geometry,int limit)
    {
        if(attrs.Count==1 && attrs[0].Value is {} resident){if(resident.Length>limit)throw new NtfsFormatException("metadata value too large");return resident;}
        var first=attrs.SingleOrDefault(a=>a.LowestVcn==0)??throw new NtfsFormatException("missing metadata stream start");
        if(first.LogicalBytes>limit)throw new NtfsFormatException("metadata value too large");
        var runs=CombineRuns(attrs,geometry,true);return ReadMapped(reader,runs,0,checked((int)first.LogicalBytes),geometry);
    }
    private static byte[] ReadMapped(INtfsMetadataReader reader,IReadOnlyList<NtfsRun> runs,long position,int length,NtfsGeometry geometry)
    {
        var bytes=new byte[length];var done=0;
        foreach(var run in runs)
        {
            var start=checked(run.Vcn*geometry.ClusterBytes);if(position>=start+run.Bytes)continue;
            if(position<start || run.Offset is null)throw new NtfsFormatException("missing metadata mapping");
            var offset=position-start;var size=(int)Math.Min(length-done,run.Bytes-offset);
            reader.ReadAt(checked(run.Offset.Value+offset),size).CopyTo(bytes,done);done+=size;position+=size;if(done==length)return bytes;
        }
        if(done!=length)throw new NtfsFormatException("truncated metadata mapping");return bytes;
    }
    private static void ValidateDataStreams(IEnumerable<NtfsAttribute> attributes,NtfsGeometry geometry)
    {
        var streams=attributes.Where(a=>a.Type==0x80).GroupBy(a=>a.Name).ToArray();
        if(!streams.Any(s=>s.Key.Length==0))throw new NtfsFormatException("ordinary file has no default DATA stream");
        foreach(var stream in streams)
        {
            var pieces=stream.ToArray();
            if(pieces.Any(a=>a.Value!=null)){if(pieces.Length!=1)throw new NtfsFormatException("mixed or duplicate resident DATA");continue;}
            var first=pieces.SingleOrDefault(a=>a.LowestVcn==0)??throw new NtfsFormatException("missing DATA extent start");
            if(pieces.Any(a=>a.Flags!=first.Flags))throw new NtfsFormatException("inconsistent DATA extent flags");
            var runs=CombineRuns(pieces,geometry,false);var mapped=runs.Sum(r=>r.Bytes);var physical=runs.Where(r=>r.Offset!=null).Sum(r=>r.Bytes);
            if(first.LogicalBytes>mapped || first.AllocatedBytes!=physical || (first.Flags&0x8001)==0 && runs.Any(r=>r.Offset is null))
                throw new NtfsFormatException("DATA length/allocation does not match extents");
            long end=0;
            foreach(var run in runs.Where(r=>r.Offset!=null).OrderBy(r=>r.Offset))
            {if(run.Offset!.Value<end)throw new NtfsFormatException("overlapping physical DATA extents");end=checked(run.Offset.Value+run.Bytes);}
        }
    }
    private static long RetainedAttributeBytes(NtfsRecord record)
    {
        long bytes=128;
        foreach(var a in record.Attributes)bytes=checked(bytes+192+a.Name.Length*2+(a.Value?.LongLength??0)*2+a.Runs.Count*48L);
        return bytes;
    }
    private static ScanSnapshot BuildSnapshot(string root,ulong serial,Info[] infos,IReadOnlyList<Name> names,HashSet<int> uncertainOwners,bool corrupt,DateTimeOffset started,IProgress<ScanProgress>? progress,CancellationToken ct)
    {
        if(!infos[5].InUse || !infos[5].Directory || infos[5].Base!=0)throw new NtfsFormatException("missing NTFS volume root");
        var children=new Dictionary<int,List<Name>>();var uncertainParents=new HashSet<int>();
        foreach(var name in names)
        {
            var parent=NtfsFormat.RecordNumber(name.Parent);
            if(name.Owner<16 || parent>=infos.Length || !infos[parent].InUse || !infos[parent].Directory || infos[parent].Sequence!=NtfsFormat.Sequence(name.Parent))
            {if(name.Owner>=16){corrupt=true;if(parent<infos.Length && infos[parent].InUse && infos[parent].Directory)uncertainParents.Add(parent);}continue;}
            if(!children.TryGetValue(parent,out var list))children[parent]=list=[];list.Add(name);
        }
        var builder=new ScanTreeBuilder(root);var dirs=new List<(int Index,string Path)> { (0,root) };
        var largest=new List<FileObservation>(101);var issues=new List<ScanIssue>();var physical=new Dictionary<int,(int Index,string Parent,string Name,long Bytes)>();
        var visitedDirectories=new HashSet<int>{5};var reached=new HashSet<int>();
        // $Extend and other reserved metadata containers are not directory-walker user entries.
        var deliberatelySkipped=new HashSet<int>(Enumerable.Range(0,16).Where(i=>i!=5));
        var pending=new Stack<(int Record,int Index,string Path,int Depth)>();pending.Push((5,0,root,0));long files=0;
        while(pending.TryPop(out var item))
        {
            ct.ThrowIfCancellationRequested();if(item.Depth>1024){corrupt=true;continue;}
            if(uncertainOwners.Contains(item.Record)||uncertainParents.Contains(item.Record)){builder.MarkIncomplete(item.Index);issues.Add(new(item.Path,ScanIssueKind.MetadataUnavailable));}
            if(!children.TryGetValue(item.Record,out var entries))continue;
            foreach(var group in entries.OrderBy(n=>n.Text,StringComparer.OrdinalIgnoreCase).ThenBy(n=>n.Text,StringComparer.Ordinal).GroupBy(n=>n.Text,StringComparer.OrdinalIgnoreCase))
            {
                if(group.Count()!=1){builder.MarkIncomplete(item.Index);issues.Add(new(item.Path,ScanIssueKind.MetadataUnavailable));continue;}
                var name=group.First();var info=infos[name.Owner];var path=Path.Combine(item.Path,name.Text);reached.Add(name.Owner);
                var uncertain=uncertainOwners.Contains(name.Owner);
                if(NativeFileApi.IsCloud(info.Flags)||info.Flags.HasFlag(FileAttributes.ReparsePoint))
                {
                    builder.MarkIncomplete(item.Index);issues.Add(new(path,NativeFileApi.IsCloud(info.Flags)?ScanIssueKind.CloudSkipped:ScanIssueKind.ReparseSkipped));deliberatelySkipped.Add(name.Owner);continue;
                }
                if(info.Directory)
                {
                    if(!visitedDirectories.Add(name.Owner)){corrupt=true;continue;}
                    var index=builder.AddDirectory(item.Index,name.Text,info.Flags);dirs.Add((index,path));pending.Push((name.Owner,index,path,item.Depth+1));
                }
                else
                {
                    files++;var index=builder.AddFile(item.Index,name.Text,info.Logical,uncertain?null:0,Modified(info.Modified),info.Flags);
                    if(uncertain){builder.MarkIncomplete(index);issues.Add(new(path,ScanIssueKind.MetadataUnavailable));}
                    else if(!physical.TryGetValue(name.Owner,out var existing))physical[name.Owner]=(index,item.Path,name.Text,info.Allocated);
                    else if(ComparePaths(path,Path.Combine(existing.Parent,existing.Name))<0)physical[name.Owner]=(index,item.Path,name.Text,info.Allocated);
                    var observation=new FileObservation(path,new(serial,$"{((ulong)info.Sequence<<48)|(uint)name.Owner:X16}{0UL:X16}"),info.Logical,uncertain?null:info.Allocated,Modified(info.Modified),1,info.Flags);
                    largest.Add(observation);largest.Sort((a,b)=>a.LogicalBytes!=b.LogicalBytes?b.LogicalBytes.CompareTo(a.LogicalBytes):ComparePaths(a.Path,b.Path));if(largest.Count>100)largest.RemoveAt(100);
                }
            }
            if(dirs.Count%1024==0)progress?.Report(new(files,dirs.Count,item.Path));
        }
        // Hidden MFT orphans/cycles/stale references cannot silently become a complete volume.
        var excludedQueue=new Queue<int>(deliberatelySkipped);
        while(excludedQueue.TryDequeue(out var excluded))if(children.TryGetValue(excluded,out var descendants))foreach(var d in descendants)if(deliberatelySkipped.Add(d.Owner))excludedQueue.Enqueue(d.Owner);
        for(var i=16;i<infos.Length;i++)if(infos[i].InUse && infos[i].Base==0 && !reached.Contains(i) && !deliberatelySkipped.Contains(i))corrupt=true;
        foreach(var value in physical.Values)builder.SetAllocatedBytes(value.Index,value.Bytes);
        if(corrupt)
        {
            // A torn/missing record has no trusted parent. Any child directory could contain it;
            // prevent folder-level history from calling omitted children deleted or fully measured.
            foreach(var directory in dirs)builder.MarkIncomplete(directory.Index);
            issues.Add(new(root,ScanIssueKind.MetadataUnavailable));
        }
        var tree=builder.Build();var observations=dirs.Select(d=>{var entry=tree.Entries[d.Index];return new DirectoryObservation(d.Path,entry.LogicalBytes,entry.AllocatedBytes,entry.CoverageComplete);}).ToArray();
        progress?.Report(new(files,dirs.Count,root));
        return new(Guid.NewGuid(),root,started,DateTimeOffset.UtcNow,true,observations,largest,issues){Tree=tree};
    }
    private static int ComparePaths(string a,string b){var result=StringComparer.OrdinalIgnoreCase.Compare(a,b);return result!=0?result:StringComparer.Ordinal.Compare(a,b);}
    private static DateTimeOffset Modified(long filetime){try{return new(DateTime.FromFileTimeUtc(filetime));}catch(ArgumentOutOfRangeException){return DateTimeOffset.MinValue;}}
    private static bool Used(byte[] bitmap,int number)=>(bitmap[number>>3]&(1<<(number&7)))!=0;
    private readonly record struct Info(ushort Sequence,bool InUse,bool Directory,ulong Base,FileAttributes Flags,long Modified,long Logical,long Allocated,bool HasLogical);
    private readonly record struct Name(int Owner,ulong Parent,string Text);
    private sealed record Layout(long Bytes,IReadOnlyList<NtfsRun> Runs,byte[] Bitmap);
}

internal sealed class RawNtfsReader : INtfsMetadataReader
{
    private readonly SafeFileHandle handle;
    public RawNtfsReader(string root)
    {
        // GENERIC_READ only; sharing all access allows a live volume. No write/flush/lock control exists here.
        handle=CreateFile(@"\\.\"+root[..2],0x80000000,7,IntPtr.Zero,3,0,IntPtr.Zero);
        if(handle.IsInvalid){var error=Marshal.GetLastWin32Error();handle.Dispose();throw new Win32Exception(error,"Cannot open the NTFS volume for read-only metadata scan.");}
    }
    public byte[] ReadAt(long offset,int bytes)
    {
        // Raw volume I/O must be sector aligned, including attribute-list/bitmap reads.
        const int alignment=4096;var aligned=offset/alignment*alignment;var prefix=checked((int)(offset-aligned));
        var size=checked((bytes+prefix+alignment-1)/alignment*alignment);var buffer=new byte[size];var done=0;
        while(done<size){var read=RandomAccess.Read(handle,buffer.AsSpan(done),aligned+done);if(read==0)throw new EndOfStreamException("Short raw NTFS metadata read.");done+=read;}
        return buffer.AsSpan(prefix,bytes).ToArray();
    }
    public void Dispose()=>handle.Dispose();
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)]private static extern SafeFileHandle CreateFile(string file,uint access,uint share,IntPtr security,uint creation,uint flags,IntPtr template);
}
