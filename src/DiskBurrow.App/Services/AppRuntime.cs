using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Storage;
using DiskBurrow.Windows.Files;
using DiskBurrow.Windows.Cleanup;
using DiskBurrow.Windows.System;
using System.Windows.Threading;
using DiskBurrow.Core.Scanning;
namespace DiskBurrow.App.Services;
public sealed class AppRuntime : IAsyncDisposable
{
    private AppSettings settings;
    private readonly IMonitoringEnvironment environment;
    private Task? monitoring;
    public MonitoringCoordinator Coordinator {get;}
    public MainViewModel Model {get;}
    public LocalizationService Locale {get;}
    public SqliteHistoryStore History {get;}
    public static async Task<AppRuntime> CreateAsync(string directory,Dispatcher dispatcher,IRunRegistry? registry=null,
        IMonitoringEnvironment? environment=null,IDiskScanner? scanner=null)
    {
        var store=new SettingsStore(directory);var settings=await store.LoadAsync(default);var startup=store.LastUserMessage;
        var runtime=new AppRuntime(directory,settings,store,new SqliteHistoryStore(new StorageBudget(directory)),dispatcher,registry??new WindowsRunRegistry(),startup,environment??new WindowsEnvironment(),scanner??new WindowsDiskScanner());
        await runtime.Model.LoadSelectedRootAsync();
        return runtime;
    }
    private AppRuntime(string dataDirectory,AppSettings settings,SettingsStore store,SqliteHistoryStore history,Dispatcher dispatcher,IRunRegistry registry,string? startup,IMonitoringEnvironment environment,IDiskScanner scanner)
    {
        this.environment=environment;this.settings=settings;History=history;ThemeService.Apply(settings.Theme);Locale=new();Locale.SetLanguage(settings.Language);
        var tempRejected=false;
        if(settings.ApprovedCustomTempPath is {} temp && new RuleDiscovery().ApproveCustomTempRoot(temp) is null){this.settings=settings with {ApprovedCustomTempPath=null};settings=this.settings;tempRejected=true;}
        var ui=new WpfDispatcher(dispatcher);
        Coordinator=new(settings,scanner,history,environment,store);
        var cleanup=new CleanupViewModel(new Planner(this),new Executor(this),history,ui,()=>history.LastUserMessage){ExcludedPaths=settings.ExcludedPaths.ToHashSet(StringComparer.OrdinalIgnoreCase)};
        var settingsVm=new SettingsViewModel(settings,store,new AutostartRegistration(registry,Environment.ProcessPath!),p=>new RuleDiscovery().ApproveCustomTempRoot(p)?.Path,s=>{this.settings=s;Coordinator.ApplySettings(s);cleanup.ExcludedPaths=s.ExcludedPaths.ToHashSet(StringComparer.OrdinalIgnoreCase);Locale.SetLanguage(s.Language);});
        var manual=new ManualDeletionViewModel(new WindowsManualDeletionService(new WindowsRuleEnvironment().KnownDirectories,AppContext.BaseDirectory,dataDirectory),history,ui,()=>history.LastUserMessage);
        Model=new(Coordinator,environment,ui,Locale,new OverviewViewModel(ui),new HistoryViewModel(history,ui,()=>history.LastUserMessage),cleanup,settingsVm,settings,manual);
        if(startup is not null)Model.ShowStartup("Settings.Recovered",startup);
        if(tempRejected)Model.ShowStartup("Settings.TempRejected","Saved TEMP approval did not pass RuleDiscovery validation.");
        if(settingsVm.AutostartDetails is {} detail)Model.ShowStartup("Settings.AutostartMoved",detail);
    }
    private RuleDiscovery Discovery(){var approved=settings.ApprovedCustomTempPath is {} p?new RuleDiscovery().ApproveCustomTempRoot(p):null;return new(approvedTempRoots:approved is null?[]:[approved]);}
    public void Start(){monitoring=Task.Run(()=>Coordinator.RunAsync(CancellationToken.None));Model.Observe(monitoring);}
    public async ValueTask DisposeAsync(){await Task.WhenAll(Model.StopAsync(),Coordinator.StopAsync()).ConfigureAwait(false);if(monitoring is not null)await monitoring.ConfigureAwait(false);await Coordinator.DisposeAsync().ConfigureAwait(false);}
    private sealed class Planner(AppRuntime app) : ICleanupPlanner {public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> e,CancellationToken ct)=>new WindowsCleanupPlanner(app.Discovery()).PreviewAsync(e,ct);}
    private sealed class Executor(AppRuntime app) : ICleanupExecutor {public Task<CleanupReport> ExecuteAsync(CleanupPlan p,IReadOnlySet<Guid> s,CancellationToken ct)=>new WindowsCleanupExecutor(app.Discovery()).ExecuteAsync(p,s,ct);}
}
