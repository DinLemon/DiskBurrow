using System.Text.Json;
using DiskBurrow.Core.Scanning;
namespace DiskBurrow.App.Services;
public sealed class ReportExporter
{
    public async Task ExportAsync(ScanSnapshot snapshot,string destination,bool pathsDisclosureAccepted,CancellationToken ct)
    {
        if(!pathsDisclosureAccepted)throw new InvalidOperationException("Explicit paths disclosure confirmation is required.");
        ct.ThrowIfCancellationRequested();
        var temporary=Path.Combine(Path.GetDirectoryName(Path.GetFullPath(destination))!,".diskburrow-export-"+Guid.NewGuid().ToString("N")+".tmp");
        var created=false;
        try
        {
            await using(var stream=new FileStream(temporary,FileMode.CreateNew,FileAccess.Write,FileShare.None,65536,true))
            {
                created=true;await JsonSerializer.SerializeAsync(stream,snapshot,new JsonSerializerOptions {WriteIndented=true},ct);await stream.FlushAsync(ct);
            }
            ct.ThrowIfCancellationRequested();File.Move(temporary,destination,overwrite:false);created=false;
        }
        finally {if(created)File.Delete(temporary);}
    }
}
