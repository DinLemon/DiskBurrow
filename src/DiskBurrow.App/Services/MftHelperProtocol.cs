using DiskBurrow.Core.Scanning;
using System.Text;

namespace DiskBurrow.App.Services;

/// <summary>Bounded binary protocol: metadata only, never a filename to execute or a deletion request.</summary>
public static class MftHelperProtocol
{
    public const int MaximumEntries=20_000_000;
    public const long MaximumBytes=1024L*1024*1024;
    public const long MaximumResidentBytes=1024L*1024*1024;
    private static readonly UTF8Encoding Utf8=new(false,true);
    public static void WriteHello(Stream stream,string nonce,string root)
    {using var w=new BinaryWriter(stream,Utf8,true);w.Write("DBMFT001"u8);WriteText(w,nonce,64);WriteText(w,root,3);w.Flush();}
    public static void ReadHello(Stream stream,string nonce,string root)
    {
        using var r=new BinaryReader(stream,Utf8,true);
        if(!r.ReadBytes(8).AsSpan().SequenceEqual("DBMFT001"u8) || ReadText(r,64)!=nonce || ReadText(r,3)!=root)throw new IOException("MFT helper authentication failed.");
    }
    public static void WriteProgress(Stream stream,ScanProgress p)
    {using var w=new BinaryWriter(stream,Utf8,true);w.Write((byte)1);w.Write(p.FilesVisited);w.Write(p.DirectoriesVisited);WriteText(w,p.CurrentPath,32767);w.Flush();}
    public static void WriteError(Stream stream,string message)
    {using var w=new BinaryWriter(stream,Utf8,true);w.Write((byte)3);WriteText(w,message,4096);w.Flush();}
    public static void WriteSnapshot(Stream stream,ScanSnapshot s)
    {
        if(s.Tree is null)throw new IOException("MFT helper produced no live index.");
        using var bounded=new BudgetStream(stream,MaximumBytes,reading:false);
        using var w=new BinaryWriter(bounded,Utf8,true);
        w.Write((byte)2);w.Write(s.Id.ToByteArray());WriteText(w,s.Root,3);w.Write(s.StartedUtc.UtcTicks);w.Write(s.CompletedUtc.UtcTicks);w.Write(s.TraversalCompleted);
        WriteCount(w,s.Directories.Count,MaximumEntries);
        foreach(var d in s.Directories){WriteText(w,d.Path,32767);w.Write(d.LogicalBytes);w.Write(d.AllocatedBytes??-1);w.Write(d.CoverageComplete);}
        WriteCount(w,s.LargestFiles.Count,100);
        foreach(var f in s.LargestFiles)
        {WriteText(w,f.Path,32767);w.Write(f.Identity is not null);if(f.Identity is {} id){w.Write(id.Volume);WriteText(w,id.FileId,128);}w.Write(f.LogicalBytes);w.Write(f.AllocatedBytes??-1);w.Write(f.ModifiedUtc.UtcTicks);w.Write(f.LinkCount);w.Write((int)f.Attributes);}
        WriteCount(w,s.Issues.Count,MaximumEntries);foreach(var issue in s.Issues){WriteText(w,issue.Path,32767);w.Write((int)issue.Kind);}
        WriteCount(w,s.Tree.Entries.Count,MaximumEntries);
        foreach(var e in s.Tree.Entries){w.Write(e.ParentIndex);WriteText(w,e.Name,255);w.Write(e.IsDirectory);w.Write(e.LogicalBytes);w.Write(e.AllocatedBytes??-1);w.Write(e.ModifiedUtc.UtcTicks);w.Write(e.FileCount);w.Write(e.CoverageComplete);w.Write((int)e.Attributes);}
        w.Flush();
    }
    public static ScanSnapshot ReadResult(Stream stream,string expectedRoot,IProgress<ScanProgress>? progress,long maximumResidentBytes=MaximumResidentBytes)
    {
        if(maximumResidentBytes<=0||maximumResidentBytes>MaximumResidentBytes)throw new ArgumentOutOfRangeException(nameof(maximumResidentBytes));
        using var bounded=new BudgetStream(stream,MaximumBytes,reading:true);
        using var r=new BinaryReader(bounded,Utf8,true);
        while(true)
        {
            var type=r.ReadByte();
            if(type==1){var f=Nonnegative(r.ReadInt64());var d=Nonnegative(r.ReadInt64());var path=ReadText(r,32767);progress?.Report(new(f,d,path));continue;}
            if(type==3)throw new IOException("Fast NTFS scan failed: "+ReadText(r,4096));
            if(type!=2)throw new IOException("Unknown MFT helper protocol frame.");
            var id=new Guid(Exact(r,16));var root=ReadText(r,3);if(root!=expectedRoot)throw new IOException("MFT helper returned another volume.");
            var started=Time(r);var completed=Time(r);var finished=r.ReadBoolean();
            long resident=0;
            void Reserve(long bytes){resident=checked(resident+bytes);if(resident>maximumResidentBytes)throw new IOException("MFT live index exceeds its supported memory budget.");}
            // Counts are not capacity hints. Read and validate each payload before growing a list.
            var count=ReadCount(r,MaximumEntries);var dirs=new List<DirectoryObservation>();
            for(var i=0;i<count;i++){var path=ReadText(r,32767);Reserve(128L+path.Length*2);dirs.Add(new(path,Nonnegative(r.ReadInt64()),Allocation(r),r.ReadBoolean()));}
            count=ReadCount(r,100);var files=new List<FileObservation>();
            for(var i=0;i<count;i++){var path=ReadText(r,32767);FileIdentity? identity=r.ReadBoolean()?new(r.ReadUInt64(),ReadText(r,128)):null;Reserve(512L+path.Length*2);files.Add(new(path,identity,Nonnegative(r.ReadInt64()),Allocation(r),Time(r),ReadCount(r,int.MaxValue),(FileAttributes)r.ReadInt32()));}
            count=ReadCount(r,MaximumEntries);var issues=new List<ScanIssue>();
            for(var i=0;i<count;i++){var path=ReadText(r,32767);var kind=(ScanIssueKind)r.ReadInt32();if(!Enum.IsDefined(kind))throw new IOException("Unknown scan issue.");Reserve(128L+path.Length*2);issues.Add(new(path,kind));}
            count=ReadCount(r,MaximumEntries);var entries=new List<ScanTreeEntry>();
            for(var i=0;i<count;i++)
            {
                var parent=r.ReadInt32();var name=ReadText(r,255);
                // Includes maximum List growth slack and the immutable ScanTree backing copy.
                Reserve(288L+name.Length*2);
                entries.Add(new(parent,name,r.ReadBoolean(),Nonnegative(r.ReadInt64()),Allocation(r),Time(r),Nonnegative(r.ReadInt64()),r.ReadBoolean(),(FileAttributes)r.ReadInt32()));
            }
            var tree=new ScanTree(root,entries);
            if(!finished || dirs.Count==0 || !StringComparer.OrdinalIgnoreCase.Equals(dirs[0].Path,root))throw new IOException("Incomplete MFT helper result.");
            return new(id,root,started,completed,true,dirs,files,issues){Tree=tree};
        }
    }
    private static DateTimeOffset Time(BinaryReader r)=>new(r.ReadInt64(),TimeSpan.Zero);
    private static long Nonnegative(long value)=>value>=0?value:throw new IOException("Negative metadata quantity.");
    private static long? Allocation(BinaryReader r){var value=r.ReadInt64();return value==-1?null:Nonnegative(value);}
    private static int ReadCount(BinaryReader r,int maximum){var count=r.ReadInt32();return count>=0&&count<=maximum?count:throw new IOException("Metadata count exceeds protocol bounds.");}
    private static void WriteCount(BinaryWriter w,int count,int maximum){if(count<0||count>maximum)throw new IOException("Metadata count exceeds protocol bounds.");w.Write(count);}
    private static void WriteText(BinaryWriter w,string text,int maximum)
    {if(text.Length>maximum)throw new IOException("Metadata text exceeds protocol bounds.");var b=Utf8.GetBytes(text);w.Write(b.Length);w.Write(b);}
    private static string ReadText(BinaryReader r,int maximum)
    {var bytes=ReadCount(r,checked(maximum*4));var text=Utf8.GetString(Exact(r,bytes));if(text.Length>maximum)throw new IOException("Metadata text exceeds protocol bounds.");return text;}
    private static byte[] Exact(BinaryReader r,int count){var bytes=r.ReadBytes(count);if(bytes.Length!=count)throw new EndOfStreamException("Truncated MFT helper frame.");return bytes;}

    private sealed class BudgetStream(Stream inner,long budget,bool reading):Stream
    {
        // Authentication uses the original pipe. Only result transfer is buffered, in one direction;
        // the helper's independent disconnect reader continues to own its original read endpoint.
        private readonly BufferedStream buffer=new(new DirectionStream(inner,reading),64*1024);
        private long spent;
        private void Charge(int count){spent=checked(spent+count);if(spent>budget)throw new IOException("MFT helper stream exceeds protocol bounds.");}
        public override int Read(byte[]b,int o,int c){var n=buffer.Read(b,o,c);Charge(n);return n;}
        public override int Read(Span<byte>b){var n=buffer.Read(b);Charge(n);return n;}
        public override void Write(byte[]b,int o,int c){Charge(c);buffer.Write(b,o,c);}
        public override void Write(ReadOnlySpan<byte>b){Charge(b.Length);buffer.Write(b);}
        public override void Flush()=>buffer.Flush();public override bool CanRead=>reading&&inner.CanRead;public override bool CanWrite=>!reading&&inner.CanWrite;public override bool CanSeek=>false;
        public override long Length=>throw new NotSupportedException();public override long Position{get=>throw new NotSupportedException();set=>throw new NotSupportedException();}
        public override long Seek(long o,SeekOrigin so)=>throw new NotSupportedException();public override void SetLength(long l)=>throw new NotSupportedException();
        protected override void Dispose(bool disposing){if(disposing)buffer.Dispose();base.Dispose(disposing);}
    }
    private sealed class DirectionStream(Stream inner,bool reading):Stream
    {
        public override int Read(byte[]b,int o,int c)=>reading?inner.Read(b,o,c):throw new NotSupportedException();
        public override int Read(Span<byte>b)=>reading?inner.Read(b):throw new NotSupportedException();
        public override void Write(byte[]b,int o,int c){if(reading)throw new NotSupportedException();inner.Write(b,o,c);}
        public override void Write(ReadOnlySpan<byte>b){if(reading)throw new NotSupportedException();inner.Write(b);}
        public override void Flush()=>inner.Flush();public override bool CanRead=>reading&&inner.CanRead;public override bool CanWrite=>!reading&&inner.CanWrite;public override bool CanSeek=>false;
        public override long Length=>throw new NotSupportedException();public override long Position{get=>throw new NotSupportedException();set=>throw new NotSupportedException();}
        public override long Seek(long o,SeekOrigin so)=>throw new NotSupportedException();public override void SetLength(long l)=>throw new NotSupportedException();
        // Deliberately leave the authenticated underlying pipe open for its owner/watchdog.
    }
}
