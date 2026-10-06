using DiskBurrow.App.Services;
using DiskBurrow.Core.Scanning;
using System.Diagnostics;
using System.ComponentModel;
using System.IO.Pipes;
using System.Security.AccessControl;
using System.Security.Principal;
namespace DiskBurrow.Tests;
public sealed class MftHelperTests
{
    [Fact] public void Helper_arguments_allow_only_a_single_strict_readonly_volume_operation()
    {
        var sid=WindowsIdentity.GetCurrent().User!.Value;
        var valid=new MftHelperArguments(@"C:\","DiskBurrow.Mft."+Guid.NewGuid().ToString("N"),new string('A',64),Environment.ProcessId,sid);
        Assert.Equal(valid,MftHelperArguments.Parse(valid.ToCommandLine().ToArray()));
        foreach(var altered in new[]{valid with {Root=@"C:\folder"},valid with {Pipe="other"},valid with {Nonce="123"},valid with {ParentId=0},valid with {UserSid="S-1-1-0"}})
            Assert.ThrowsAny<ArgumentException>(()=>MftHelperArguments.Parse(altered.ToCommandLine().ToArray()));
        var args=valid.ToCommandLine().ToArray();args[0]="--delete";Assert.Throws<ArgumentException>(()=>MftHelperArguments.Parse(args));
    }
    [Fact] public void Protocol_preserves_live_index_and_rejects_nonce_root_and_truncated_result()
    {
        using var stream=new MemoryStream();MftHelperProtocol.WriteHello(stream,"nonce",@"C:\");stream.Position=0;
        Assert.Throws<IOException>(()=>MftHelperProtocol.ReadHello(stream,"different",@"C:\"));
        stream.SetLength(0);stream.Position=0;var s=Snapshot();MftHelperProtocol.WriteProgress(stream,new(1,2,@"C:\"));MftHelperProtocol.WriteSnapshot(stream,s);stream.Position=0;
        var read=MftHelperProtocol.ReadResult(stream,@"C:\",null);Assert.Equal(s.Id,read.Id);Assert.Equal(2,read.Tree!.Entries.Count);Assert.Equal(10,read.Tree.Entries[0].LogicalBytes);
        stream.Position=0;Assert.Throws<IOException>(()=>MftHelperProtocol.ReadResult(stream,@"D:\",null));
        stream.SetLength(stream.Length-1);stream.Position=0;Assert.Throws<EndOfStreamException>(()=>MftHelperProtocol.ReadResult(stream,@"C:\",null));
    }
    [Fact] public void Protocol_rejects_hostile_lengths_before_allocating_declared_bytes()
    {
        using var stream=new MemoryStream();using(var writer=new BinaryWriter(stream,System.Text.Encoding.UTF8,true)){writer.Write((byte)3);writer.Write(int.MaxValue);}
        stream.Position=0;Assert.Throws<IOException>(()=>MftHelperProtocol.ReadResult(stream,@"C:\",null));
    }
    [Fact] public void Large_declared_counts_with_no_payload_do_not_allocate_the_declared_arrays()
    {
        using var stream=new MemoryStream();using(var w=new BinaryWriter(stream,System.Text.Encoding.UTF8,true))
        {w.Write((byte)2);w.Write(Guid.NewGuid().ToByteArray());w.Write(3);w.Write(System.Text.Encoding.UTF8.GetBytes(@"C:\"));w.Write(DateTimeOffset.UtcNow.UtcTicks);w.Write(DateTimeOffset.UtcNow.UtcTicks);w.Write(true);w.Write(1_000_000);}
        stream.Position=0;var before=GC.GetAllocatedBytesForCurrentThread();
        Assert.Throws<EndOfStreamException>(()=>MftHelperProtocol.ReadResult(stream,@"C:\",null));
        Assert.True(GC.GetAllocatedBytesForCurrentThread()-before<256*1024,"A declared count allocated a large backing array before receiving its payload.");
    }
    [Fact] public void Aggregate_live_index_budget_is_enforced_before_immutable_backing_copy()
    {
        using var stream=new MemoryStream();MftHelperProtocol.WriteSnapshot(stream,Snapshot());stream.Position=0;
        var error=Assert.Throws<IOException>(()=>MftHelperProtocol.ReadResult(stream,@"C:\",null,512));Assert.Contains("memory budget",error.Message);
    }
    [Fact] public void Large_live_index_transfer_batches_primitive_io_without_changing_wire_bytes()
    {
        const int count=20_000;var builder=new ScanTreeBuilder(@"C:\");for(var i=0;i<count;i++)builder.AddFile(0,"file"+i,1,4096,DateTimeOffset.UtcNow);
        var tree=builder.Build();var s=new ScanSnapshot(Guid.NewGuid(),@"C:\",DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[new(@"C:\",count,count*4096L,true)],[],[]){Tree=tree};
        using var stream=new CountingStream();MftHelperProtocol.WriteSnapshot(stream,s);
        Assert.True(stream.Writes<200,$"Primitive writes reached the underlying transport {stream.Writes} times.");
        stream.Position=0;var result=MftHelperProtocol.ReadResult(stream,@"C:\",null);Assert.Equal(s.Id,result.Id);Assert.Equal(count+1,result.Tree!.Entries.Count);
        Assert.True(stream.Reads<200,$"Primitive reads reached the underlying transport {stream.Reads} times.");
        Assert.True(stream.CanRead); // Protocol wrappers leave the authenticated pipe open.
    }
    [Fact] public async Task Native_pipe_has_current_Sid_acl_and_mutual_process_authentication()
    {
        using var identity=WindowsIdentity.GetCurrent();var sid=identity.User!;
        var name="DiskBurrow.Mft."+Guid.NewGuid().ToString("N");
        using var server=MftPipeSecurity.CreateServer(name,sid);
        var rules=server.GetAccessControl().GetAccessRules(true,true,typeof(SecurityIdentifier)).Cast<PipeAccessRule>().ToArray();
        Assert.Single(rules);Assert.Equal(sid,rules[0].IdentityReference);Assert.Equal(AccessControlType.Allow,rules[0].AccessControlType);
        using var client=new NamedPipeClientStream(".",name,PipeDirection.InOut,PipeOptions.Asynchronous);
        using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(5));var connection=server.WaitForConnectionAsync(timeout.Token);await client.ConnectAsync(timeout.Token);await connection;
        MftPipeSecurity.RequireClient(server,Environment.ProcessId);MftPipeSecurity.RequireServer(client,Environment.ProcessId);
        Assert.Throws<IOException>(()=>MftPipeSecurity.RequireClient(server,int.MaxValue));Assert.Throws<IOException>(()=>MftPipeSecurity.RequireServer(client,int.MaxValue));
        MftPipeSecurity.RequireProcessImage(Environment.ProcessId,Environment.ProcessPath!);
        Assert.Throws<IOException>(()=>MftPipeSecurity.RequireProcessImage(Environment.ProcessId,Path.Combine(Path.GetTempPath(),"wrong.exe")));
        var receiving=Task.Run(()=>{MftHelperProtocol.ReadHello(server,"nonce",@"C:\");return MftHelperProtocol.ReadResult(server,@"C:\",null);},timeout.Token);
        MftHelperProtocol.WriteHello(client,"nonce",@"C:\");MftHelperProtocol.WriteSnapshot(client,Snapshot());
        Assert.Equal(10,(await receiving.WaitAsync(timeout.Token)).Tree!.Entries[0].LogicalBytes);
    }
    [Fact] public async Task Malformed_helper_mode_does_not_initialize_normal_application_services()
    {
        Assert.Equal(3,await MftHelperHost.RunAsync(["--mft-scan","not-a-drive"]));
    }
    [Fact] public async Task Uac_rejection_is_clean_cancellation_and_launch_is_same_exe_strict_arguments()
    {
        ProcessStartInfo? observed=null;
        var scanner=new ElevatedMftScanner(info=>{observed=info;return Task.FromException<Process>(new Win32Exception(1223));});
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>scanner.ScanAsync(@"C:\",null,default));
        Assert.True(observed!.UseShellExecute);Assert.Equal("runas",observed.Verb);Assert.Equal(ProcessWindowStyle.Hidden,observed.WindowStyle);
        Assert.Equal(Environment.ProcessPath,observed.FileName);var args=MftHelperArguments.Parse(observed.ArgumentList.ToArray());Assert.Equal(Environment.ProcessId,args.ParentId);Assert.Equal(@"C:\",args.Root);
    }
    [Fact] public async Task Cancelling_a_pending_launch_drains_only_the_owned_late_child()
    {
        var pending=new TaskCompletionSource<Process>(TaskCreationOptions.RunContinuationsAsynchronously);var entered=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var scanner=new ElevatedMftScanner(_=>{entered.SetResult();return pending.Task;});
        using var ct=new CancellationTokenSource();var scan=scanner.ScanAsync(@"C:\",null,ct.Token);await entered.Task;ct.Cancel();
        await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>scan);
        var shell=Environment.GetEnvironmentVariable("ComSpec")??@"C:\Windows\System32\cmd.exe";
        using var child=Process.Start(new ProcessStartInfo(shell){UseShellExecute=false,CreateNoWindow=true,ArgumentList={"/d","/c","pause"},RedirectStandardInput=true})!;
        using var observer=Process.GetProcessById(child.Id);_ = observer.Handle;
        pending.SetResult(child);await observer.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(10));Assert.True(observer.HasExited);
    }
    private static ScanSnapshot Snapshot()
    {
        var builder=new ScanTreeBuilder(@"C:\");builder.AddFile(0,"file",10,4096,DateTimeOffset.UtcNow);var tree=builder.Build();
        return new(Guid.NewGuid(),@"C:\",DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[new(@"C:\",10,4096,true)],[],[]){Tree=tree};
    }
    private sealed class CountingStream:MemoryStream
    {
        public int Writes{get;private set;}public int Reads{get;private set;}
        public override void Write(byte[]b,int o,int c){Writes++;base.Write(b,o,c);}
        public override void Write(ReadOnlySpan<byte>b){Writes++;base.Write(b);}
        public override int Read(byte[]b,int o,int c){Reads++;return base.Read(b,o,c);}
        public override int Read(Span<byte>b){Reads++;return base.Read(b);}
    }
}
