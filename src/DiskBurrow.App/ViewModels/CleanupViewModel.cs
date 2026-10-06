using DiskBurrow.App.Services;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.History;
namespace DiskBurrow.App.ViewModels;
public sealed class CleanupViewModel(ICleanupPlanner planner,ICleanupExecutor executor,IHistoryStore? history,IUiDispatcher dispatcher,Func<string?>? historyDiagnostic=null) : ObservableModel
{
    private readonly HashSet<Guid> selected=[];
    private readonly HashSet<Guid> excluded=[];
    private readonly HashSet<string> excludedCategories=new(StringComparer.Ordinal);
    private readonly SemaphoreSlim gate=new(1,1);
    private IReadOnlyDictionary<Guid,CleanupCandidate> byId=new Dictionary<Guid,CleanupCandidate>();
    private long filterVersion;
    public CleanupPlan? Plan {get;private set;}
    public IReadOnlySet<Guid> SelectedIds=>selected.ToHashSet();
    public IReadOnlyList<CleanupCandidate> VisibleCandidates {get;private set;}=[];
    public IReadOnlyList<string> Categories {get;private set;}=[];
    public CleanupReport? Report {get;private set;}
    public string? JournalError {get;private set;}
    public bool Busy {get;private set;}
    public bool CanExecuteCleanup=>!Busy && Report is null && selected.Count>0;
    public long SelectedBytes=>selected.Sum(id=>byId[id].File.LogicalBytes);
    public int SelectedCount=>selected.Count;
    public bool IsSelected(Guid id)=>selected.Contains(id);
    public int UnattemptedCount {get;private set;}
    public IReadOnlySet<string> ExcludedPaths {get;set;}=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
    public async Task AnalyzeAsync(CancellationToken ct)
    {
        if(!await gate.WaitAsync(0,ct)) return;
        try {
            await dispatcher.InvokeAsync(()=>{Busy=true;Refresh();});
            var plan=await planner.PreviewAsync(ExcludedPaths,ct);
            var prepared=await Task.Run(()=>(Rows:plan.Candidates.OrderByDescending(c=>c.File.LogicalBytes).Take(2000).ToArray(),Index:plan.Candidates.ToDictionary(c=>c.Id),Categories:plan.Candidates.Select(c=>c.Rule.RuleId).Distinct().Prepend("All").ToArray()),ct);
            await dispatcher.InvokeAsync(()=>{Plan=plan;byId=prepared.Index;Categories=prepared.Categories;selected.Clear();excluded.Clear();excludedCategories.Clear();Report=null;JournalError=null;UnattemptedCount=0;VisibleCandidates=prepared.Rows;Refresh();});
        } finally { await dispatcher.InvokeAsync(()=>{Busy=false;Refresh();}); gate.Release(); }
    }
    public void Select(Guid id,bool value)
    {
        if(Busy || Report is not null) return;
        if(value && byId.TryGetValue(id,out var candidate) && !excluded.Contains(id) && !excludedCategories.Contains(candidate.Rule.RuleId)) selected.Add(id); else selected.Remove(id);
        Refresh();
    }
    public void ExcludeFile(Guid id) { if(Busy)return; excluded.Add(id);selected.Remove(id);Refresh(); }
    public void ExcludeCategory(string rule) { if(Busy||rule=="All")return; excludedCategories.Add(rule); foreach(var id in selected.Where(id=>byId[id].Rule.RuleId==rule).ToArray()) selected.Remove(id); Refresh(); }
    public async Task FilterAsync(string filter,string? category)
    {
        var version=Interlocked.Increment(ref filterVersion);
        var plan=Plan;var excludedCopy=excluded.ToHashSet();var categories=excludedCategories.ToHashSet();
        var rows=await Task.Run(()=>plan?.Candidates.Where(c=>!excludedCopy.Contains(c.Id) && !categories.Contains(c.Rule.RuleId) && (category is null or "All" || c.Rule.RuleId==category) && c.File.Path.Contains(filter,StringComparison.OrdinalIgnoreCase)).OrderByDescending(c=>c.File.LogicalBytes).Take(2000).ToArray() ?? []);
        await dispatcher.InvokeAsync(()=>{if(version==Interlocked.Read(ref filterVersion)&&ReferenceEquals(plan,Plan)){VisibleCandidates=rows;Refresh();}});
    }
    public async Task ExecuteSelectedAsync(bool permanentDeletionConfirmed,CancellationToken ct)
    {
        if(!permanentDeletionConfirmed || !CanExecuteCleanup || Plan is null || !await gate.WaitAsync(0,ct))return;
        try {
            var plan=Plan;var ids=selected.ToHashSet();
            await dispatcher.InvokeAsync(()=>{Busy=true;Refresh();});
            var report=(await executor.ExecuteAsync(plan,ids,ct)).WithAudit(plan);
            // Publish before journaling; persistence failure cannot permit the same deletion again.
            await dispatcher.InvokeAsync(()=>{Report=report;UnattemptedCount=ids.Count-report.Items.Count;selected.Clear();Refresh();});
            if(history is not null) try { var diagnostic=await Task.Run(async()=>{await history.AppendCleanupAsync(report,CancellationToken.None);return historyDiagnostic?.Invoke();});if(diagnostic is not null)await dispatcher.InvokeAsync(()=>{JournalError=diagnostic;Refresh();}); } catch(Exception error) { await dispatcher.InvokeAsync(()=>{JournalError=error.Message;Refresh();}); }
        }finally {await dispatcher.InvokeAsync(()=>{Busy=false;Refresh();});gate.Release();}
    }
}
