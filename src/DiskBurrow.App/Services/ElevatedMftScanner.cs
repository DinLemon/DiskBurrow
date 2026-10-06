using DiskBurrow.Core.Scanning;
using DiskBurrow.Windows.Files.Mft;
using Microsoft.Win32.SafeHandles;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO.Pipes;
using System.Runtime.InteropServices;
using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;

namespace DiskBurrow.App.Services;

public sealed record MftHelperArguments(string Root,string Pipe,string Nonce,int ParentId,string UserSid)
{
    public static MftHelperArguments Parse(string[] args)
    {
        if(args.Length!=6 || args[0]!="--mft-scan" || args[1].Length!=3 || !char.IsAsciiLetter(args[1][0]) || args[1][1..]!=":\\" ||
            args[2].Length!=47 || !args[2].StartsWith("DiskBurrow.Mft.",StringComparison.Ordinal) || !Guid.TryParseExact(args[2][15..],"N",out _) ||
            args[3].Length!=64 || !args[3].All(Uri.IsHexDigit) || !int.TryParse(args[4],NumberStyles.None,CultureInfo.InvariantCulture,out var pid) || pid<=0)
            throw new ArgumentException("Invalid isolated MFT helper arguments.");
        var sid=new SecurityIdentifier(args[5]);
        if(sid.Value!=args[5] || !sid.IsAccountSid())throw new ArgumentException("Invalid helper user SID.");
        return new(char.ToUpperInvariant(args[1][0])+":\\",args[2],args[3],pid,sid.Value);
    }
    public IEnumerable<string> ToCommandLine()=>["--mft-scan",Root,Pipe,Nonce,ParentId.ToString(CultureInfo.InvariantCulture),UserSid];
}
public sealed record MftHelperEvidence(int? ProcessId,bool Authenticated,int? ExitCode,bool Cancelled,TimeSpan Elapsed);

/// <summary>Interactive UAC launcher. It does not elevate the UI or acquire any cleanup capability.</summary>
public sealed class ElevatedMftScanner(Func<ProcessStartInfo,Task<Process>>? launcher=null) : IDiskScanner
{
    public MftHelperEvidence? LastRun {get;private set;}
    public async Task<ScanSnapshot> ScanAsync(string root,IProgress<ScanProgress>? progress,CancellationToken ct)
    {
        root=WindowsMftScanner.ValidateRoot(root);ct.ThrowIfCancellationRequested();
        var exe=Environment.ProcessPath??throw new IOException("The application executable is unavailable.");
        var sid=WindowsIdentity.GetCurrent().User?.Value??throw new IOException("The current Windows user SID is unavailable.");
        var args=new MftHelperArguments(root,"DiskBurrow.Mft."+Guid.NewGuid().ToString("N"),Convert.ToHexString(RandomNumberGenerator.GetBytes(32)),Environment.ProcessId,sid);
        using var pipe=MftPipeSecurity.CreateServer(args.Pipe,new SecurityIdentifier(sid));
        var info=new ProcessStartInfo(exe){UseShellExecute=true,Verb="runas",WindowStyle=ProcessWindowStyle.Hidden};
        foreach(var argument in args.ToCommandLine())info.ArgumentList.Add(argument);
        var launching=launcher is null?Task.Run(()=>Process.Start(info)??throw new IOException("Windows did not return the MFT helper process.")):launcher(info);
        Process? child=null;var authenticated=false;var completed=false;var elapsed=Stopwatch.StartNew();
        try
        {
            try{child=await launching.WaitAsync(ct);}
            catch(OperationCanceledException)
            {
                // ShellExecute can still be awaiting the secure-desktop UAC answer. Any later owned child is drained.
                _=DrainLateLaunchAsync(launching);throw;
            }
            catch(Win32Exception e)when(e.NativeErrorCode==1223){throw new OperationCanceledException("The UAC prompt was cancelled.",e,ct);}
            MftPipeSecurity.RequireProcessImage(child.Id,exe);
            using var timeout=CancellationTokenSource.CreateLinkedTokenSource(ct);timeout.CancelAfter(TimeSpan.FromMinutes(20));
            using var connect=CancellationTokenSource.CreateLinkedTokenSource(timeout.Token);connect.CancelAfter(TimeSpan.FromSeconds(45));
            var connection=pipe.WaitForConnectionAsync(connect.Token);var exited=child.WaitForExitAsync();
            if(await Task.WhenAny(connection,exited)==exited && !connection.IsCompleted)
            {
                connect.Cancel();try{await connection;}catch(OperationCanceledException){}
                throw new IOException("The isolated MFT helper exited before authentication ("+child.ExitCode+").");
            }
            await connection;
            MftPipeSecurity.RequireClient(pipe,child.Id);
            using var close=timeout.Token.Register(()=>pipe.Dispose());
            var result=await Task.Run(()=>
            {
                MftHelperProtocol.ReadHello(pipe,args.Nonce,root);pipe.WriteByte(0xA5);pipe.Flush();
                authenticated=true;var snapshot=MftHelperProtocol.ReadResult(pipe,root,progress);timeout.Token.ThrowIfCancellationRequested();return snapshot;
            },timeout.Token);
            completed=true;return result;
        }
        catch(Exception e)when(ct.IsCancellationRequested && e is IOException or ObjectDisposedException){throw new OperationCanceledException(ct);}
        finally
        {
            pipe.Dispose();
            if(child is not null)
            {
                await DrainOwnedAsync(child,completed);
                int? code=null;try{if(child.HasExited)code=child.ExitCode;}catch(InvalidOperationException){}
                LastRun=new(child.Id,authenticated,code,ct.IsCancellationRequested,elapsed.Elapsed);child.Dispose();
            }
            else LastRun=new(null,false,null,ct.IsCancellationRequested,elapsed.Elapsed);
        }
    }
    private static async Task DrainLateLaunchAsync(Task<Process> launch)
    {try{using var child=await launch;await DrainOwnedAsync(child);}catch(Exception) { /* UAC rejection: no helper was started. */ }}
    private static async Task DrainOwnedAsync(Process child,bool completed=false)
    {
        if(completed)
        {
            try{await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(3));return;}catch(InvalidOperationException){return;}catch(TimeoutException){}catch(Win32Exception){}
        }
        try{if(!child.HasExited)child.Kill(entireProcessTree:false);}catch(InvalidOperationException){}catch(Win32Exception){}
        try{await child.WaitForExitAsync().WaitAsync(TimeSpan.FromSeconds(10));}catch(InvalidOperationException){}catch(Win32Exception){}catch(TimeoutException){}
    }
}

public static class MftHelperHost
{
    public static async Task<int> RunAsync(string[] arguments)
    {
        try
        {
            var args=MftHelperArguments.Parse(arguments);
            using var identity=WindowsIdentity.GetCurrent();
            if(identity.User?.Value!=args.UserSid || !new WindowsPrincipal(identity).IsInRole(WindowsBuiltInRole.Administrator))return 3;
            var exe=Environment.ProcessPath??throw new IOException("No helper executable.");
            // Keep a live parent-process handle so PID recycling cannot substitute a new parent.
            using var parent=Process.GetProcessById(args.ParentId);_ = parent.Handle;
            MftPipeSecurity.RequireProcessImage(args.ParentId,exe);
            using var pipe=new NamedPipeClientStream(".",args.Pipe,PipeDirection.InOut,PipeOptions.Asynchronous,TokenImpersonationLevel.Identification);
            using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(45));await pipe.ConnectAsync(timeout.Token);
            MftPipeSecurity.RequireServer(pipe,args.ParentId);
            MftHelperProtocol.WriteHello(pipe,args.Nonce,args.Root);
            using var handshakeClose=timeout.Token.Register(()=>pipe.Dispose());
            var acknowledged=await Task.Run(()=>pipe.ReadByte());if(acknowledged!=0xA5)return 3;
            timeout.CancelAfter(Timeout.InfiniteTimeSpan);
            using var scanCancellation=new CancellationTokenSource(TimeSpan.FromMinutes(20));
            var finished=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            // A normal UI may lack PROCESS_TERMINATE on an elevated child: the helper cooperatively
            // observes pipe EOF/parent exit, then terminates itself if raw I/O does not drain promptly.
            var watch=WatchOwnerAsync(pipe,parent,scanCancellation,finished.Task);
            var sync=new object();
            try
            {
                var scanner=new WindowsMftScanner();
                var progress=new Relay(p=>{lock(sync)MftHelperProtocol.WriteProgress(pipe,p);});
                var result=await scanner.ScanAsync(args.Root,progress,scanCancellation.Token);
                lock(sync)MftHelperProtocol.WriteSnapshot(pipe,result);
                return 0;
            }
            catch(Exception e){lock(sync)MftHelperProtocol.WriteError(pipe,e.Message);return 4;}
            finally{finished.TrySetResult();scanCancellation.Cancel();try{await watch;}catch(OperationCanceledException){}}
        }
        catch(Exception){return 3;}
    }
    private static async Task WatchOwnerAsync(NamedPipeClientStream pipe,Process parent,CancellationTokenSource scan,Task finished)
    {
        var disconnected=ObserveDisconnectAsync(pipe,scan.Token);
        try
        {
            while(!finished.IsCompleted && !scan.IsCancellationRequested && !parent.HasExited && !disconnected.IsCompleted)
                await Task.Delay(100,scan.Token);
        }
        catch(OperationCanceledException){}
        if(finished.IsCompleted)return;
        scan.Cancel();pipe.Dispose();
        if(await Task.WhenAny(finished,Task.Delay(TimeSpan.FromSeconds(5)))!=finished)Environment.Exit(5);
    }
    private static async Task ObserveDisconnectAsync(Stream pipe,CancellationToken ct)
    {try{_ = await pipe.ReadAsync(new byte[1],ct);}catch(IOException){}catch(ObjectDisposedException){}catch(OperationCanceledException){}}
    private sealed class Relay(Action<ScanProgress> action):IProgress<ScanProgress>{public void Report(ScanProgress value)=>action(value);}
}

public static class MftPipeSecurity
{
    public static NamedPipeServerStream CreateServer(string pipeName,SecurityIdentifier sid)
    {
        var acl=new PipeSecurity();acl.SetAccessRuleProtection(true,false);acl.SetOwner(sid);
        acl.AddAccessRule(new PipeAccessRule(sid,PipeAccessRights.FullControl,AccessControlType.Allow));
        // An explicit SID ACL permits this user's elevated helper. CurrentUserOnly also compares elevation levels.
        return NamedPipeServerStreamAcl.Create(pipeName,PipeDirection.InOut,1,PipeTransmissionMode.Byte,
            PipeOptions.Asynchronous|PipeOptions.FirstPipeInstance,64*1024,64*1024,acl);
    }
    public static void RequireClient(NamedPipeServerStream pipe,int expected)
    {if(!GetNamedPipeClientProcessId(pipe.SafePipeHandle,out var actual)||actual!=(uint)expected)throw new IOException("Unexpected MFT helper client PID.");}
    public static void RequireServer(NamedPipeClientStream pipe,int expected)
    {if(!GetNamedPipeServerProcessId(pipe.SafePipeHandle,out var actual)||actual!=(uint)expected)throw new IOException("Unexpected MFT helper server PID.");}
    public static void RequireProcessImage(int processId,string expectedExe)
    {
        using var process=OpenProcess(0x1000,false,(uint)processId);var buffer=new StringBuilder(32768);var length=(uint)buffer.Capacity;
        if(process.IsInvalid||!QueryFullProcessImageName(process,0,buffer,ref length)||!StringComparer.OrdinalIgnoreCase.Equals(Path.GetFullPath(buffer.ToString()),Path.GetFullPath(expectedExe)))throw new IOException("Unexpected MFT helper executable.");
    }
    [DllImport("kernel32.dll",SetLastError=true)]private static extern bool GetNamedPipeClientProcessId(SafePipeHandle pipe,out uint pid);
    [DllImport("kernel32.dll",SetLastError=true)]private static extern bool GetNamedPipeServerProcessId(SafePipeHandle pipe,out uint pid);
    [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)]private static extern bool QueryFullProcessImageName(SafeProcessHandle process,uint flags,StringBuilder name,ref uint size);
    [DllImport("kernel32.dll",SetLastError=true)]private static extern SafeProcessHandle OpenProcess(uint access,bool inherit,uint pid);
}
