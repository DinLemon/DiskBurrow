using System.Diagnostics;
using System.Text.Json;
using System.Windows.Threading;
using DiskBurrow.Windows.System;
namespace DiskBurrow.App.Services;
public static class RuntimeVerifier
{
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
    public static async Task<int> RunAsync(string workspace,Dispatcher dispatcher)
    {
        var parent=ValidateWorkspace(workspace);
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
            if(!dispatcherObserved||loaded.Count!=1||settings.Language!="en"||snapshot.LargestFiles.Count!=1||englishResource==russianResource||!modules.ContainsKey("coreclr.dll")||!modules.Keys.Any(k=>k.Contains("sqlite",StringComparison.OrdinalIgnoreCase)))throw new InvalidOperationException("Runtime proof assertions failed.");
            await using var proof=new FileStream(Path.Combine(run,"runtime-proof.json"),FileMode.CreateNew);
            await JsonSerializer.SerializeAsync(proof,new {Success=true,Runtime=Environment.Version.ToString(),Executable=Environment.ProcessPath,Modules=modules,DataDirectory=data,DispatcherObserved=dispatcherObserved,WindowConstructed=true,NotifyIconConstructed=true,ScanCompleted=snapshot.TraversalCompleted,FixtureFiles=snapshot.LargestFiles.Count,HistorySnapshots=loaded.Count,SettingsRoundTrip=true,ResourcesSwitched=true,MonitoringStarted=false,CleanupExecuted=false,AutostartApplied=false,VisualInteractionVerified=false},new JsonSerializerOptions{WriteIndented=true});
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
