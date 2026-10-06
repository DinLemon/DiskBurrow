using DiskBurrow.Core.Monitoring;
using DiskBurrow.Storage;
using DiskBurrow.Windows.System;
namespace DiskBurrow.App.ViewModels;
public sealed class SettingsViewModel : ObservableModel
{
    private const long GiB=1024L*1024*1024;
    private readonly SettingsStore store;
    private readonly AutostartRegistration autostart;
    private readonly Func<string,string?> approve;
    private readonly Action<AppSettings> apply;
    private AppSettings current;
    public int[] Intervals {get;}=[1,6,12,24];
    public string[] Languages {get;}=["ru","en"];
    public int IntervalHours {get;set;}
    public double LowGiB {get;set;}
    public double GrowthGiB {get;set;}
    public bool AllowOnBattery {get;set;}
    public bool Autostart {get;set;}
    public bool Paused {get;set;}
    public string Language {get;set;}
    public string Exclusions {get;set;}
    public string CustomTemp {get;set;}
    public string? AutostartDetails {get;private set;}
    public SettingsViewModel(AppSettings settings,SettingsStore store,AutostartRegistration autostart,Func<string,string?> approve,Action<AppSettings> apply)
    {
        current=settings;this.store=store;this.autostart=autostart;this.approve=approve;this.apply=apply;
        IntervalHours=settings.IntervalHours;LowGiB=(double)settings.LowSpaceBytes/GiB;GrowthGiB=(double)settings.GrowthBytes/GiB;AllowOnBattery=settings.AllowOnBattery;Autostart=settings.Autostart;Paused=settings.Paused;Language=settings.Language;Exclusions=string.Join(Environment.NewLine,settings.ExcludedPaths);CustomTemp=settings.ApprovedCustomTempPath??"";
        try {if(autostart.IsPathChanged)AutostartDetails="Settings.AutostartMoved";}catch(Exception error){AutostartDetails=error.Message;}
    }
    public async Task SaveAsync(CancellationToken ct)
    {
        if(!double.IsFinite(LowGiB)||!double.IsFinite(GrowthGiB)||LowGiB<0||GrowthGiB<=0||LowGiB>long.MaxValue/(double)GiB||GrowthGiB>long.MaxValue/(double)GiB)throw new ArgumentException("Thresholds must be finite and in range.");
        string? approved=null;
        if(!string.IsNullOrWhiteSpace(CustomTemp))approved=approve(CustomTemp.Trim())??throw new ArgumentException("Custom TEMP failed safety validation.");
        var next=current with {IntervalHours=IntervalHours,LowSpaceBytes=checked((long)(LowGiB*GiB)),GrowthBytes=checked((long)(GrowthGiB*GiB)),AllowOnBattery=AllowOnBattery,Autostart=Autostart,Paused=Paused,Language=Language,ExcludedPaths=Exclusions.Split(['\r','\n'],StringSplitOptions.RemoveEmptyEntries|StringSplitOptions.TrimEntries),ApprovedCustomTempPath=approved};
        next.Validate();await store.SaveAsync(next,ct);
        // Only explicit Save can mutate the user's Run registration.
        autostart.Apply(next.Autostart);AutostartDetails=autostart.LastUserMessage;
        current=next;apply(next);Refresh();
    }
}
