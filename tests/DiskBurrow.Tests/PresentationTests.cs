using DiskBurrow.App.Services;
using DiskBurrow.App.ViewModels;
using DiskBurrow.Core.Cleanup;
using DiskBurrow.Core.Scanning;
using DiskBurrow.Core.History;
using DiskBurrow.Core.Monitoring;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;

namespace DiskBurrow.Tests;

public class PresentationTests
{
    private static readonly ScanSnapshot Snapshot = new(Guid.NewGuid(), @"C:\fixture", DateTimeOffset.UtcNow, DateTimeOffset.UtcNow, true,
        [new(@"C:\fixture", 10, null, false)], [], [new(@"C:\fixture\denied", ScanIssueKind.AccessDenied)]);
    private static CleanupPlan Plan() => new(Guid.NewGuid(), DateTimeOffset.UtcNow,
        [new(Guid.NewGuid(), new("UserTemp", @"C:\fixture", TimeSpan.FromDays(7), null),
            new(@"C:\fixture\old", null, 10, 10, DateTimeOffset.UtcNow.AddDays(-8), 1, 0), "Cleanup.TempAge")]);
    private static CleanupViewModel Vm(Executor executor, IHistoryStore? history = null) => new(new Planner(Plan()), executor, history, new InlineDispatcher());

    [Fact] public async Task NoCandidateIsInitiallySelected() { var vm = Vm(new()); await vm.AnalyzeAsync(default); Assert.Empty(vm.SelectedIds); Assert.False(vm.CanExecuteCleanup); Assert.Single(vm.VisibleCandidates); }
    [Fact] public async Task CleanupCoverageShowsAllFourRulesAndEmptyOrWarningCategories()
    {
        var plan=Plan() with {Warnings=[new("ChromeCache",@"C:\fixture\Chrome","Cleanup.OwnerRunning")]};
        var vm=new CleanupViewModel(new Planner(plan),new Executor(),null,new InlineDispatcher());await vm.AnalyzeAsync(default);
        Assert.Equal(new[]{"All","UserTemp","CrashDumps","ChromeCache","EdgeCache"},vm.Categories);
        Assert.Equal(4,vm.CategorySummaries.Count);
        Assert.Equal(1,vm.CategorySummaries.Single(s=>s.RuleId=="UserTemp").Count);
        Assert.Equal(10,vm.CategorySummaries.Single(s=>s.RuleId=="UserTemp").LogicalBytes);
        Assert.Equal("Cleanup.NoEligible",vm.CategorySummaries.Single(s=>s.RuleId=="CrashDumps").StatusKey);
        Assert.Equal("Cleanup.HasWarnings",vm.CategorySummaries.Single(s=>s.RuleId=="ChromeCache").StatusKey);
        Assert.Equal(new[]{"Cleanup.OwnerRunning"},vm.CategorySummaries.Single(s=>s.RuleId=="ChromeCache").WarningKeys);
        vm.Select(plan.Candidates[0].Id,true);await vm.FilterAsync("absent","ChromeCache");
        Assert.Equal(1,vm.CategorySummaries.Single(s=>s.RuleId=="UserTemp").Count);Assert.Single(vm.SelectedIds);
    }
    [Fact] public async Task CategorySummaryCountsEntireAnalyzedPlanBeyondVisibleLimit()
    {
        var original=Plan();var candidate=original.Candidates[0];
        var plan=original with {Candidates=Enumerable.Range(0,2005).Select(_=>candidate with {Id=Guid.NewGuid()}).ToArray()};
        var vm=new CleanupViewModel(new Planner(plan),new Executor(),null,new InlineDispatcher());await vm.AnalyzeAsync(default);
        Assert.Equal(2000,vm.VisibleCandidates.Count);Assert.Equal(2005,vm.CategorySummaries.Single(s=>s.RuleId=="UserTemp").Count);
        Assert.Equal(20050,vm.CategorySummaries.Single(s=>s.RuleId=="UserTemp").LogicalBytes);
    }
    [Fact] public async Task ExecuteRequiresSelectionAndPermanentDeletionConfirmation() { var e = new Executor(); var vm = Vm(e); await vm.AnalyzeAsync(default); await vm.ExecuteSelectedAsync(true, default); vm.Select(vm.VisibleCandidates[0].Id, true); await vm.ExecuteSelectedAsync(false, default); Assert.Equal(0, e.InvocationCount); await vm.ExecuteSelectedAsync(true, default); Assert.Equal(1, e.InvocationCount); }
    [Fact] public async Task SelectionSurvivesFiltering() { var vm = Vm(new()); await vm.AnalyzeAsync(default); var id = vm.VisibleCandidates[0].Id; vm.Select(id, true); await vm.FilterAsync("absent", null); Assert.Empty(vm.VisibleCandidates); Assert.Contains(id, vm.SelectedIds); }
    [Fact] public async Task CategoryExclusionClearsSelection() { var vm = Vm(new()); await vm.AnalyzeAsync(default); var id = vm.VisibleCandidates[0].Id; vm.Select(id, true); vm.ExcludeCategory("UserTemp"); Assert.Empty(vm.SelectedIds); Assert.False(vm.CanExecuteCleanup); }
    [Fact] public async Task JournalFailureDoesNotRepeatDeletion() { var e = new Executor(); var vm = Vm(e, new BrokenHistory()); await vm.AnalyzeAsync(default); vm.Select(vm.VisibleCandidates[0].Id, true); await vm.ExecuteSelectedAsync(true, default); Assert.NotNull(vm.Report); Assert.NotNull(vm.JournalError); Assert.Equal(vm.Plan!.Candidates[0].File.Path,Assert.Single(vm.Report!.Items).Audit?.Path); await vm.ExecuteSelectedAsync(true, default); Assert.Equal(1, e.InvocationCount); }
    [Fact] public async Task PartialCleanupRetainsOutcomesAndUnknownDelta() { var e = new Executor { Partial = true }; var vm = Vm(e); await vm.AnalyzeAsync(default); vm.Select(vm.VisibleCandidates[0].Id, true); await vm.ExecuteSelectedAsync(true, default); Assert.True(vm.Report!.WasCancelled); Assert.False(vm.Report.FreeSpaceDeltaAvailable); Assert.Single(vm.Report.Items); }
    [Fact] public async Task UiReportsCoverageAndUnknownAllocatedBytes() { var vm = new OverviewViewModel(new InlineDispatcher()); await vm.ShowAsync(Snapshot); Assert.Same(Snapshot, vm.Snapshot); Assert.True(vm.HasUnknownAllocatedBytes); Assert.False(vm.CoverageComplete); }
    [Fact] public async Task HistoryFailureLeavesCurrentOverviewVisible() { var overview = new OverviewViewModel(new InlineDispatcher()); await overview.ShowAsync(Snapshot); var history = new HistoryViewModel(new BrokenHistory(), new InlineDispatcher()); await history.LoadAsync(Snapshot, default); Assert.NotNull(history.ErrorDetails); Assert.Same(Snapshot, overview.Snapshot); }
    [Fact] public async Task ExportRequiresExplicitAction() { using var tree = new TempTree(); var path = Path.Combine(tree.Root, "unconfirmed.json"); await Assert.ThrowsAsync<InvalidOperationException>(() => new ReportExporter().ExportAsync(Snapshot, path, false, default)); Assert.False(File.Exists(path)); await new ReportExporter().ExportAsync(Snapshot, path, true, default); Assert.Contains(Snapshot.Root.Replace("\\", "\\\\"), await File.ReadAllTextAsync(path)); }
    [Fact] public void RussianAndEnglishResourceKeysMatch() { Assert.Equal(LocalizationService.ReadKeys("ru").Order(), LocalizationService.ReadKeys("en").Order()); }
    [Fact] public void AllActionsHaveLocalizedStatus() { foreach(var lang in new[]{"ru","en"}) foreach(var key in new[]{"Status.Ready","Status.Scanning","Status.Analyzing","Status.Cleaning","Status.Cancelled","Status.Exported","Status.Saved","Status.Error","History.Error","Journal.Error","Cleanup.UnsafeRoot"}) Assert.Contains(key, LocalizationService.ReadKeys(lang)); }
    [Fact] public async Task SettingsAutostartRequiresExplicitSave() { using var tree=new TempTree();var registry=new FakeRun();var vm=new SettingsViewModel(new(),new SettingsStore(tree.Root),new AutostartRegistration(registry,@"C:\fixture\DiskBurrow.exe"),p=>null,s=>{});vm.Autostart=true;Assert.Equal(0,registry.Writes);await vm.SaveAsync(default);Assert.Equal(1,registry.Writes);Assert.True((await new SettingsStore(tree.Root).LoadAsync(default)).Autostart); }
    [Fact] public async Task UnapprovedCustomTempCannotBeSaved() { using var tree=new TempTree();var registry=new FakeRun();var vm=new SettingsViewModel(new(),new SettingsStore(tree.Root),new AutostartRegistration(registry,@"C:\fixture\DiskBurrow.exe"),p=>null,s=>{});vm.CustomTemp=@"C:\unapproved";await Assert.ThrowsAsync<ArgumentException>(()=>vm.SaveAsync(default));Assert.Equal(0,registry.Writes);Assert.False(File.Exists(Path.Combine(tree.Root,"settings.json"))); }
    [Fact] public void RuntimeVerificationRequiresOwnedWorkspace() { using var tree=new TempTree();Assert.Throws<ArgumentException>(()=>RuntimeVerifier.ValidateWorkspace("relative"));Assert.Throws<ArgumentException>(()=>RuntimeVerifier.ValidateWorkspace(tree.Root));File.WriteAllText(Path.Combine(tree.Root,RuntimeVerifier.MarkerName),RuntimeVerifier.MarkerText);Assert.Equal(Path.TrimEndingDirectorySeparator(tree.Root),RuntimeVerifier.ValidateWorkspace(tree.Root)); }
    [Fact] public void RuntimeVerificationAcceptsOnlyExplicitWholeDriveProbeArguments()
    {
        Assert.Null(RuntimeVerifier.ParseArguments(["--verify-runtime",@"E:\owned\work\probe"]).MftRoot);
        Assert.Equal(@"C:\",RuntimeVerifier.ParseArguments(["--verify-runtime",@"E:\owned\work\probe","--mft-root",@"C:\"]).MftRoot);
        foreach(var args in new[]{new[]{"--verify-runtime","workspace","--mft-root",@"C:\folder"},new[]{"--verify-runtime","workspace","--mft-root",@"\\server\share"},new[]{"--verify-runtime","workspace","--run",@"C:\"},new[]{"--verify-runtime","workspace","--mft-root"},new[]{"--unknown","workspace"}})
            Assert.Throws<ArgumentException>(()=>RuntimeVerifier.ParseArguments(args));
    }
    [Fact] public async Task InvalidMftProbeRootCreatesNoRuntimeRunAndDoesNotLaunchHelper()
    {
        using var tree=new TempTree();File.WriteAllText(Path.Combine(tree.Root,RuntimeVerifier.MarkerName),RuntimeVerifier.MarkerText);
        await Assert.ThrowsAsync<ArgumentException>(()=>RuntimeVerifier.RunAsync(tree.Root,System.Windows.Threading.Dispatcher.CurrentDispatcher,@"C:\folder"));
        Assert.Empty(Directory.GetDirectories(tree.Root));
    }
    [Fact] public async Task CancelledExportLeavesNoPartialReport() { using var tree=new TempTree();var path=Path.Combine(tree.Root,"cancelled.json");using var stop=new CancellationTokenSource();stop.Cancel();await Assert.ThrowsAnyAsync<OperationCanceledException>(()=>new ReportExporter().ExportAsync(Snapshot,path,true,stop.Token));Assert.False(File.Exists(path));Assert.Empty(Directory.GetFiles(tree.Root)); }
    [Fact] public async Task ExportNeverOverwritesExistingReport() { using var tree=new TempTree();var path=tree.FileAt("existing.json",3);await Assert.ThrowsAsync<IOException>(()=>new ReportExporter().ExportAsync(Snapshot,path,true,default));Assert.Equal(3,new FileInfo(path).Length); }
    [Fact] public async Task RealStorageBudgetFailureIsVisibleAsSeparateJournalDiagnostic() { using var tree=new TempTree();var store=new SqliteHistoryStore(new StorageBudget(tree.Root,1024,0));var executor=new Executor();var vm=new CleanupViewModel(new Planner(Plan()),executor,store,new InlineDispatcher(),()=>store.LastUserMessage);await vm.AnalyzeAsync(default);vm.Select(vm.VisibleCandidates[0].Id,true);await vm.ExecuteSelectedAsync(true,default);Assert.NotNull(vm.Report);Assert.NotNull(vm.JournalError);Assert.Equal(1,executor.InvocationCount);Assert.False(vm.CanExecuteCleanup); }
    [Fact] public async Task RealHistoryReadDiagnosticIsVisibleWithoutThrowing() { using var tree=new TempTree();File.WriteAllText(Path.Combine(tree.Root,"history.db"),"not a SQLite database");var store=new SqliteHistoryStore(new StorageBudget(tree.Root));var vm=new HistoryViewModel(store,new InlineDispatcher(),()=>store.LastUserMessage);await vm.LoadAsync(Snapshot,default);Assert.NotNull(vm.ErrorDetails); }
    [Fact] public void RecommendationsRequireObservedKnownCachePath() { var dirs=new[]{new DirectoryObservation(@"C:\fixture\user\.nuget\packages",42,42,true),new DirectoryObservation(@"C:\fixture\local\pip\Cache",7,7,true),new DirectoryObservation(@"C:\fixture\user\Documents",100,100,true)};var found=Recommendations.FindObservedCaches(dirs,@"C:\fixture\user",@"C:\fixture\local");Assert.Equal(2,found.Count);Assert.DoesNotContain(found,r=>r.Path.EndsWith("Documents"));Assert.Equal(42,found.Single(r=>r.Program=="NuGet").LogicalBytes); }
    [Fact] public async Task SlowOlderProjectionCannotReplaceLatestSnapshot() { var ui=new QueuedDispatcher();var vm=new OverviewViewModel(ui);var first=vm.ShowAsync(Snapshot);var older=await ui.Queue.Reader.ReadAsync();var newest=Snapshot with {Id=Guid.NewGuid()};var second=vm.ShowAsync(newest);var latest=await ui.Queue.Reader.ReadAsync();latest.Apply();await second;older.Apply();await first;Assert.Same(newest,vm.Snapshot); }
    private sealed class QueuedDispatcher : IUiDispatcher {public System.Threading.Channels.Channel<Invocation> Queue=System.Threading.Channels.Channel.CreateUnbounded<Invocation>();public Task InvokeAsync(Action action){var invocation=new Invocation(action);Queue.Writer.TryWrite(invocation);return invocation.Done.Task;} }
    private sealed class Invocation(Action action) {public TaskCompletionSource Done=new(TaskCreationOptions.RunContinuationsAsynchronously);public void Apply(){action();Done.SetResult();} }
    [Fact] public async Task OlderHistoryLoadCannotReplaceLatestRoot()
    {
        var ui=new QueuedDispatcher();var store=new SnapshotHistory();var vm=new HistoryViewModel(store,ui);
        var first=vm.LoadAsync(store.Old,default);var older=await ui.Queue.Reader.ReadAsync();
        var second=vm.LoadAsync(store.New,default);var latest=await ui.Queue.Reader.ReadAsync();
        latest.Apply();await second;older.Apply();await first;
        Assert.Same(store.New,vm.Snapshots[0]);Assert.All(vm.Snapshots,s=>Assert.Equal(store.New.Root,s.Root));Assert.Equal(store.New.CompletedUtc,vm.ComparedUtc);
    }
    [Fact] public async Task OlderHistoryComparisonCannotReplaceLatestSelection()
    {
        var ui=new QueuedDispatcher();var store=new SnapshotHistory();var vm=new HistoryViewModel(store,ui);
        var load=vm.LoadAsync(store.New,default);(await ui.Queue.Reader.ReadAsync()).Apply();await load;
        var first=vm.CompareSelectedAsync(store.Previous);var older=await ui.Queue.Reader.ReadAsync();
        var second=vm.CompareSelectedAsync(store.New);var latest=await ui.Queue.Reader.ReadAsync();
        latest.Apply();await second;older.Apply();await first;
        Assert.Equal(store.New.CompletedUtc,vm.ComparedUtc);Assert.Equal(store.Previous.CompletedUtc,vm.PreviousUtc);Assert.Equal(90,Assert.Single(vm.Changes).LogicalDeltaBytes);
    }
    [Fact] public async Task NewHistoryLoadInvalidatesPendingComparisonBeforePublishing()
    {
        var ui=new QueuedDispatcher();var store=new SnapshotHistory();var vm=new HistoryViewModel(store,ui);
        var load=vm.LoadAsync(store.Old,default);(await ui.Queue.Reader.ReadAsync()).Apply();await load;
        var compare=vm.CompareSelectedAsync(store.Old with {CompletedUtc=store.Old.CompletedUtc.AddDays(1)});var stale=await ui.Queue.Reader.ReadAsync();
        var newer=vm.LoadAsync(store.New,default);var latest=await ui.Queue.Reader.ReadAsync();
        stale.Apply();await compare;Assert.Equal(store.Old.CompletedUtc,vm.ComparedUtc);
        latest.Apply();await newer;Assert.Equal(store.New.CompletedUtc,vm.ComparedUtc);
    }
    private sealed class SnapshotHistory : IHistoryStore
    {
        public ScanSnapshot Old=new(Guid.NewGuid(),@"C:\fixture\old",DateTimeOffset.UtcNow.AddDays(-2),DateTimeOffset.UtcNow.AddDays(-2),true,[],[],[]);
        public ScanSnapshot Previous=new(Guid.NewGuid(),@"C:\fixture\new",DateTimeOffset.UtcNow.AddDays(-1),DateTimeOffset.UtcNow.AddDays(-1),true,[new(@"C:\fixture\new",10,10,true)],[],[]);
        public ScanSnapshot New=new(Guid.NewGuid(),@"C:\fixture\new",DateTimeOffset.UtcNow,DateTimeOffset.UtcNow,true,[new(@"C:\fixture\new",100,100,true)],[],[]);
        public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>Task.FromResult<IReadOnlyList<ScanSnapshot>>(root==Old.Root?[Old]:[New,Previous]);
        public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>throw new InvalidOperationException();
        public Task AppendCleanupAsync(CleanupReport report,CancellationToken ct)=>throw new InvalidOperationException();
    }
    [Fact] public async Task OlderHistoryReadErrorCannotReplaceNewSuccessfulLoad()
    {
        var ui=new QueuedDispatcher();var store=new SnapshotHistory();var vm=new HistoryViewModel(new OldErrorHistory(store),ui);
        var first=vm.LoadAsync(store.Old,default);var older=await ui.Queue.Reader.ReadAsync();
        var second=vm.LoadAsync(store.New,default);var latest=await ui.Queue.Reader.ReadAsync();
        latest.Apply();await second;older.Apply();await first;
        Assert.Null(vm.ErrorDetails);Assert.Equal(store.New.CompletedUtc,vm.ComparedUtc);
    }
    private sealed class OldErrorHistory(SnapshotHistory inner) : IHistoryStore
    {
        public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>root==inner.Old.Root?throw new IOException("obsolete root error"):inner.LoadRecentAsync(root,limit,ct);
        public Task<HistoryWriteResult> SaveAsync(ScanSnapshot snapshot,CancellationToken ct)=>throw new InvalidOperationException();
        public Task AppendCleanupAsync(CleanupReport report,CancellationToken ct)=>throw new InvalidOperationException();
    }
    private sealed class FakeRun : IRunRegistry {public int Writes;public string? Read(string name)=>null;public void Write(string name,string value)=>Writes++;public void Delete(string name)=>Writes++;}

    private sealed class Planner(CleanupPlan plan) : ICleanupPlanner { public Task<CleanupPlan> PreviewAsync(IReadOnlySet<string> excludedPaths, CancellationToken ct) => Task.FromResult(plan); }
    private sealed class Executor : ICleanupExecutor { public int InvocationCount; public bool Partial; public Task<CleanupReport> ExecuteAsync(CleanupPlan plan, IReadOnlySet<Guid> ids, CancellationToken ct) { InvocationCount++; return Task.FromResult(new CleanupReport(plan.Id, ids.Select(id => new CleanupItemResult(id, CleanupOutcome.Deleted, null)).ToArray(), 0) { WasCancelled = Partial, FreeSpaceDeltaAvailable = !Partial }); } }
    private sealed class BrokenHistory : IHistoryStore { public Task<HistoryWriteResult> SaveAsync(ScanSnapshot s,CancellationToken ct)=>throw new IOException("history fixture"); public Task<IReadOnlyList<ScanSnapshot>> LoadRecentAsync(string root,int limit,CancellationToken ct)=>throw new IOException("history fixture"); public Task AppendCleanupAsync(CleanupReport r,CancellationToken ct)=>throw new IOException("journal fixture"); }
}
