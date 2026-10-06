using System.Diagnostics;
using System.Text.Json;
using System.Windows.Threading;
using DiskBurrow.Windows.System;
namespace DiskBurrow.App.Services;
public static class RuntimeVerifier
{
    public sealed record Arguments(string Workspace,string? MftRoot);
    public static Arguments ParseArguments(string[] args)
    {
        if(args.Length is not (2 or 4)||args[0]!="--verify-runtime"||
            args.Length==4&&(args[2]!="--mft-root"||args[3].Length!=3||!char.IsAsciiLetter(args[3][0])||args[3][1..]!=":\\"))
            throw new ArgumentException("Invalid runtime verification arguments.");
        return new(args[1],args.Length==4?args[3]:null);
    }
    public const string MarkerName=".diskburrow-runtime-workspace";
    public const string MarkerText="DiskBurrow owned runtime verification workspace";
    public static string ValidateWorkspace(string workspace)
    {
        if(!Path.IsPathFullyQualified(workspace)||!Directory.Exists(workspace))throw new ArgumentException("An existing absolute owned work directory is required.");
        var full=Path.TrimEndingDirectorySeparator(Path.GetFullPath(workspace));
        if(!full.Split(Path.DirectorySeparatorChar).Contains("work",StringComparer.OrdinalIgnoreCase))throw new ArgumentException("Verification directory must be under work/.");
        for(var path=full;path is not null;path=Path.GetDirectoryName(path))
            if((File.GetAttributes(path)&FileAttributes.ReparsePoint)!=0)throw new ArgumentException("Verification directory ancestors must not be reparse points.");
        var marker=Path.Combine(full,MarkerName);
        if(!File.Exists(marker)||(File.GetAttributes(marker)&FileAttributes.ReparsePoint)!=0||new FileInfo(marker).Length>1024||File.ReadAllText(marker).Trim()!=MarkerText)throw new ArgumentException("Owned runtime workspace marker is missing or invalid.");
        return full;
    }
    public static async Task<int> RunAsync(string workspace,Dispatcher dispatcher,string? mftRoot=null)
    {
        var parent=ValidateWorkspace(workspace);
        if(mftRoot is not null)mftRoot=DiskBurrow.Windows.Files.Mft.WindowsMftScanner.ValidateRoot(mftRoot);
        var run=Path.Combine(parent,"runtime-"+Guid.NewGuid().ToString("N"));
        if(Directory.Exists(run)||run.StartsWith(Path.TrimEndingDirectorySeparator(AppContext.BaseDirectory)+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase))throw new ArgumentException("Verification needs a fresh directory outside the EXE directory.");
        Directory.CreateDirectory(run);
        AppRuntime? runtime=null;MainWindow? window=null;TrayService? tray=null;
        try
        {
            if(!dispatcher.CheckAccess())throw new InvalidOperationException("Verification must begin on WPF Dispatcher.");
            var fixture=Path.Combine(run,"fixture");Directory.CreateDirectory(fixture);
            await File.WriteAllTextAsync(Path.Combine(fixture,"tiny-fixture.txt"),"DiskBurrow runtime fixture");
            var data=Path.Combine(run,"data");
            runtime=await AppRuntime.CreateAsync(data,dispatcher,new NoRunRegistry());
            window=new(runtime.Model);window.Show();
            tray=new(runtime.Locale,_=>window.ActivateWindow(),()=>{},()=>{},()=>{});
            var dispatcherObserved=false;
            runtime.Model.Overview.PropertyChanged+=(_,_)=>{if(dispatcher.CheckAccess())dispatcherObserved=true;};
            await runtime.Model.ScanAsync(fixture);
            var snapshot=runtime.Coordinator.LiveSnapshot??throw new InvalidOperationException("Native fixture scan did not produce a completed snapshot.");
            await runtime.Model.ShowSnapshotAsync(snapshot);
            var loaded=await runtime.History.LoadRecentAsync(fixture,30,default);
            var settingsStore=new DiskBurrow.Storage.SettingsStore(data);
            await settingsStore.SaveAsync(new(){Language="en"},default);
            var settings=await settingsStore.LoadAsync(default);
            runtime.Model.Language="en";
            var englishResource=(string)window.FindResource("Nav.Cleanup");
            runtime.Model.Language="ru";
            var russianResource=(string)window.FindResource("Nav.Cleanup");
            using var process=Process.GetCurrentProcess();
            var modules=process.Modules.Cast<ProcessModule>().Where(m=>m.ModuleName.Equals("coreclr.dll",StringComparison.OrdinalIgnoreCase)||m.ModuleName.Equals("hostpolicy.dll",StringComparison.OrdinalIgnoreCase)||m.ModuleName.Contains("sqlite",StringComparison.OrdinalIgnoreCase)).ToDictionary(m=>m.ModuleName,m=>m.FileName,StringComparer.OrdinalIgnoreCase);
            await runtime.Model.Map.WaitForSearchAsync();
            if(!dispatcherObserved||loaded.Count!=1||settings.Language!="en"||snapshot.LargestFiles.Count!=1||snapshot.Tree?.Entries.Count!=2||!runtime.Model.Map.HasTree||runtime.Model.Map.IsHistorical||englishResource==russianResource||!modules.ContainsKey("coreclr.dll")||!modules.Keys.Any(k=>k.Contains("sqlite",StringComparison.OrdinalIgnoreCase)))throw new InvalidOperationException("Runtime proof assertions failed.");
            object? mftProof=null;
            if(mftRoot is not null)
            {
                using var identity=System.Security.Principal.WindowsIdentity.GetCurrent();
                if(new System.Security.Principal.WindowsPrincipal(identity).IsInRole(System.Security.Principal.WindowsBuiltInRole.Administrator))
                    throw new InvalidOperationException("The MFT verifier parent must remain unelevated.");
                var scanner=new ElevatedMftScanner();var elapsed=Stopwatch.StartNew();runtime.Model.Root=mftRoot;
                var mft=await runtime.Coordinator.RequestScanAsync(mftRoot,scanner,CancellationToken.None);
                await runtime.Model.ShowSnapshotAsync(mft);await runtime.Model.Map.WaitForSearchAsync();
                var helper=scanner.LastRun;
                if(!mft.TraversalCompleted||mft.Root!=mftRoot||mft.Tree is null||!runtime.Model.Map.HasTree||helper?.Authenticated!=true||helper.ExitCode!=0||helper.Cancelled)
                    throw new InvalidOperationException("The real MFT helper proof is incomplete.");
                var matched=0;var sampled=0;var native=new DiskBurrow.Windows.Files.NativeFileApi();
                foreach(var observation in mft.LargestFiles.Take(10))
                {
                    try
                    {
                        var current=native.Inspect(observation.Path);sampled++;
                        if(current.Identity==observation.Identity&&current.LogicalBytes==observation.LogicalBytes&&current.ModifiedUtc==observation.ModifiedUtc)matched++;
                    }
                    catch(Exception error)when(error is IOException or UnauthorizedAccessException){ }
                }
                if(matched==0)throw new InvalidOperationException("No stable native metadata sample matched the MFT result.");
                process.Refresh();
                mftProof=new {Root=mftRoot,ParentElevated=false,Helper=helper,Entries=mft.Tree.Entries.Count,
                    FilePaths=mft.Tree.Entries[0].FileCount,Directories=mft.Directories.Count,
                    IssueCounts=mft.Issues.GroupBy(issue=>issue.Kind.ToString()).ToDictionary(group=>group.Key,group=>group.Count()),
                    BackendSeconds=(mft.CompletedUtc-mft.StartedUtc).TotalSeconds,TotalSeconds=elapsed.Elapsed.TotalSeconds,
                    GuiPeakWorkingSetBytes=process.PeakWorkingSet64,NativeSamples=sampled,NativeMatches=matched,
                    AtomicSnapshot=false,PhysicalBytesIncludeNamedStreams=true,HumanButtonClickVerified=false};
            }
            await using var proof=new FileStream(Path.Combine(run,"runtime-proof.json"),FileMode.CreateNew);
            await JsonSerializer.SerializeAsync(proof,new {Success=true,Runtime=Environment.Version.ToString(),Executable=Environment.ProcessPath,Modules=modules,DataDirectory=data,DispatcherObserved=dispatcherObserved,WindowConstructed=true,NotifyIconConstructed=true,ScanCompleted=snapshot.TraversalCompleted,FixtureFiles=snapshot.LargestFiles.Count,HistorySnapshots=loaded.Count,SettingsRoundTrip=true,ResourcesSwitched=true,LiveMapVerified=true,Mft=mftProof,MonitoringStarted=false,CleanupExecuted=false,AutostartApplied=false,VisualInteractionVerified=false},new JsonSerializerOptions{WriteIndented=true});
            return 0;
        }
        catch(Exception error)
        {
            await File.WriteAllTextAsync(Path.Combine(run,"runtime-failure.json"),JsonSerializer.Serialize(new {Success=false,Error=error.GetType().Name,Detail=error.Message,Cause=error.GetBaseException().Message}));return 2;
        }
        finally {tray?.Dispose();if(window is not null){window.AllowClose=true;window.Close();}if(runtime is not null)await runtime.DisposeAsync();}
    }
    private sealed class NoRunRegistry : IRunRegistry
    {
        public string? Read(string name)=>null;
        public void Write(string name,string value)=>throw new InvalidOperationException("Registry mutation is disabled in runtime verification.");
        public void Delete(string name)=>throw new InvalidOperationException("Registry mutation is disabled in runtime verification.");
    }
}
