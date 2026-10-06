using DiskBurrow.App.Services;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
using System.Windows;
using Microsoft.Win32;
namespace DiskBurrow.App.ViewModels;
public sealed class MainViewModel : ObservableModel
{
    private readonly MonitoringCoordinator coordinator;
    private readonly IUiDispatcher dispatcher;
    private readonly LocalizationService locale;
    private CancellationTokenSource? operation;
    private Task foregroundCompletion=Task.CompletedTask;
    private bool detached;
    private bool foreground;
    private bool cancelScanRequested;
    private string statusKey="Status.Ready",filter="";
    private string? category;
    private string? primaryKey,startupKey;
    private long generation;
    private string root="";
    private readonly CancellationTokenSource displayLifetime=new();
    private readonly HashSet<Task> observedTasks=[];
    private CleanupPlan? displayedPlan;
    private string? alertKey;
    public OverviewViewModel Overview {get;}
    public HistoryViewModel History {get;}
    public CleanupViewModel Cleanup {get;}
    public SettingsViewModel Settings {get;}
    public string Root {get=>root;set{if(root==value)return;root=value;Interlocked.Increment(ref generation);History.InvalidateRequests();if(!detached)Observe(LoadSelectedRootAsync());Refresh();}}
    public string Language {get=>locale.Language;set {Settings.Language=value;locale.SetLanguage(value);Refresh();}}
    public bool Busy=>foreground||coordinator.OperationRunning;
    public bool Idle=>!Busy;
    public bool Paused {get;private set;}
    public string Status=>locale.Text(coordinator.ScanRunning?"Status.Scanning":Paused && statusKey=="Status.Ready"?"Status.Paused":statusKey);
    public string Progress {get;private set;}="";
    public string? ErrorDetails {get;private set;}
    public string? PrimaryMessage=>primaryKey is {} key?locale.Text(key):null;
    public string? StartupMessage=>startupKey is {} key?locale.Text(key):null;
    public string? StartupDetails {get;private set;}
    public string? AlertPath {get;private set;}
    public string? AlertMessage=>alertKey is {} key?locale.Text(key):null;
    public string Filter {get=>filter;set {filter=value;Observe(FilterAsync());}}
    public string? Category {get=>category;set {category=value;Observe(FilterAsync());}}
    public IReadOnlyList<CandidateRow> CleanupRows {get;private set;}=[];
    public IReadOnlyList<OutcomeRow> Outcomes {get;private set;}=[];
    public string SelectedSummary=>$"{locale.Text("Cleanup.Count")}: {Cleanup.SelectedCount}   {locale.Text("Cleanup.Selected")}: {Bytes(Cleanup.SelectedBytes)}";
    public IReadOnlyList<WarningRow> CleanupWarnings=>Cleanup.Plan?.Warnings.Take(1000).Select(w=>new WarningRow(w.Path??"",w.ReasonKey,locale)).ToArray()??[];
    public string FreeDelta=>Cleanup.Report is {} r ? r.FreeSpaceDeltaAvailable?Bytes(r.FreeSpaceDeltaBytes):locale.Text("Unknown") : "—";
    public string Coverage=>locale.Text(Overview.CoverageComplete?"Coverage.Complete":"Coverage.Incomplete");
    public bool HasNoScan=>Overview.Snapshot is null;
    public IReadOnlyList<CacheRecommendation> CacheRecommendations {get;private set;}=[];
    public string Logical=>Overview.Snapshot is null?"—":Bytes(Overview.LogicalBytes);
    public string Allocated=>Overview.Snapshot is null?"—":Overview.HasUnknownAllocatedBytes?locale.Text("Unknown"):Bytes(Overview.AllocatedBytes);
    public string Used=>Bytes(Overview.Volume?.UsedBytes);
    public string Free=>Bytes(Overview.Volume?.FreeBytes);
    public string Total=>Bytes(Overview.Volume?.TotalBytes);
    public event Action? StateChanged;
    public MainViewModel(MonitoringCoordinator coordinator,IMonitoringEnvironment environment,IUiDispatcher dispatcher,LocalizationService locale,
        OverviewViewModel overview,HistoryViewModel history,CleanupViewModel cleanup,SettingsViewModel settings,AppSettings initial)
    {
        this.coordinator=coordinator;this.dispatcher=dispatcher;this.locale=locale;Overview=overview;History=history;Cleanup=cleanup;Settings=settings;root=environment.SystemRoot;Paused=initial.Paused;
        coordinator.SnapshotChanged+=SnapshotReceived;coordinator.StatusChanged+=CoordinatorStatus;
        coordinator.ProgressChanged+=ProgressReceived;
        coordinator.VolumeObserved+=VolumeReceived;
        Overview.PropertyChanged+=(_,_)=>Refresh();History.PropertyChanged+=(_,_)=>Refresh();
        Cleanup.PropertyChanged+=(_,_)=>{if(!ReferenceEquals(displayedPlan,Cleanup.Plan)){displayedPlan=Cleanup.Plan;Outcomes=[];}CleanupRows=Cleanup.VisibleCandidates.Select(c=>new CandidateRow(c,Cleanup,locale)).ToArray();Refresh();StateChanged?.Invoke();};
        locale.Changed+=()=>{Overview.RefreshLanguage();CleanupRows=Cleanup.VisibleCandidates.Select(c=>new CandidateRow(c,Cleanup,locale)).ToArray();Outcomes=Outcomes.Select(o=>new OutcomeRow(o.Path,o.OutcomeKey,o.ReasonKey,locale)).ToArray();Refresh();StateChanged?.Invoke();};
    }
    private bool IsCurrent(long request)=>!detached&&request==Interlocked.Read(ref generation);
    private long BeginDisplay(){History.InvalidateRequests();return Interlocked.Increment(ref generation);}
    private void VolumeReceived(VolumeObservation observation)=>Observe(Overview.SetObservationAsync(observation,()=>!detached&&SameRoot(Root,observation.Root)));
    private static bool SameRoot(string a,string b)=>StringComparer.OrdinalIgnoreCase.Equals(SnapshotComparer.NormalizePath(a),SnapshotComparer.NormalizePath(b));
    public Task LoadSelectedRootAsync()
    {
        var request=BeginDisplay();var selectedRoot=Root;
        return LoadContextAsync(selectedRoot,request);
    }
    private async Task LoadContextAsync(string selectedRoot,long request,AlertEvent? alert=null)
    {
        await Overview.ClearAsync(selectedRoot,()=>IsCurrent(request));
        if(!IsCurrent(request))return;
        var volumeTask=ReadVolumeSafelyAsync(selectedRoot,request);
        var snapshot=await History.LoadRootAsync(selectedRoot,displayLifetime.Token,snapshotId:alert?.Kind==AlertKind.FolderGrowth?alert.SnapshotId:null,previousId:alert?.PreviousSnapshotId,canPublish:()=>IsCurrent(request));
        if(!IsCurrent(request)) {await volumeTask;return;}
        if(snapshot is not null)await Overview.ShowAsync(snapshot,canPublish:()=>IsCurrent(request),focusPath:alert?.DestinationPath);
        var observation=await volumeTask;
        if(observation is not null)await Overview.SetObservationAsync(observation,()=>IsCurrent(request));
        await dispatcher.InvokeAsync(()=>
        {
            if(!IsCurrent(request))return;
            CacheRecommendations=[];
            alertKey=alert?.Kind==AlertKind.FolderGrowth?(snapshot is null?"Alert.Unavailable":Overview.FocusedDirectory is null?"Alert.PathUnavailable":History.ComparisonUnavailable?"Alert.ComparisonUnavailable":"Alert.Context"):null;
            if(History.ErrorDetails is {} error){primaryKey="History.Error";ErrorDetails=error;}
            Refresh();
        });
    }
    private async Task<VolumeObservation?> ReadVolumeSafelyAsync(string selectedRoot,long request)
    {
        try{return await coordinator.ReadVolumeAsync(selectedRoot,displayLifetime.Token);}
        catch(OperationCanceledException) when(displayLifetime.IsCancellationRequested){return null;}
        catch(Exception error){await dispatcher.InvokeAsync(()=>{if(IsCurrent(request)){ErrorDetails=error.Message;Refresh();}});return null;}
    }
    public async Task ActivateAlertAsync(AlertEvent? alert=null)
    {
        if(detached)return;
        if(alert is null){var request=Interlocked.Read(ref generation);var observation=await ReadVolumeSafelyAsync(Root,request);if(observation is not null)await Overview.SetObservationAsync(observation,()=>IsCurrent(request));return;}
        var contextRoot=alert.Root??(alert.Kind==AlertKind.LowSpace?alert.DestinationPath:null);
        var requestId=BeginDisplay();AlertPath=alert.DestinationPath;
        if(contextRoot is null||alert.Kind==AlertKind.FolderGrowth&&alert.SnapshotId is null)
        {await Overview.ClearAsync(Root,()=>IsCurrent(requestId));await dispatcher.InvokeAsync(()=>{if(IsCurrent(requestId)){alertKey="Alert.Unavailable";Refresh();}});return;}
        root=contextRoot;Refresh();await LoadContextAsync(contextRoot,requestId,alert);
    }
    public string Bytes(long? bytes)=>bytes is {} n?$"{(n/1073741824d).ToString("N2",System.Globalization.CultureInfo.GetCultureInfo(locale.Language))} {(locale.Language=="ru"?"ГиБ":"GiB")}":locale.Text("Unknown");
    public void ShowStartup(string key,string details){startupKey=key;StartupDetails=details;Refresh();}
    private void CoordinatorStatus()=>Observe(dispatcher.InvokeAsync(()=>{if(coordinator.ScanRunning&&cancelScanRequested)coordinator.CancelScan();if(coordinator.LastUserMessage is {} details){primaryKey="History.Error";ErrorDetails=details;}Refresh();StateChanged?.Invoke();}));
    private void ProgressReceived(ScanProgress p)=>Observe(dispatcher.InvokeAsync(()=>{Progress=$"{p.FilesVisited:N0} / {p.DirectoriesVisited:N0}   {p.CurrentPath}";Refresh();}));
    private void SnapshotReceived(ScanSnapshot snapshot){if(SameRoot(Root,snapshot.Root))Observe(ShowSnapshotAsync(snapshot));}
    public async Task ShowSnapshotAsync(ScanSnapshot snapshot)
    {
        root=snapshot.Root;var request=BeginDisplay();
        var observation=await ReadVolumeSafelyAsync(snapshot.Root,request);
        if(!IsCurrent(request))return;
        await Overview.ShowAsync(snapshot,canPublish:()=>IsCurrent(request));
        if(observation is not null)await Overview.SetObservationAsync(observation,()=>IsCurrent(request));
        if(!IsCurrent(request))return;
        await History.LoadRootAsync(snapshot.Root,displayLifetime.Token,live:snapshot,canPublish:()=>IsCurrent(request));
        var recommendations=await Task.Run(()=>Recommendations.FindObservedCaches(snapshot.Directories,Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData)));
        await dispatcher.InvokeAsync(()=>{if(!IsCurrent(request))return;AlertPath=null;alertKey=null;CacheRecommendations=recommendations;Refresh();if(History.ErrorDetails is {} e){primaryKey="History.Error";ErrorDetails=e;Refresh();}});
    }
    public async Task ScanAsync(string? root=null)
    {
        if(Busy)return;
        var scanRoot=root??Root;
        this.root=scanRoot;BeginDisplay();Refresh();
        cancelScanRequested=false;
        try{await RunActionAsync("Status.Scanning",async _=>{var result=await Task.Run(()=>coordinator.RequestScanAsync(scanRoot,CancellationToken.None));if(!result.TraversalCompleted)await ShowSnapshotAsync(result);});}finally{cancelScanRequested=false;}
    }
    public async Task AnalyzeAsync()=>await RunActionAsync("Status.Analyzing",async ct=>await coordinator.RunExclusiveAsync(async inner=>{await Cleanup.AnalyzeAsync(inner);return true;},ct));
    public async Task DeleteAsync(bool confirmed)
    {
        if(!confirmed||!Cleanup.CanExecuteCleanup)return;
        await RunActionAsync("Status.Cleaning",async ct=>
        {
            await coordinator.RunExclusiveAsync(async inner=>{await Cleanup.ExecuteSelectedAsync(true,inner);return true;},ct);
            var report=Cleanup.Report;
            var outcomes=await Task.Run(()=>report?.Items.Take(2000).Select(i=>new OutcomeRow(i.Audit?.Path??"","Outcome."+i.Outcome,i.ReasonKey,locale)).ToArray()??[]);
            await dispatcher.InvokeAsync(()=>{Outcomes=outcomes;if(Cleanup.JournalError is {} e){primaryKey="Journal.Error";ErrorDetails=e;}if(report?.WasCancelled==true)statusKey="Status.Cancelled";Refresh();});
        });
    }
    public void Cancel(){if(coordinator.ScanRunning||foreground&&statusKey=="Status.Scanning"){cancelScanRequested=true;coordinator.CancelScan();}else operation?.Cancel();}
    public void TogglePause(){Paused=!Paused;Settings.Paused=Paused;coordinator.Pause(Paused);Refresh();StateChanged?.Invoke();}
    private async Task FilterAsync(){await Cleanup.FilterAsync(filter,category);}
    public void Observe(Task task){lock(observedTasks)observedTasks.Add(task);_ = ObserveCoreAsync(task);}
    private async Task ObserveCoreAsync(Task task){try{await task;}catch(OperationCanceledException) when(displayLifetime.IsCancellationRequested){ }catch(Exception error){await dispatcher.InvokeAsync(()=>{if(!detached)ShowError(error);});}finally{lock(observedTasks)observedTasks.Remove(task);}}
    private void ShowError(Exception error){statusKey="Status.Error";primaryKey="Status.Error";ErrorDetails=error.Message;Refresh();}
    public Task RunActionAsync(string status,Func<CancellationToken,Task> action)
    {
        if(Busy||detached)return Task.CompletedTask;
        var completion=new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        foregroundCompletion=completion.Task;
        foreground=true;var cancellation=new CancellationTokenSource();operation=cancellation;statusKey=status;primaryKey=null;ErrorDetails=null;
        return CompleteActionAsync(status,action,cancellation,completion);
    }
    private async Task CompleteActionAsync(string status,Func<CancellationToken,Task> action,CancellationTokenSource cancellation,TaskCompletionSource completion)
    {
        try {Refresh();StateChanged?.Invoke();await action(cancellation.Token);if(statusKey==status)statusKey="Status.Ready";}
        catch(OperationCanceledException){statusKey="Status.Cancelled";}
        catch(Exception error){ShowError(error);}
        finally {try{cancellation.Dispose();operation=null;foreground=false;Refresh();StateChanged?.Invoke();}finally{completion.TrySetResult();}}
    }
    public async Task HandleActionAsync(string action,object? selection=null)
    {
        switch(action)
        {
            case "Scan":await ScanAsync();break;
            case "Choose":if(!Busy){var dialog=new OpenFolderDialog();if(dialog.ShowDialog()==true){Root=dialog.FolderName;Refresh();}}break;
            case "Cancel":Cancel();break;
            case "Analyze":await AnalyzeAsync();break;
            case "Delete":if(!Busy && Cleanup.CanExecuteCleanup && MessageBox.Show(SelectedSummary+Environment.NewLine+locale.Text("Cleanup.Confirm"),"DiskBurrow",MessageBoxButton.YesNo,MessageBoxImage.Warning,MessageBoxResult.No)==MessageBoxResult.Yes)await DeleteAsync(true);break;
            case "Save":await SaveSettingsAsync();break;
            case "Export":
                if(Overview.Snapshot is {} snapshot && !Busy && MessageBox.Show(locale.Text("Export.Disclosure"),"DiskBurrow",MessageBoxButton.YesNo,MessageBoxImage.Warning,MessageBoxResult.No)==MessageBoxResult.Yes)
                {var dialog=new SaveFileDialog{Filter="JSON (*.json)|*.json",FileName="DiskBurrow-report.json"};if(dialog.ShowDialog()==true)await ExportAsync(snapshot,dialog.FileName);}break;
            case "Storage":Recommendations.OpenStorage();break;
            case "Chrome":Recommendations.OpenInstructions(Recommendations.Chrome);break;
            case "Edge":Recommendations.OpenInstructions(Recommendations.Edge);break;
            case "Repository":ProjectLinks.Open(false);break;
            case "Releases":ProjectLinks.Open(true);break;
            case "Recommendation":if(selection is CacheRecommendation recommendation)Recommendations.OpenInstructions(recommendation.InstructionsUrl);break;
            case "ExcludeFile":if(selection is CandidateRow file){Cleanup.ExcludeFile(file.Id);await FilterAsync();}break;
            case "ExcludeCategory":if(category is {} rule){Cleanup.ExcludeCategory(rule);category=null;await FilterAsync();Refresh();}break;
            case "OpenPath":if(selection is DirectoryObservation dir)Recommendations.OpenFolder(dir.Path);else if(selection is FileObservation item)Recommendations.OpenFolder(Path.GetDirectoryName(item.Path)!);break;
        }
    }
    public Task SaveSettingsAsync()=>RunActionAsync("Status.Saving",async ct=>{await Settings.SaveAsync(ct);statusKey="Status.Saved";if(Settings.AutostartDetails is {} d){primaryKey="Settings.AutostartMoved";ErrorDetails=d;}});
    public Task ExportAsync(ScanSnapshot snapshot,string path)=>RunActionAsync("Status.Exporting",async ct=>{await Task.Run(()=>new ReportExporter().ExportAsync(snapshot,path,true,ct),ct);statusKey="Status.Exported";});
    public Task StopAsync(){Detach();lock(observedTasks)return Task.WhenAll(observedTasks.Append(foregroundCompletion));}
    public void Detach(){detached=true;displayLifetime.Cancel();BeginDisplay();Cancel();coordinator.SnapshotChanged-=SnapshotReceived;coordinator.StatusChanged-=CoordinatorStatus;coordinator.ProgressChanged-=ProgressReceived;coordinator.VolumeObserved-=VolumeReceived;}
}
public sealed class CandidateRow(CleanupCandidate candidate,CleanupViewModel owner,LocalizationService locale)
{
    public Guid Id=>candidate.Id;
    public string Path=>candidate.File.Path;
    public long Bytes=>candidate.File.LogicalBytes;
    public DateTimeOffset Modified=>candidate.File.ModifiedUtc.ToLocalTime();
    public string Category=>locale.Text(candidate.Rule.RuleId);
    public string Reason=>locale.Text(candidate.ReasonKey);
    public bool Selected {get=>owner.IsSelected(Id);set=>owner.Select(Id,value);}
}
public sealed record WarningRow(string Path,string ReasonKey,LocalizationService Locale){public string Reason=>Locale.Text(ReasonKey);}
public sealed record OutcomeRow(string Path,string OutcomeKey,string? ReasonKey,LocalizationService Locale)
{
    public string Outcome=>Locale.Text(OutcomeKey);
    public string Reason=>ReasonKey is {} key?Locale.Text(key):"";
}
